//! What Settings › Design systems shows and starts (ADR 0088): every system
//! here with what picks it, one system's page, its assets, and the threads
//! that make or change one.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use silt_design::SystemName;

use crate::core::{Core, CoreError};
use crate::design::{self, DesignThread, PERSONAL_DIR, PROJECT_DIR, System};
use crate::protocol::{
    DesignAssetData, DesignCatalog, DesignChat, DesignDetail, DesignFrom, DesignGroup, DesignKeep, DesignProject, DesignSource,
    DesignStart, DesignSummary, StartOptions, ThreadDesign,
};

impl Core {
    /// Every system: each project's, each chat's, then the person's, with
    /// the ones threads are still making in their own folders.
    pub(crate) fn design_catalog(&self) -> DesignCatalog {
        let picks = design::project_choices(&self.data_dir);
        let workspaces = self.workspaces();
        let threads = design::design_threads(&self.data_dir);
        let uses = |system: &System, repo: Option<&str>, clone: Option<&Path>| {
            let name = system.name.as_str();
            let mut uses = Vec::new();
            if let Some(clone) = clone
                && system.source == DesignSource::Project
                && design::project_default(clone).as_deref() == Some(name)
            {
                uses.push(format!("{}'s default", project_name(repo.unwrap_or_default(), clone)));
            }
            for (picked_repo, picked) in &picks {
                if picked == name && (system.source == DesignSource::Personal || Some(picked_repo.as_str()) == repo) {
                    let shown = self
                        .clones
                        .get(picked_repo)
                        .map_or_else(|| picked_repo.clone(), |clone| project_name(picked_repo, clone));
                    uses.push(format!("picked for {shown}"));
                }
            }
            if system.source == DesignSource::Personal {
                for workspace in &workspaces.workspaces {
                    if workspace.defaults.design_system.as_deref() == Some(name) {
                        uses.push(format!("default for {}", workspace.name));
                    }
                }
            }
            uses
        };
        let making = |name: &str, keep: DesignKeep| {
            threads.iter().rev().find(|(_, record)| record.name == name && record.keep == keep).map(|(thread, _)| thread.clone())
        };

        let mut groups = Vec::new();
        let mut clones: Vec<(&String, &PathBuf)> = self.clones.iter().filter(|(repo, _)| !silt_types::is_chat(repo)).collect();
        clones.sort_by_key(|(repo, clone)| project_name(repo, clone));
        for (repo, clone) in clones {
            let here: Vec<System> = design::systems(Some(clone), &self.data_dir)
                .into_iter()
                .filter(|system| system.source == DesignSource::Project)
                .collect();
            let mut systems: Vec<_> = here
                .iter()
                .map(|system| {
                    system.summary(uses(system, Some(repo), Some(clone)), making(system.name.as_str(), DesignKeep::Repo), false)
                })
                .collect();
            // Systems a thread of this project is making in its own checkout,
            // not merged into this clone yet.
            for (thread, record) in threads.iter().filter(|(_, record)| record.keep == DesignKeep::Repo && !record.changing) {
                if here.iter().any(|system| system.name.as_str() == record.name) {
                    continue;
                }
                let Ok(id) = self.find_thread(thread) else { continue };
                if self.threads.get(&id).is_none_or(|known| known.repo != *repo) {
                    continue;
                }
                if let Some(system) = self.thread_system(id, &record.name, DesignSource::Project) {
                    systems.push(system.summary(Vec::new(), Some(thread.clone()), true));
                }
            }
            if !systems.is_empty() {
                let project =
                    DesignProject { repo: repo.clone(), name: project_name(repo, clone), clone: clone.display().to_string() };
                groups.push(DesignGroup { project: Some(project), chat: None, systems });
            }
        }
        groups.extend(self.chat_design_groups());
        let personal: Vec<_> = design::systems(None, &self.data_dir)
            .iter()
            .map(|system| system.summary(uses(system, None, None), making(system.name.as_str(), DesignKeep::Personal), false))
            .collect();
        groups.push(DesignGroup { project: None, chat: None, systems: personal });
        DesignCatalog { groups }
    }

    /// Each chat's own systems, newest chat first: one is often made in a
    /// chat before there's a project to keep it.
    fn chat_design_groups(&self) -> Vec<DesignGroup> {
        let mut folders: Vec<(PathBuf, silt_types::ThreadId)> = self
            .threads
            .keys()
            .filter(|thread| self.is_chat(**thread))
            .filter_map(|thread| Some((self.chat_folder_here(*thread)?, *thread)))
            .collect();
        folders.sort_by(|a, b| b.0.cmp(&a.0));
        folders.dedup_by(|a, b| a.0 == b.0);
        folders
            .into_iter()
            .filter_map(|(folder, thread)| {
                let default = design::project_default(&folder);
                let systems: Vec<DesignSummary> = design::systems(Some(&folder), &self.data_dir)
                    .into_iter()
                    .filter(|system| system.source == DesignSource::Project)
                    .map(|system| {
                        let uses = if default.as_deref() == Some(system.name.as_str()) {
                            vec!["the chat's default".to_owned()]
                        } else {
                            Vec::new()
                        };
                        DesignSummary { chat: true, ..system.summary(uses, Some(thread.to_string()), false) }
                    })
                    .collect();
                if systems.is_empty() {
                    return None;
                }
                let title = self.thread_title(thread).unwrap_or_else(|| "Chat".to_owned());
                let chat = DesignChat { thread: thread.to_string(), title, folder: folder.display().to_string() };
                Some(DesignGroup { project: None, chat: Some(chat), systems })
            })
            .collect()
    }

    /// Copies a system a thread's folder holds, a chat's, to the person's
    /// own, so every project can use it and it syncs.
    pub(crate) fn keep_design_system(&self, thread: &str, name: &str) -> Result<DesignSummary, CoreError> {
        let id = self.find_thread(thread)?;
        let system = self
            .thread_system(id, name, DesignSource::Project)
            .ok_or_else(|| CoreError::Invalid(format!("the thread's folder has no design system {name}")))?;
        crate::design_sync::keep_as_personal(&self.data_dir, system.name.as_str(), system.dir())?;
        let kept = design::systems(None, &self.data_dir)
            .into_iter()
            .find(|system| system.name.as_str() == name)
            .ok_or_else(|| CoreError::Invalid(format!("{name} wasn't kept")))?;
        Ok(kept.summary(Vec::new(), None, false))
    }

    /// One system's page.
    pub(crate) fn design_system(
        &self,
        source: DesignSource,
        name: &str,
        clone: Option<&Path>,
        thread: Option<&str>,
    ) -> Result<DesignDetail, CoreError> {
        let (system, thread) = self.find_design_system(source, name, clone, thread)?;
        let chat = source == DesignSource::Project
            && thread.as_deref().and_then(|thread| self.find_thread(thread).ok()).is_some_and(|id| self.is_chat(id));
        let unmerged = thread.is_some() && !chat;
        Ok(system.detail(DesignSummary { chat, ..system.summary(Vec::new(), thread, unmerged) }))
    }

    /// One of a system's assets, which only its own `assets/` holds.
    pub(crate) fn design_asset(
        &self,
        source: DesignSource,
        name: &str,
        clone: Option<&Path>,
        thread: Option<&str>,
        path: &str,
    ) -> Result<DesignAssetData, CoreError> {
        let (system, _) = self.find_design_system(source, name, clone, thread)?;
        let (bytes, media_type) =
            system.asset(path).ok_or_else(|| CoreError::Invalid(format!("{name} keeps no asset {path}")))?;
        Ok(DesignAssetData { media_type: media_type.to_owned(), data: BASE64.encode(bytes) })
    }

    /// The system named, in a known clone, the person's folder, or the
    /// folder of the thread making it; and that thread, when it's one.
    fn find_design_system(
        &self,
        source: DesignSource,
        name: &str,
        clone: Option<&Path>,
        thread: Option<&str>,
    ) -> Result<(System, Option<String>), CoreError> {
        let missing = || CoreError::Invalid(format!("there's no design system {name} here"));
        if let Some(thread) = thread {
            let id = self.find_thread(thread)?;
            let system = self.thread_system(id, name, source).ok_or_else(missing)?;
            return Ok((system, Some(thread.to_owned())));
        }
        let folder = match (source, clone) {
            (DesignSource::Project, Some(clone)) => {
                if !self.clones.values().any(|known| known == clone) {
                    return Err(CoreError::Invalid(format!("this machine knows no clone at {}", clone.display())));
                }
                Some(clone)
            }
            (DesignSource::Project, None) => {
                return Err(CoreError::Invalid("a repository's design system needs its clone".into()));
            }
            (DesignSource::Personal, _) => None,
        };
        let system = design::systems(folder, &self.data_dir)
            .into_iter()
            .find(|system| system.name.as_str() == name && system.source == source)
            .ok_or_else(missing)?;
        Ok((system, None))
    }

    /// The system `name` as a thread's folder has it: its checkout for a
    /// repository's, the person's folder for theirs.
    fn thread_system(&self, thread: silt_types::ThreadId, name: &str, source: DesignSource) -> Option<System> {
        let folder = match source {
            DesignSource::Project => Some(self.thread_folder(thread)?),
            DesignSource::Personal => None,
        };
        design::systems(folder.as_deref(), &self.data_dir)
            .into_iter()
            .find(|system| system.name.as_str() == name && system.source == source)
    }

    /// Where a thread's files are: its own checkout, else the clone it works
    /// in.
    fn thread_folder(&self, thread: silt_types::ThreadId) -> Option<PathBuf> {
        self.checkouts
            .values()
            .find(|checkout| checkout.record.thread == thread)
            .map(|checkout| checkout.record.path.clone())
            .or_else(|| self.clone_of(thread).ok())
    }

    /// Starts a thread whose agent makes a system, or changes one, with the
    /// design-system skill, and remembers which system it's for.
    pub(crate) async fn design_start(&mut self, start: DesignStart) -> Result<String, CoreError> {
        let changing = start.change.as_deref().is_some_and(|change| !change.trim().is_empty());
        let name: SystemName = if changing {
            start
                .name
                .as_deref()
                .unwrap_or_default()
                .parse()
                .map_err(|_| CoreError::Invalid("which design system to change?".into()))?
        } else {
            design::slug(&start.title)
                .ok_or_else(|| CoreError::Invalid("a design system's name needs a letter or a digit".into()))?
        };
        let title = if start.title.trim().is_empty() { name.to_string() } else { start.title.trim().to_owned() };
        let clone = start.clone.clone();
        if let Some(clone) = &clone
            && !self.clones.values().any(|known| known == clone)
        {
            return Err(CoreError::Invalid(format!("this machine knows no clone at {}", clone.display())));
        }
        if start.keep == DesignKeep::Repo && clone.is_none() {
            return Err(CoreError::Invalid("a repository's design system needs its project".into()));
        }
        let personal = self.data_dir.join(PERSONAL_DIR).join(name.as_str());
        if changing {
            let exists = match start.keep {
                DesignKeep::Repo => {
                    clone.as_ref().is_some_and(|clone| clone.join(PROJECT_DIR).join(name.as_str()).join("tokens.json").is_file())
                }
                DesignKeep::Personal => personal.join("tokens.json").is_file(),
            };
            if !exists {
                return Err(CoreError::Invalid(format!("there's no design system {name} to change")));
            }
        } else {
            let from = start.from.unwrap_or(DesignFrom::Code);
            let has = |text: &Option<String>| text.as_deref().is_some_and(|text| !text.trim().is_empty());
            match from {
                DesignFrom::Code if clone.is_none() => {
                    return Err(CoreError::Invalid("a design system from code needs its project".into()));
                }
                DesignFrom::Website if !has(&start.link) && start.attachments.is_empty() => {
                    return Err(CoreError::Invalid("give the website's address, or attach a brand guide or screenshots".into()));
                }
                DesignFrom::Website if has(&start.link) && !web_address(start.link.as_deref().unwrap_or_default()) => {
                    return Err(CoreError::Invalid("the website's address starts with https:// or http://".into()));
                }
                DesignFrom::Description if !has(&start.notes) && start.attachments.is_empty() => {
                    return Err(CoreError::Invalid("describe the design system to make".into()));
                }
                _ => {}
            }
            let taken = match start.keep {
                DesignKeep::Repo => clone.as_ref().is_some_and(|clone| clone.join(PROJECT_DIR).join(name.as_str()).exists()),
                DesignKeep::Personal => personal.exists(),
            };
            if taken {
                return Err(CoreError::Invalid(format!("there's already a design system {name}; change it instead")));
            }
        }
        let dir = match start.keep {
            DesignKeep::Repo => format!("{PROJECT_DIR}/{name}"),
            DesignKeep::Personal => personal.display().to_string(),
        };
        let project = clone.as_deref().map(|clone| {
            let repo =
                self.clones.iter().find(|(_, known)| known.as_path() == clone).map(|(repo, _)| repo.as_str()).unwrap_or_default();
            project_name(repo, clone)
        });
        let prompt = design::start_prompt(&start, &name, &dir, project.as_deref());
        let options = StartOptions {
            prompt: Some(prompt),
            attachments: start.attachments.clone(),
            chat: clone.is_none(),
            ..StartOptions::default()
        };
        let thread = self.acp_start(None, clone.as_deref(), &start.agent, options).await?;
        design::record_design_thread(
            &self.data_dir,
            &thread,
            DesignThread { name: name.to_string(), title, keep: start.keep, changing },
        )?;
        Ok(thread)
    }

    /// The system a thread was started for, and what it holds so far.
    pub(crate) fn thread_design(&self, thread: &str) -> Result<Option<ThreadDesign>, CoreError> {
        let id = self.find_thread(thread)?;
        let Some(record) = design::design_threads(&self.data_dir)
            .into_iter()
            .find(|(known, _)| self.find_thread(known).ok() == Some(id))
            .map(|(_, record)| record)
        else {
            return Ok(self.written_design(id, thread));
        };
        let source = match record.keep {
            DesignKeep::Repo => DesignSource::Project,
            DesignKeep::Personal => DesignSource::Personal,
        };
        let detail = self.thread_system(id, &record.name, source).map(|system| {
            let unmerged = source == DesignSource::Project;
            system.detail(system.summary(Vec::new(), Some(thread.to_owned()), unmerged))
        });
        Ok(Some(ThreadDesign { name: record.name, title: record.title, keep: record.keep, changing: record.changing, detail }))
    }

    /// The system a thread asked in its own words wrote: the one written
    /// last since it loaded the design-system skill, in its folder or the
    /// person's; else, for a chat, the one its folder last had written.
    fn written_design(&self, id: silt_types::ThreadId, thread: &str) -> Option<ThreadDesign> {
        let folder = self.thread_folder(id);
        let since = design::skill_threads(&self.data_dir)
            .into_iter()
            .find(|(known, _)| self.find_thread(known).ok() == Some(id))
            .map(|(_, since_ms)| since_ms);
        let with_written = |systems: Vec<System>| -> Vec<(System, design::Written)> {
            systems
                .into_iter()
                .filter_map(|system| {
                    let written = design::written(system.dir())?;
                    Some((system, written))
                })
                .collect()
        };
        let chat = self.is_chat(id);
        let found = since
            .and_then(|since_ms| {
                design::written_since(with_written(design::systems(folder.as_deref(), &self.data_dir)), since_ms)
            })
            .or_else(|| {
                let folder = folder.as_deref().filter(|_| chat)?;
                let own = design::systems(Some(folder), &self.data_dir)
                    .into_iter()
                    .filter(|system| system.source == DesignSource::Project)
                    .collect();
                design::written_since(with_written(own), i64::MIN).map(|(system, _)| (system, false))
            });
        let (system, changing) = found?;
        let in_folder = system.source == DesignSource::Project;
        let summary = system.summary(Vec::new(), Some(thread.to_owned()), in_folder && !chat);
        let summary = DesignSummary { chat: in_folder && chat, ..summary };
        Some(ThreadDesign {
            name: summary.name.clone(),
            title: summary.title.clone(),
            keep: if in_folder { DesignKeep::Repo } else { DesignKeep::Personal },
            changing,
            detail: Some(system.detail(summary)),
        })
    }
}

/// A project's name as the sidebar shows it: its clone's folder.
fn project_name(repo: &str, clone: &Path) -> String {
    clone
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| repo.rsplit('/').next().unwrap_or(repo).to_owned())
}

fn web_address(link: &str) -> bool {
    let link = link.trim();
    (link.starts_with("https://") || link.starts_with("http://")) && !link.contains(char::is_whitespace) && link.len() <= 2048
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_website_is_an_http_address() {
        assert!(web_address("https://acme.example/brand"));
        assert!(!web_address("file:///etc/passwd"));
        assert!(!web_address("https://a b"));
        assert!(!web_address("javascript:alert(1)"));
    }

    #[test]
    fn a_project_is_named_by_its_clones_folder() {
        assert_eq!(project_name("github.com/acme/web", Path::new("/src/web-app")), "web-app");
    }
}
