//! Erasing a Local only thread (ADR 0092): the thread and its forks, what
//! the store holds of them, their blobs no other thread needs, their managed
//! checkouts, and the agents' own copies of their sessions. A marker written
//! first lists what's to go, so an erasure a crash stopped is finished at the
//! next start.

use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use silt_log::{ConvOp, Op};
use silt_types::{BlobHash, FamilyId, ThreadId};
use tracing::warn;

use crate::core::{Core, CoreError};

const MARKERS: &str = "erasing";

/// What an erasure removes, kept until it's done.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Erasure {
    pub(crate) threads: Vec<ThreadId>,
    /// The agents' own session files, and folders of theirs.
    pub(crate) files: Vec<PathBuf>,
    /// Managed checkouts: the clone they're of, their id and their folder.
    pub(crate) checkouts: Vec<(PathBuf, String, PathBuf)>,
    /// Blobs the erased deltas name; those no remaining delta needs go.
    pub(crate) blobs: Vec<BlobHash>,
}

impl Erasure {
    /// What it removes, said to the person before they confirm.
    pub(crate) fn said(&self) -> String {
        let threads = self.threads.iter().map(|thread| thread.short()).collect::<Vec<_>>().join(", ");
        let mut said = format!(
            "erasing removes thread{} {threads} from this machine for good: its history, search entries and files",
            if self.threads.len() == 1 { "" } else { "s" }
        );
        if !self.checkouts.is_empty() {
            said.push_str(&format!(
                ", {} managed checkout{}",
                self.checkouts.len(),
                if self.checkouts.len() == 1 { "" } else { "s" }
            ));
        }
        if !self.files.is_empty() {
            said.push_str(&format!(", and {} of the agents' own session files", self.files.len()));
        }
        said.push_str(". What you copied elsewhere, commits pushed and backups of the disk stay.");
        said
    }
}

impl Core {
    /// The thread and every fork of it, forks of forks included.
    fn with_forks(&self, thread: ThreadId) -> Vec<ThreadId> {
        let Some(family) = self.threads.get(&thread).map(|record| record.family) else {
            return vec![thread];
        };
        let mut erased = vec![thread];
        let Some(projected) = self.families.get(&family) else {
            return erased;
        };
        loop {
            let more: Vec<ThreadId> = projected
                .threads()
                .filter(|(id, state)| !erased.contains(id) && state.forked_from().is_some_and(|from| erased.contains(&from)))
                .map(|(id, _)| *id)
                .collect();
            if more.is_empty() {
                return erased;
            }
            erased.extend(more);
        }
    }

    /// What erasing `selector` would remove, refused for a thread that isn't
    /// in a Local only workspace.
    pub(crate) async fn erasure_of(&self, selector: &str) -> Result<Erasure, CoreError> {
        let thread = self.find_thread(selector)?;
        let family =
            self.threads.get(&thread).map(|record| record.family).ok_or_else(|| CoreError::Invalid("no such thread".into()))?;
        let workspace = self.family_workspace(family);
        if !self.effective_policy(&workspace).local_only {
            return Err(CoreError::Invalid(format!(
                "only threads of a Local only workspace can be erased, and {} isn't one; `silt thread delete` and `silt thread purge` remove others",
                workspace.name
            )));
        }
        let threads = self.with_forks(thread);
        let erasing: HashSet<ThreadId> = threads.iter().copied().collect();
        let log = self.local_log(family);
        let mut files = Vec::new();
        let mut blobs = Vec::new();
        let local_sessions = self.data_dir.join("local").join("sessions");
        for delta in log.iter().filter(|delta| erasing.contains(&delta.thread)) {
            if let Some(blob) = delta.blob() {
                blobs.push(blob);
            }
            if let Op::Conv(ConvOp::Native { session, line, .. }) = &delta.op
                && session.agent == silt_agents::acp::AGENT
                && let Some((agent, native)) = silt_agents::acp::session_of(line)
                && agent == silt_acp::registry::LOCAL_AGENT
                && silt_local::is_session_id(&native)
            {
                files.push(local_sessions.join(format!("{native}.jsonl")));
            }
        }
        for session in self.store.sessions().await?.into_iter().filter(|session| erasing.contains(&session.thread)) {
            let agent = silt_agents::agent(&session.agent);
            for offset in self.store.session_offsets(&session.agent, &session.session_id).await? {
                if let Some(dir) = agent.and_then(|agent| agent.session_dir(&offset.path)) {
                    files.push(dir);
                }
                files.push(offset.path);
            }
        }
        files
            .extend(threads.iter().map(|thread| crate::acp::attachments_dir(&self.data_dir, *thread)).filter(|dir| dir.exists()));
        let family_gone =
            self.threads.values().filter(|record| record.family == family).all(|record| erasing.contains(&record.id));
        if let Some(record) = self.threads.get(&thread).filter(|record| family_gone && silt_types::is_chat(&record.repo)) {
            files.push(record.repo_root.clone());
        }
        files.sort();
        files.dedup();
        blobs.sort();
        blobs.dedup();
        let checkouts = self
            .checkouts
            .values()
            .filter(|checkout| erasing.contains(&checkout.record.thread) && checkout.record.managed)
            .filter_map(|checkout| {
                let root = self.threads.get(&checkout.record.thread)?.repo_root.clone();
                Some((root, checkout.record.thread.to_string(), checkout.record.path.clone()))
            })
            .collect();
        Ok(Erasure { threads, files, checkouts, blobs })
    }

    /// Erases `selector` once confirmed; unconfirmed, says what would go.
    pub(crate) async fn erase(&mut self, selector: &str, confirmed: bool) -> Result<String, CoreError> {
        let erasure = self.erasure_of(selector).await?;
        let said = erasure.said();
        if !confirmed {
            return Ok(said);
        }
        let marker = self.marker(erasure.threads[0]);
        let bytes = serde_json::to_vec(&erasure).map_err(|error| CoreError::Invalid(error.to_string()))?;
        silt_os::create_private_dir(&self.data_dir.join(MARKERS))?;
        silt_os::write_private(&marker, &bytes)?;
        self.carry_out(&erasure).await?;
        std::fs::remove_file(&marker)?;
        self.announce(crate::protocol::BusEvent::Threads);
        Ok(format!("erased {}", erasure.threads.iter().map(|thread| thread.short()).collect::<Vec<_>>().join(", ")))
    }

    fn marker(&self, thread: ThreadId) -> PathBuf {
        self.data_dir.join(MARKERS).join(format!("{thread}.json"))
    }

    /// Whether a marker read back asks only what an erasure of Local only
    /// threads would: anything else in it, such as a file elsewhere, means
    /// it isn't one Silt wrote, and nothing of it is done.
    fn trusted(&self, erasure: &Erasure) -> bool {
        let threads_local = erasure.threads.iter().all(|thread| {
            self.threads.get(thread).is_none_or(|record| self.effective_policy(&self.family_workspace(record.family)).local_only)
        });
        let roots = self.erasable_roots();
        let files_inside = erasure.files.iter().all(|file| inside(file, &roots));
        let checkouts_managed = erasure.checkouts.iter().all(|(clone, id, path)| {
            id.parse::<ThreadId>().is_ok_and(|parsed| parsed.to_string() == *id)
                && plain(clone)
                && *path == clone.join(".silt").join("checkouts").join(id)
        });
        threads_local && files_inside && checkouts_managed
    }

    /// Finishes the erasures a stop interrupted.
    pub(crate) async fn finish_erasures(&mut self) {
        let Ok(entries) = std::fs::read_dir(self.data_dir.join(MARKERS)) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(erasure) = std::fs::read(&path).ok().and_then(|bytes| serde_json::from_slice::<Erasure>(&bytes).ok()) else {
                continue;
            };
            if !self.trusted(&erasure) {
                warn!(marker = %path.display(), "not finishing an erasure whose marker asks more than an erasure would");
                continue;
            }
            match self.carry_out(&erasure).await {
                Ok(()) => {
                    let _ = std::fs::remove_file(&path);
                }
                Err(error) => warn!(%error, "could not finish erasing a thread; it's tried again at the next start"),
            }
        }
    }

    /// Every step, each safe to run again.
    async fn carry_out(&mut self, erasure: &Erasure) -> Result<(), CoreError> {
        let here: HashSet<ThreadId> =
            erasure.threads.iter().copied().filter(|thread| self.threads.contains_key(thread)).collect();
        let families: HashSet<FamilyId> =
            here.iter().filter_map(|thread| self.threads.get(thread).map(|record| record.family)).collect();
        self.drop_threads(&here).await?;
        for thread in &erasure.threads {
            self.store.remove_run_tokens(*thread).await?;
            // What its sandboxes asked of plugins, and reached, goes too.
            self.store.remove_plugin_calls(*thread).await?;
            let log = crate::sandbox::access_log(&self.data_dir, &thread.to_string());
            if let Err(error) = std::fs::remove_file(&log)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                return Err(error.into());
            }
            #[cfg(unix)]
            self.release_devices(*thread);
        }
        for family in families {
            if !self.threads.values().any(|record| record.family == family) {
                self.store.forget_family(family).await?;
                self.family_workspaces.remove(&family);
                self.homes.remove(&family);
            }
        }
        let allowed = self.erasable_roots();
        for file in &erasure.files {
            if !inside(file, &allowed) {
                warn!(path = %file.display(), "not erasing a file outside the agents' session folders");
                continue;
            }
            let removed = if file.is_dir() { std::fs::remove_dir_all(file) } else { std::fs::remove_file(file) };
            if let Err(error) = removed
                && error.kind() != std::io::ErrorKind::NotFound
            {
                return Err(error.into());
            }
        }
        for (clone, id, path) in &erasure.checkouts {
            let managed = id.parse::<ThreadId>().is_ok_and(|parsed| parsed.to_string() == *id)
                && plain(clone)
                && *path == clone.join(".silt").join("checkouts").join(id);
            let Some(repo) = managed.then(|| silt_git::Repo::discover(clone).ok()).flatten() else {
                continue;
            };
            let checkout = silt_git::ManagedCheckout { path: path.clone(), git_dir: repo.managed_git_dir(id) };
            repo.remove_managed_checkout(id, &checkout)?;
        }
        let needed: HashSet<BlobHash> = self.every_log().flat_map(|log| log.iter().filter_map(silt_log::Delta::blob)).collect();
        let blobs = self.store.blobs();
        for blob in erasure.blobs.iter().filter(|blob| !needed.contains(blob)) {
            blobs.remove(*blob).await?;
        }
        self.models.unload().await;
        self.store.compact_erased().await?;
        Ok(())
    }

    /// Where erasure may remove files from: the agents' own folders, by the
    /// variables that move them and where they are by default, chats'
    /// folders, and the local agent's sessions.
    fn erasable_roots(&self) -> Vec<PathBuf> {
        const HOMES: [(&str, &str); 5] = [
            ("CLAUDE_CONFIG_DIR", ".claude"),
            ("CODEX_HOME", ".codex"),
            ("QUDIKS_HOME", ".qudiks"),
            ("GEMINI_CLI_HOME", ".gemini"),
            ("COPILOT_HOME", ".copilot"),
        ];
        let mut roots = vec![
            self.data_dir.join("local").join("sessions"),
            self.data_dir.join("attachments"),
            crate::chats::chats_dir(&self.data_dir),
        ];
        let home = silt_os::home_dir();
        for (variable, folder) in HOMES {
            roots.extend(std::env::var_os(variable).map(PathBuf::from));
            roots.extend(home.as_ref().map(|home| home.join(folder)));
            for workspace in &self.workspaces().workspaces {
                roots.extend(workspace.env.get(variable).map(PathBuf::from));
            }
        }
        roots.retain(|root| root.is_absolute() && plain(root) && root.components().count() > 2);
        roots
    }

    /// Each family's log here.
    fn every_log(&self) -> impl Iterator<Item = &[silt_log::Delta]> {
        let families: HashSet<FamilyId> = self.families.keys().chain(self.raw.keys()).copied().collect();
        families.into_iter().map(|family| self.local_log(family)).collect::<Vec<_>>().into_iter()
    }
}

/// A path with no `.` or `..` in it, absolute, so where it points is plain.
fn plain(path: &std::path::Path) -> bool {
    path.is_absolute()
        && path.components().all(|part| {
            matches!(part, std::path::Component::Normal(_) | std::path::Component::RootDir | std::path::Component::Prefix(_))
        })
}

/// Whether `path` is strictly within one of `roots`, read without the disk.
fn inside(path: &std::path::Path, roots: &[PathBuf]) -> bool {
    plain(path) && roots.iter().any(|root| path.starts_with(root) && path != root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_paths_strictly_inside_a_root_are_erased() {
        let roots = [PathBuf::from("/home/me/.claude")];
        assert!(inside(std::path::Path::new("/home/me/.claude/projects/x/s.jsonl"), &roots));
        assert!(!inside(std::path::Path::new("/home/me/.claude"), &roots), "never the root itself");
        assert!(!inside(std::path::Path::new("/home/me/.claude/../../me"), &roots), "nothing climbing out");
        assert!(!inside(std::path::Path::new("/home/me/notes"), &roots));
        assert!(!inside(std::path::Path::new("relative/.claude/x"), &roots));
    }
}
