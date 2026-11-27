//! Settings › Memory (ADR 0137): what each agent remembers, read from its
//! own stores, everywhere and in each project with a clone here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use silt_agents::memory::{self, Kind, Store};
use silt_types::ThreadId;

use crate::core::{Core, CoreError};
use crate::protocol::{
    MemoryCount, MemoryEntryView, MemoryOverview, MemoryProject, MemoryStoreView, MemoryThread, MemoryView, RulesFileView,
};

/// A project's rules files, and the agents that read each.
const RULES_FILES: &[(&str, &[&str])] = &[
    ("CLAUDE.md", &["claude-code", "copilot-cli", "opencode"]),
    ("AGENTS.md", &["codex", "qudiks", "opencode", "antigravity-cli", "copilot-cli"]),
    ("GEMINI.md", &["gemini-cli", "antigravity-cli", "copilot-cli"]),
    (".github/copilot-instructions.md", &["copilot-cli"]),
];

/// An agent's store, with whose it is.
struct Found {
    agent: &'static str,
    store: Store,
}

impl Core {
    pub(crate) async fn memory_overview(&self) -> Result<MemoryOverview, CoreError> {
        let threads = self.session_threads().await?;
        let everywhere = count(&self.read_stores(None, &threads));
        let mut projects: Vec<MemoryProject> = self
            .clones
            .iter()
            .filter(|(repo, _)| !silt_types::is_chat(repo))
            .map(|(repo, clone)| MemoryProject {
                name: project_name(repo, clone),
                clone: clone.clone(),
                count: count(&self.read_stores(Some((repo, clone)), &threads)),
            })
            .collect();
        projects.sort_by(|a, b| b.count.changed_ms.cmp(&a.count.changed_ms).then_with(|| a.name.cmp(&b.name)));
        Ok(MemoryOverview { everywhere, projects })
    }

    pub(crate) async fn memory_of(&self, clone: Option<&Path>) -> Result<MemoryView, CoreError> {
        let threads = self.session_threads().await?;
        let Some(clone) = clone else {
            return Ok(MemoryView { project: None, clone: None, stores: self.read_stores(None, &threads), rules: Vec::new() });
        };
        let (repo, clone) = self.known_clone(clone)?;
        let rules = RULES_FILES
            .iter()
            .map(|(file, readers)| RulesFileView {
                file: (*file).to_owned(),
                readers: readers.iter().map(|reader| (*reader).to_owned()).collect(),
                text: std::fs::read_to_string(clone.join(file)).ok(),
            })
            .collect();
        Ok(MemoryView {
            project: Some(project_name(&repo, &clone)),
            clone: Some(clone.clone()),
            stores: self.read_stores(Some((&repo, &clone)), &threads),
            rules,
        })
    }

    pub(crate) fn set_memory(&self, clone: Option<&Path>, store: &str, file: Option<&str>, text: &str) -> Result<(), CoreError> {
        let found = self.store_shown_as(clone, store)?;
        if !found.store.editable() {
            return Err(CoreError::Invalid(format!("{store} is rewritten by its agent, so Silt doesn't change it")));
        }
        match (found.store.kind, file) {
            (Kind::Memory, Some(file)) => memory::write_entry(&found.store.path, file, text),
            (Kind::Memory, None) => return Err(CoreError::Invalid("name the memory to change".into())),
            (_, Some(_)) => return Err(CoreError::Invalid(format!("{store} is one file"))),
            (_, None) => memory::write_whole(&found.store.path, text),
        }
        .map_err(|error| CoreError::Invalid(format!("couldn't save to {store}: {error}")))
    }

    pub(crate) fn forget_memory(&self, clone: Option<&Path>, store: &str, file: &str) -> Result<(), CoreError> {
        let found = self.store_shown_as(clone, store)?;
        if found.store.kind != Kind::Memory {
            return Err(CoreError::Invalid(format!("{store} isn't a folder of memories")));
        }
        memory::forget_entry(&found.store.path, file)
            .map_err(|error| CoreError::Invalid(format!("couldn't forget {file}: {error}")))
    }

    /// The store Silt lists as `shown` there, so a request can only name
    /// one of those.
    fn store_shown_as(&self, clone: Option<&Path>, shown: &str) -> Result<Found, CoreError> {
        let known = clone.map(|clone| self.known_clone(clone)).transpose()?;
        let home = silt_os::home_dir();
        self.stores(known.as_ref().map(|(repo, clone)| (repo.as_str(), clone.as_path())))
            .into_iter()
            .find(|found| shown_path(&found.store.path, home.as_deref()) == shown)
            .ok_or_else(|| CoreError::Invalid(format!("no agent keeps its memory in {shown} here")))
    }

    fn known_clone(&self, clone: &Path) -> Result<(String, PathBuf), CoreError> {
        self.clones
            .iter()
            .find(|(_, known)| known.as_path() == clone)
            .map(|(repo, known)| (repo.clone(), known.clone()))
            .ok_or_else(|| CoreError::Invalid(format!("this machine knows no clone at {}", clone.display())))
    }

    /// Every agent's stores there, each path once: Antigravity reads Gemini
    /// CLI's `GEMINI.md` too.
    fn stores(&self, project: Option<(&str, &Path)>) -> Vec<Found> {
        let Some(home) = silt_os::home_dir() else { return Vec::new() };
        let workspace_env = project.map(|(repo, clone)| self.workspace_env(repo, clone)).unwrap_or_default();
        let env = |name: &str| workspace_env.get(name).cloned().or_else(|| std::env::var(name).ok());
        let mut found: Vec<Found> = Vec::new();
        for agent in silt_agents::AGENTS {
            for store in agent.memory_stores(&home, &env, project.map(|(_, clone)| clone)) {
                if !found.iter().any(|other| other.store.path == store.path) {
                    found.push(Found { agent: agent.id(), store });
                }
            }
        }
        found
    }

    /// The variables the project's workspace gives its agents, such as its
    /// own `CLAUDE_CONFIG_DIR` (ADR 0065).
    fn workspace_env(&self, repo: &str, clone: &Path) -> std::collections::BTreeMap<String, String> {
        let clone = clone.to_string_lossy().replace('\\', "/");
        let workspaces = self.workspaces();
        workspaces.resolve(repo, Some(&clone), self.project_workspaces.get(repo)).workspace.env.clone()
    }

    /// The stores there with what they hold, leaving out those not there.
    fn read_stores(&self, project: Option<(&String, &PathBuf)>, threads: &HashMap<String, ThreadId>) -> Vec<MemoryStoreView> {
        let home = silt_os::home_dir();
        self.stores(project.map(|(repo, clone)| (repo.as_str(), clone.as_path())))
            .into_iter()
            .filter_map(|Found { agent, store }| {
                let mut view = MemoryStoreView {
                    agent: agent.to_owned(),
                    kind: kind_name(store.kind).to_owned(),
                    path: shown_path(&store.path, home.as_deref()),
                    editable: store.editable(),
                    entries: Vec::new(),
                    text: None,
                    error: None,
                };
                match store.kind {
                    Kind::Memory => match memory::read_indexed(&store.path) {
                        Ok(entries) if entries.is_empty() => return None,
                        Ok(entries) => {
                            view.entries = entries.into_iter().map(|entry| self.entry_view(entry, threads)).collect();
                        }
                        Err(error) => view.error = Some(error.to_string()),
                    },
                    Kind::Rules | Kind::Summary => match std::fs::read_to_string(&store.path) {
                        Ok(text) if text.trim().is_empty() => return None,
                        Ok(text) => view.text = Some(text),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
                        Err(error) => view.error = Some(error.to_string()),
                    },
                }
                Some(view)
            })
            .collect()
    }

    fn entry_view(&self, entry: memory::Entry, threads: &HashMap<String, ThreadId>) -> MemoryEntryView {
        let thread = entry
            .session
            .as_ref()
            .and_then(|session| threads.get(session))
            .map(|thread| MemoryThread { id: thread.to_string(), title: self.thread_title(*thread) });
        MemoryEntryView {
            file: entry.file,
            name: entry.name,
            description: entry.description,
            kind: entry.kind,
            body: entry.body,
            text: entry.text,
            modified_ms: entry.modified_ms,
            thread,
        }
    }

    /// The thread of each session Silt recorded, by the agent's session id.
    async fn session_threads(&self) -> Result<HashMap<String, ThreadId>, CoreError> {
        Ok(self.store.sessions().await?.into_iter().map(|record| (record.session_id, record.thread)).collect())
    }
}

/// What the stores there hold, for the overview.
fn count(stores: &[MemoryStoreView]) -> MemoryCount {
    let mut count = MemoryCount::default();
    for store in stores {
        if store.error.is_some() {
            count.unread.push(store.agent.clone());
            continue;
        }
        let changed = store.entries.iter().filter_map(|entry| entry.modified_ms).max();
        count.changed_ms = count.changed_ms.max(changed);
        count.memories += store.entries.len();
        count.files += usize::from(store.text.is_some());
        if !store.entries.is_empty() && !count.agents.contains(&store.agent) {
            count.agents.push(store.agent.clone());
        }
    }
    count
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Memory => "memory",
        Kind::Rules => "rules",
        Kind::Summary => "summary",
    }
}

/// `path` with `~` for the home folder, as the person knows it.
fn shown_path(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) => format!("~/{}", rest.to_string_lossy().replace('\\', "/")),
        None => path.to_string_lossy().replace('\\', "/"),
    }
}

fn project_name(repo: &str, clone: &Path) -> String {
    clone.file_name().and_then(|name| name.to_str()).map(str::to_owned).unwrap_or_else(|| silt_proto::project_name(repo))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(agent: &str, entries: usize, text: bool, error: bool) -> MemoryStoreView {
        let entry = MemoryEntryView {
            file: "a.md".into(),
            name: "a".into(),
            description: String::new(),
            kind: None,
            body: String::new(),
            text: String::new(),
            modified_ms: Some(entries as i64),
            thread: None,
        };
        MemoryStoreView {
            agent: agent.into(),
            kind: "memory".into(),
            path: "~/x".into(),
            editable: true,
            entries: vec![entry; entries],
            text: text.then(String::new),
            error: error.then(|| "denied".into()),
        }
    }

    #[test]
    fn a_count_says_what_was_read_and_what_wasnt() {
        let counted =
            count(&[store("claude-code", 3, false, false), store("codex", 0, true, false), store("gemini-cli", 0, false, true)]);
        assert_eq!(counted.memories, 3);
        assert_eq!(counted.files, 1);
        assert_eq!(counted.agents, ["claude-code"]);
        assert_eq!(counted.changed_ms, Some(3));
        assert_eq!(counted.unread, ["gemini-cli"]);
    }

    #[test]
    fn paths_are_shown_from_home() {
        let home = Path::new("/home/u");
        assert_eq!(shown_path(Path::new("/home/u/.claude/CLAUDE.md"), Some(home)), "~/.claude/CLAUDE.md");
        assert_eq!(shown_path(Path::new("/etc/x"), Some(home)), "/etc/x");
    }
}
