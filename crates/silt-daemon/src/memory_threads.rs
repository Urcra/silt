//! A person's memory as families (ADR 0137): each store, the memory
//! everywhere and each project's, is a thread of a reserved memory project
//! whose checkout is the store's folder. It syncs to the person's servers
//! like any private thread, and edits from any machine land in every
//! machine's folder, merged as text.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use silt_agents::memory;
use silt_types::ThreadId;
use tracing::warn;

use crate::core::{Core, CoreError, now_ms};
use crate::homes::Home;

/// How often the stores are checked for a thread.
const EVERY: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(crate) struct MemoryThreads {
    last: Option<Instant>,
}

impl MemoryThreads {
    /// Looks at the stores again at the next tick.
    pub(crate) fn wake(&mut self) {
        self.last = None;
    }
}

impl Core {
    /// Gives each store of memory here its thread, while agents share
    /// memory: adopting the one a server lists, else making one once the
    /// server that would hold it has listed what it holds, so two machines
    /// don't each make one.
    pub(crate) async fn drive_memory_threads(&mut self) {
        if !self.resident || !memory::sharing(&self.data_dir) {
            return;
        }
        if self.memory_threads.last.is_some_and(|last| last.elapsed() < EVERY) {
            return;
        }
        self.memory_threads.last = Some(Instant::now());
        for (project, folder) in self.memory_places() {
            if let Err(error) = self.keep_memory_thread(project.as_deref(), &folder).await {
                warn!(%error, folder = %folder.display(), "couldn't keep this memory in a thread");
            }
        }
    }

    /// The stores of memory here: the memory everywhere, and each project's
    /// with a clone here.
    fn memory_places(&self) -> Vec<(Option<String>, PathBuf)> {
        let mut places = vec![(None, memory::everywhere_dir(&self.data_dir))];
        for (repo, clone) in &self.clones {
            if silt_types::is_personal(repo) {
                continue;
            }
            if let Some(folder) = self.project_memory_store(repo, clone) {
                places.push((Some(repo.clone()), folder));
            }
        }
        places
    }

    async fn keep_memory_thread(&mut self, project: Option<&str>, folder: &Path) -> Result<(), CoreError> {
        if self.checkouts.values().any(|checkout| checkout.record.path == folder) {
            return Ok(());
        }
        let name = silt_types::memory_project(project);
        if !folder.join(".git").exists() {
            silt_git::Repo::init_memory(folder, &name)?;
        }
        // The oldest thread of the project, should two machines have made
        // one each before either heard of the other's.
        let listed = self
            .threads
            .values()
            .filter(|record| record.repo == name)
            .min_by_key(|record| (record.created_ms, record.id))
            .map(|record| record.id);
        match listed {
            Some(thread) => self.adopt_memory_thread(thread, folder).await,
            None if self.may_make_memory_thread(project) => self.create_thread(folder, false, true, None).await.map(drop),
            None => Ok(()),
        }
    }

    /// Whether this machine may make the store's thread: the server that
    /// would hold it has listed what it holds since the daemon started, or
    /// the store stays on this machine.
    fn may_make_memory_thread(&self, project: Option<&str>) -> bool {
        let home = match project {
            Some(project) => self.new_home(project),
            None => self.workspace_home(self.workspaces().default_workspace()),
        };
        match home {
            Some(Home::Server(server)) => self.listed_from.contains(&server),
            Some(Home::Local) => true,
            None => self.remotes.is_empty(),
        }
    }

    /// Takes on the memory thread another machine made, in this machine's
    /// folder: its files are written there, and what the folder held that
    /// the thread hasn't is brought in, as edits the thread then records.
    async fn adopt_memory_thread(&mut self, thread: ThreadId, folder: &Path) -> Result<(), CoreError> {
        let kept = self.data_dir.join("runs").join(format!("memory-{}-{}", thread.short(), now_ms()));
        std::fs::create_dir_all(&kept)?;
        let result = async {
            for entry in std::fs::read_dir(folder)?.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if (memory::is_entry_file(&name) || name == memory::INDEX) && entry.path().is_file() {
                    std::fs::copy(entry.path(), kept.join(&name))?;
                }
            }
            self.check_out_memory(thread, folder).await?;
            memory::merge_into(&kept, folder, "here")?;
            Ok::<_, CoreError>(())
        }
        .await;
        let _ = std::fs::remove_dir_all(&kept);
        result
    }
}
