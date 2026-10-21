//! Projects as the web UI shows them (ADR 0023): the server's, with this
//! machine's clones, or the clones alone without a server.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;

use silt_proto::{ProjectInfo, project_name};
use tokio::sync::oneshot;
use tracing::warn;

use crate::core::{Core, CoreError, Event};
use crate::protocol::{Project, Request, Response};

impl Core {
    /// The projects to show: the server's when it answers, each with its
    /// clone here, then the clones the server doesn't know.
    pub(crate) fn projects(&self) -> impl Future<Output = Result<Vec<Project>, CoreError>> + Send + 'static {
        let local = self.local_projects();
        let remote = self.remote.clone();
        async move {
            let Some(remote) = remote else {
                return Ok(local);
            };
            match remote.projects().await {
                Ok(reply) => Ok(merged(reply.projects, local)),
                Err(error) => {
                    warn!(%error, "could not list the server's projects; showing this machine's");
                    Ok(local)
                }
            }
        }
    }

    /// A project for each clone here, counting its threads and open merge
    /// requests on this machine.
    fn local_projects(&self) -> Vec<Project> {
        let mut threads: BTreeMap<&str, u32> = BTreeMap::new();
        for thread in self.threads.values().filter(|thread| !self.deleted.contains(&thread.id)) {
            *threads.entry(thread.repo.as_str()).or_default() += 1;
        }
        let mut open: BTreeMap<String, u32> = BTreeMap::new();
        for mr in self.mr_list(None).unwrap_or_default().into_iter().filter(|mr| mr.status == "open") {
            if let Some(target) = self.find_thread(&mr.target).ok().and_then(|thread| self.threads.get(&thread)) {
                *open.entry(target.repo.clone()).or_default() += 1;
            }
        }
        let mut projects: Vec<Project> = self
            .clones
            .iter()
            .map(|(repo, path)| Project {
                name: project_name(repo),
                threads: threads.get(repo.as_str()).copied().unwrap_or_default(),
                open_mrs: open.get(repo).copied().unwrap_or_default(),
                clone: Some(path.clone()),
                ..project(repo)
            })
            .collect();
        for project in &mut projects {
            project.origin_url = project.clone.as_deref().and_then(origin_url);
        }
        projects.sort_by(|a, b| a.name.cmp(&b.name));
        projects
    }

    /// Adds the clone at `path`, which needs an origin to name its project.
    pub(crate) async fn add_clone(&mut self, path: &Path) -> Result<String, CoreError> {
        let repo = silt_git::Repo::discover(path)?;
        let origin = repo.remote_url("origin")?.ok_or_else(|| {
            CoreError::Invalid(format!(
                "{} has no origin remote; projects without one come with local-only workspaces",
                repo.root().display()
            ))
        })?;
        self.remember_clone(repo.root()).await;
        Ok(silt_git::normalize_remote(&origin))
    }

    /// Clones `url` to `path` off the event loop, then adds the clone
    /// through it.
    pub(crate) fn clone_project(
        &self,
        url: &str,
        path: &Path,
    ) -> Result<impl Future<Output = Result<String, CoreError>> + Send + 'static, CoreError> {
        check_clone_target(url, path)?;
        let (url, path, events) = (url.to_owned(), path.to_owned(), self.events.clone());
        Ok(async move {
            let output = tokio::process::Command::new("git").args(["clone", "--quiet", "--", &url]).arg(&path).output().await?;
            if !output.status.success() {
                return Err(CoreError::Invalid(format!(
                    "git could not clone {url}: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }
            let (reply, answer) = oneshot::channel();
            events
                .send(Event::Request(Request::AddClone { path }, reply))
                .map_err(|_| CoreError::Invalid("the daemon is stopping".into()))?;
            match answer.await {
                Ok(Response::Ok { data }) => Ok(data.as_str().unwrap_or_default().to_owned()),
                Ok(Response::Error { message }) => Err(CoreError::Invalid(message)),
                Err(_) => Err(CoreError::Invalid("the daemon stopped".into())),
            }
        })
    }
}

impl Core {
    /// The person's orgs, as the server has them.
    pub(crate) fn orgs(
        &self,
    ) -> Result<impl Future<Output = Result<Vec<silt_proto::OrgInfo>, CoreError>> + Send + 'static, CoreError> {
        let remote = self.signed_in()?;
        Ok(async move {
            let reply = remote.orgs().await.map_err(|source| CoreError::Server { doing: "list your orgs", source })?;
            Ok(reply.orgs)
        })
    }

    /// Passes an org change to the server, which checks who may make it.
    pub(crate) fn org_change(
        &self,
        change: silt_proto::OrgChange,
    ) -> Result<impl Future<Output = Result<&'static str, CoreError>> + Send + 'static, CoreError> {
        let remote = self.signed_in()?;
        Ok(async move {
            remote.org_change(&change).await.map_err(|source| CoreError::Server { doing: "change the org", source })?;
            Ok("done")
        })
    }

    pub(crate) fn devices(
        &self,
    ) -> Result<impl Future<Output = Result<Vec<crate::protocol::DeviceView>, CoreError>> + Send + 'static, CoreError> {
        let remote = self.signed_in()?;
        Ok(async move { remote.devices().await.map_err(|source| CoreError::Server { doing: "list your devices", source }) })
    }

    pub(crate) fn device_change(
        &self,
        id: String,
        change: crate::protocol::DeviceChange,
    ) -> Result<impl Future<Output = Result<crate::protocol::DeviceView, CoreError>> + Send + 'static, CoreError> {
        let remote = self.signed_in()?;
        Ok(async move {
            remote.change_device(&id, &change).await.map_err(|source| CoreError::Server { doing: "change the device", source })
        })
    }

    pub(crate) fn device_revoke(
        &self,
        id: String,
    ) -> Result<impl Future<Output = Result<&'static str, CoreError>> + Send + 'static, CoreError> {
        let remote = self.signed_in()?;
        Ok(async move {
            remote.revoke_device(&id).await.map_err(|source| CoreError::Server { doing: "sign the device out", source })?;
            Ok("signed out")
        })
    }

    fn signed_in(&self) -> Result<crate::remote::Remote, CoreError> {
        self.remote.clone().ok_or_else(|| CoreError::Invalid("not connected to a server; use `silt auth login`".into()))
    }
}

fn project(repo: &str) -> Project {
    Project {
        repo: repo.to_owned(),
        name: project_name(repo),
        org: None,
        origin_url: None,
        visibility: None,
        default_agent: None,
        default_preset: None,
        threads: 0,
        open_mrs: 0,
        clone: None,
    }
}

fn origin_url(clone: &Path) -> Option<String> {
    let url = silt_git::Repo::discover(clone).ok()?.remote_url("origin").ok()??;
    Some(silt_proto::shareable_url(&url))
}

/// Where a clone may go: an absolute path whose parent exists, not over
/// something already there; and a URL git won't read as an option.
fn check_clone_target(url: &str, path: &Path) -> Result<(), CoreError> {
    if url.trim().is_empty() || url.starts_with('-') {
        return Err(CoreError::Invalid("give the URL to clone".into()));
    }
    if !path.is_absolute() {
        return Err(CoreError::Invalid("give the clone an absolute path".into()));
    }
    if !path.parent().is_some_and(Path::is_dir) {
        return Err(CoreError::Invalid(format!("{} isn't a folder", path.parent().unwrap_or(path).display())));
    }
    let taken = std::fs::read_dir(path).map_or(path.exists(), |mut entries| entries.next().is_some());
    if taken {
        return Err(CoreError::Invalid(format!("{} is already there", path.display())));
    }
    Ok(())
}

/// The server's projects with what this machine knows of each, then the
/// clones of projects the server doesn't list.
fn merged(server: Vec<ProjectInfo>, local: Vec<Project>) -> Vec<Project> {
    let mut here: BTreeMap<String, Project> = local.into_iter().map(|project| (project.repo.clone(), project)).collect();
    let mut projects: Vec<Project> = server
        .into_iter()
        .map(|info| {
            let mine = here.remove(&info.repo);
            Project {
                origin_url: info.origin_url.or_else(|| mine.as_ref().and_then(|mine| mine.origin_url.clone())),
                open_mrs: mine.as_ref().map_or(0, |mine| mine.open_mrs),
                clone: mine.and_then(|mine| mine.clone),
                repo: info.repo,
                name: info.name,
                org: info.org,
                visibility: info.visibility,
                default_agent: info.default_agent,
                default_preset: info.default_preset,
                threads: info.threads,
            }
        })
        .collect();
    projects.extend(here.into_values());
    projects.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.repo.cmp(&b.repo)));
    projects
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn info(repo: &str, threads: u32) -> ProjectInfo {
        ProjectInfo {
            repo: repo.into(),
            org: Some("acme".into()),
            name: project_name(repo),
            origin_url: None,
            visibility: None,
            default_agent: Some("claude-acp".into()),
            default_preset: None,
            threads,
        }
    }

    #[test]
    fn the_servers_projects_gain_this_machines_clones_and_unknown_clones_follow() {
        let here = |repo: &str, path: &str| Project {
            open_mrs: 2,
            clone: Some(PathBuf::from(path)),
            origin_url: Some(format!("https://{repo}.git")),
            threads: 9,
            ..project(repo)
        };
        let projects = merged(
            vec![info("github.com/acme/web", 3), info("github.com/acme/api", 1)],
            vec![here("github.com/acme/api", "/src/api"), here("example.com/mine", "/src/mine")],
        );
        let names: Vec<&str> = projects.iter().map(|project| project.name.as_str()).collect();
        assert_eq!(names, ["api", "mine", "web"]);
        let api = &projects[0];
        assert_eq!((api.threads, api.open_mrs, api.clone.as_deref()), (1, 2, Some(Path::new("/src/api"))));
        assert_eq!(api.default_agent.as_deref(), Some("claude-acp"));
        assert_eq!(api.origin_url.as_deref(), Some("https://github.com/acme/api.git"));
        assert_eq!((projects[1].org.as_deref(), projects[1].threads), (None, 9));
        assert_eq!(projects[2].clone, None);
    }

    #[test]
    fn a_clone_goes_only_where_nothing_is() {
        let dir = tempfile::tempdir().expect("temp dir");
        let fresh = dir.path().join("fresh");
        assert!(check_clone_target("https://example.com/x.git", &fresh).is_ok());
        assert!(check_clone_target("--upload-pack=evil", &fresh).is_err());
        assert!(check_clone_target("https://example.com/x.git", Path::new("relative/x")).is_err());
        assert!(check_clone_target("https://example.com/x.git", &dir.path().join("no/such/parent")).is_err());
        std::fs::write(dir.path().join("taken"), "x").expect("write");
        assert!(check_clone_target("https://example.com/x.git", &dir.path().join("taken")).is_err());
        std::fs::create_dir(dir.path().join("empty")).expect("mkdir");
        assert!(check_clone_target("https://example.com/x.git", &dir.path().join("empty")).is_ok());
    }
}
