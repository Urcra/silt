//! Projects as the web UI shows them (ADR 0023): the server's, with this
//! machine's clones, or the clones alone without a server.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;

use silt_proto::{ProjectInfo, project_name};
use silt_types::ThreadId;
use tokio::sync::oneshot;
use tracing::warn;

use crate::core::{Core, CoreError, Event};
use crate::protocol::{Project, Request, Response};

impl Core {
    /// The projects to show: each server's that answers, with its clone
    /// here, then the clones no server knows.
    pub(crate) fn projects(&self) -> impl Future<Output = Result<Vec<Project>, CoreError>> + Send + 'static {
        let local = self.local_projects();
        let remotes = self.asked_remotes().unwrap_or_default();
        let homes: BTreeMap<String, String> =
            self.project_homes.iter().map(|(repo, home)| (repo.clone(), home.shown().to_owned())).collect();
        async move {
            let mut listed = Vec::new();
            for remote in remotes {
                match remote.projects().await {
                    Ok(reply) => listed.push((remote.config.url.clone(), reply.projects)),
                    Err(error) => warn!(%error, server = remote.config.url, "could not list a server's projects"),
                }
            }
            let mut projects = merged(listed, local);
            projects.retain(|project| !silt_types::is_chat(&project.repo));
            for project in &mut projects {
                project.home = homes
                    .get(&project.repo)
                    .cloned()
                    .or_else(|| (project.clone.is_some() && project.origin_url.is_none()).then(|| "local".to_owned()));
            }
            Ok(projects)
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

    /// Adds the clone at `path`, naming its project.
    pub(crate) async fn add_clone(&mut self, path: &Path) -> Result<String, CoreError> {
        let repo = silt_git::Repo::discover(path).map_err(|_| {
            CoreError::Invalid(format!("{} isn't in a git repository; create it as a new project instead", path.display()))
        })?;
        self.remember_clone(repo.root()).await;
        Ok(repo.project()?)
    }

    /// Moves the threads recorded in the clone at `root` to `project`, the
    /// project it names now, from one it named by its folder or not at all:
    /// a repository without a remote that gained one, or threads recorded
    /// before such a repository was a project (ADR 0053). Threads that sync
    /// with a server stay where they are, as others hold them there.
    pub(crate) async fn follow_clone_to(&mut self, root: &Path, project: &str) {
        let mut moving: BTreeMap<String, Vec<ThreadId>> = BTreeMap::new();
        for thread in self.threads.values().filter(|thread| {
            thread.repo_root == root
                && thread.repo != project
                && (thread.repo.is_empty() || silt_git::is_folder_project(&thread.repo))
                && self.remote_of(thread.family).is_none()
        }) {
            moving.entry(thread.repo.clone()).or_default().push(thread.id);
        }
        for (from, threads) in moving {
            if let Err(error) = self.store.move_threads(&threads, &from, project, root).await {
                warn!(%error, from, to = project, "could not move threads to their clone's project");
                continue;
            }
            for thread in &threads {
                self.threads.get_mut(thread).expect("listed above").repo = project.to_owned();
            }
            if self.clones.get(&from).is_some_and(|path| path == root) {
                self.clones.remove(&from);
            }
            if let Some(home) = self.project_homes.get(&from).cloned() {
                self.project_homes.entry(project.to_owned()).or_insert(home);
            }
            self.announce(crate::protocol::BusEvent::Threads);
        }
    }

    /// [`Self::follow_clone_to`] for every clone threads were recorded in,
    /// as the daemon starts.
    pub(crate) async fn follow_clones_to_their_projects(&mut self) {
        let roots: std::collections::BTreeSet<std::path::PathBuf> = self
            .threads
            .values()
            .filter(|thread| thread.repo.is_empty() || silt_git::is_folder_project(&thread.repo))
            .filter(|thread| thread.repo_root.join(".git").exists())
            .map(|thread| thread.repo_root.clone())
            .collect();
        for root in roots {
            if let Ok(project) = silt_git::Repo::discover(&root).and_then(|repo| repo.project()) {
                self.follow_clone_to(&root, &project).await;
            }
        }
    }

    /// Makes the folder of the chat `selector` a project of its own, named
    /// by the folder, and moves the chats recorded in it there, here and on
    /// their server (ADR 0059). Returns the project.
    pub(crate) async fn make_project(&mut self, selector: &str) -> Result<String, CoreError> {
        let thread = self.find_thread(selector)?;
        if !self.is_chat(thread) {
            return Err(CoreError::Invalid(format!("thread {} isn't a chat", thread.short())));
        }
        let root = self
            .chat_folder_here(thread)
            .ok_or_else(|| CoreError::Invalid("the chat's folder isn't on this machine; continue it here first".into()))?;
        let moving: Vec<ThreadId> = self
            .threads
            .values()
            .filter(|record| record.repo_root == root && silt_types::is_chat(&record.repo))
            .map(|record| record.id)
            .collect();
        if moving.iter().any(|thread| {
            self.agent_state(*thread).is_some_and(|state| {
                matches!(
                    state,
                    silt_proto::AgentState::Starting
                        | silt_proto::AgentState::Working
                        | silt_proto::AgentState::AwaitingPermission
                        | silt_proto::AgentState::AwaitingInput
                )
            })
        }) {
            return Err(CoreError::Invalid("an agent is working in the chat; stop it or wait for it first".into()));
        }
        // What the folder names once it leaves the chats, as it has no remote.
        let project = silt_git::normalize_remote(&root.to_string_lossy());
        for thread in &moving {
            if let Some(remote) = self.remote_for(*thread) {
                let body = silt_proto::MoveChat { repo: project.clone(), origin_url: None };
                remote
                    .move_chat(*thread, &body)
                    .await
                    .map_err(|source| CoreError::Server { doing: "move the chat into its project", source })?;
            }
        }
        silt_git::Repo::discover(&root)?.leave_chats()?;
        self.store.move_threads(&moving, silt_types::CHATS, &project, &root).await?;
        for thread in &moving {
            self.threads.get_mut(thread).expect("listed above").repo = project.clone();
        }
        if let Some(crate::homes::Home::Server(url)) = self.new_home(silt_types::CHATS) {
            self.store.set_project_home(&project, crate::homes::Home::Server(url.clone()).stored()).await?;
            self.project_homes.insert(project.clone(), crate::homes::Home::Server(url));
        }
        self.remember_clone(&root).await;
        self.announce(crate::protocol::BusEvent::Threads);
        Ok(project)
    }

    /// Makes the folder `path` and an empty git repository in it, then adds
    /// it, for a project to start from nothing (ADR 0053).
    pub(crate) async fn new_project(&mut self, path: &Path) -> Result<String, CoreError> {
        check_new_folder(path)?;
        let repo = silt_git::Repo::init(path)?;
        self.remember_clone(repo.root()).await;
        Ok(repo.project()?)
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
            // Deep trees pass Windows' 260 characters (ADR 0045).
            let long_paths: &[&str] = if cfg!(windows) { &["-c", "core.longpaths=true"] } else { &[] };
            let output = tokio::process::Command::new("git")
                .args(long_paths)
                .args(["clone", "--quiet", "--", &url])
                .arg(&path)
                .output()
                .await?;
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

    pub(crate) fn notification_settings(
        &self,
        change: Option<silt_proto::KindSetting>,
    ) -> Result<impl Future<Output = Result<Vec<silt_proto::KindSetting>, CoreError>> + Send + 'static, CoreError> {
        let remote = self.signed_in()?;
        Ok(async move {
            let settings = match change {
                Some(setting) => remote.change_notification_setting(&setting).await,
                None => remote.notification_settings().await,
            };
            settings.map_err(|source| CoreError::Server { doing: "read your notification settings", source })
        })
    }

    /// The thread's read-only links on the server, a new one when `new`
    /// says, or with `revoke` revoked (ADR 0027).
    pub(crate) fn thread_links(
        &self,
        selector: &str,
        new: Option<crate::protocol::NewLink>,
        revoke: Option<i64>,
    ) -> Result<impl Future<Output = Result<serde_json::Value, CoreError>> + Send + 'static, CoreError> {
        let thread = self.find_thread(selector)?;
        let remote = self.server_for(thread)?;
        Ok(async move {
            let server = |source| CoreError::Server { doing: "share the thread by link", source };
            let answer = match (new, revoke) {
                (Some(new), _) => serde_json::to_value(remote.make_link(thread, &new).await.map_err(server)?),
                (None, Some(link)) => {
                    remote.revoke_link(thread, link).await.map_err(server)?;
                    serde_json::to_value("revoked")
                }
                (None, None) => serde_json::to_value(remote.links(thread).await.map_err(server)?),
            };
            answer.map_err(|error| CoreError::Invalid(error.to_string()))
        })
    }

    pub(crate) fn pair_phone(
        &self,
    ) -> Result<impl Future<Output = Result<silt_proto::view::PhonePairing, CoreError>> + Send + 'static, CoreError> {
        let remote = self.signed_in()?;
        Ok(async move { remote.pair_phone().await.map_err(|source| CoreError::Server { doing: "pair a phone", source }) })
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
        self.asked_remote().cloned().ok_or_else(|| CoreError::Invalid("not connected to a server; use `silt auth login`".into()))
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
        git_notes: silt_proto::GitNotes::Off,
        forge_comments: false,
        threads: 0,
        open_mrs: 0,
        clone: None,
        home: None,
        server: None,
    }
}

fn origin_url(clone: &Path) -> Option<String> {
    let url = silt_git::Repo::discover(clone).ok()?.remote_url("origin").ok()??;
    Some(silt_proto::shareable_url(&url))
}

/// Where a clone may go, as a new folder may; and a URL git won't read as
/// an option.
fn check_clone_target(url: &str, path: &Path) -> Result<(), CoreError> {
    if url.trim().is_empty() || url.starts_with('-') {
        return Err(CoreError::Invalid("give the URL to clone".into()));
    }
    check_new_folder(path)
}

/// Where a new folder may go: an absolute path whose parent exists, not over
/// something already there.
fn check_new_folder(path: &Path) -> Result<(), CoreError> {
    if !path.is_absolute() {
        return Err(CoreError::Invalid("give the folder an absolute path".into()));
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

/// Each server's projects, by its URL, with what this machine knows of
/// each, then the clones of projects no server lists.
fn merged(servers: Vec<(String, Vec<ProjectInfo>)>, local: Vec<Project>) -> Vec<Project> {
    let here: BTreeMap<String, Project> = local.into_iter().map(|project| (project.repo.clone(), project)).collect();
    let mut listed = std::collections::BTreeSet::new();
    let mut projects: Vec<Project> = Vec::new();
    for (server, infos) in servers {
        for info in infos {
            let mine = here.get(&info.repo);
            listed.insert(info.repo.clone());
            projects.push(Project {
                origin_url: info.origin_url.or_else(|| mine.and_then(|mine| mine.origin_url.clone())),
                open_mrs: mine.map_or(0, |mine| mine.open_mrs),
                clone: mine.and_then(|mine| mine.clone.clone()),
                repo: info.repo,
                name: info.name,
                org: info.org,
                visibility: info.visibility,
                default_agent: info.default_agent,
                default_preset: info.default_preset,
                git_notes: info.git_notes,
                forge_comments: info.forge_comments,
                threads: info.threads,
                home: None,
                server: Some(server.clone()),
            });
        }
    }
    projects.extend(here.into_values().filter(|project| !listed.contains(&project.repo)));
    projects.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.repo.cmp(&b.repo)).then_with(|| a.server.cmp(&b.server)));
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
            git_notes: silt_proto::GitNotes::Off,
            forge_comments: false,
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
            vec![("https://work.example".into(), vec![info("github.com/acme/web", 3), info("github.com/acme/api", 1)])],
            vec![here("github.com/acme/api", "/src/api"), here("example.com/mine", "/src/mine")],
        );
        let names: Vec<&str> = projects.iter().map(|project| project.name.as_str()).collect();
        assert_eq!(names, ["api", "mine", "web"]);
        let api = &projects[0];
        assert_eq!((api.threads, api.open_mrs, api.clone.as_deref()), (1, 2, Some(Path::new("/src/api"))));
        assert_eq!(api.default_agent.as_deref(), Some("claude-acp"));
        assert_eq!(api.origin_url.as_deref(), Some("https://github.com/acme/api.git"));
        assert_eq!(api.server.as_deref(), Some("https://work.example"));
        assert_eq!((projects[1].org.as_deref(), projects[1].threads, projects[1].server.as_deref()), (None, 9, None));
        assert_eq!(projects[2].clone, None);
    }

    #[test]
    fn a_project_on_two_servers_shows_on_each_with_its_clone() {
        let here = Project { clone: Some(PathBuf::from("/src/app")), ..project("github.com/acme/app") };
        let projects = merged(
            vec![
                ("https://work.example".into(), vec![info("github.com/acme/app", 3)]),
                ("https://oss.example".into(), vec![info("github.com/acme/app", 1)]),
            ],
            vec![here],
        );
        let servers: Vec<Option<&str>> = projects.iter().map(|project| project.server.as_deref()).collect();
        assert_eq!(servers, [Some("https://oss.example"), Some("https://work.example")]);
        assert!(projects.iter().all(|project| project.clone.as_deref() == Some(Path::new("/src/app"))));
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
