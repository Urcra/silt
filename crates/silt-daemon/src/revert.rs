//! Reverting a thread to one of its turns (ADR 0029): the files go back to
//! how that turn left them, recorded as new changes, and the turns after it
//! are marked rewound rather than deleted.

use std::path::Path;

use silt_log::{Author, ConvOp, FileView, Op};
use silt_types::{DeltaId, ThreadId};

use crate::core::{Core, CoreError};
use crate::disk;
use crate::protocol::Reverted;

impl Core {
    pub(crate) async fn revert(&mut self, selector: Option<&str>, cwd: &Path, turn: usize) -> Result<Reverted, CoreError> {
        let thread = self.thread_at(selector, cwd)?;
        self.ensure_writer(thread)?;
        if self.agent_live(thread) {
            return Err(CoreError::Invalid("an agent is working in this thread; stop it before reverting".into()));
        }
        let end = self.turn_end(thread, turn).ok_or_else(|| CoreError::Invalid(format!("the thread has no turn {turn}")))?;
        let family_id = self.threads[&thread].family;
        let to = last_conversation_before(&self.families[&family_id].log()[..end], thread)
            .ok_or_else(|| CoreError::Invalid(format!("turn {turn} has no conversation to go back to")))?;
        let (checkout_id, root) = self.checkout_for_agent(thread).await?;
        let files = self.write_turn_files(thread, turn, &root).await?;
        self.ingest_paths(checkout_id, files.iter().map(|path| root.join(path)).collect()).await?;
        self.append_conv(family_id, thread, Author::User, ConvOp::Rewound { to }).await?;
        Ok(Reverted { thread: thread.to_string(), turn, files })
    }

    /// Writes into the checkout at `root` the files as turn `turn` left
    /// them, where they differ from now. Returns the paths it touched.
    async fn write_turn_files(&mut self, thread: ThreadId, turn: usize, root: &Path) -> Result<Vec<String>, CoreError> {
        let family_id = self.threads[&thread].family;
        let point = self.turn_point(thread, turn).expect("the turn was found");
        let family = &self.families[&family_id];
        let now = family.graph().version_vector(&family.thread_head(thread));
        let then = family.graph().version_vector(&point);
        let mut touched = Vec::new();
        for change in self.changes(family_id, &now, &then) {
            let current = change.old_path.as_deref().map(|path| disk::read(root, path)).transpose()?.flatten();
            if let Some(new_path) = &change.new_path {
                let view = change.inode.and_then(|inode| self.families[&family_id].file_at(inode, &then));
                let bytes = self.worktree_bytes(root, new_path, view).await?;
                let executable = current.as_ref().is_some_and(|file| file.executable);
                disk::replace(root, new_path, &bytes, executable, |_| true)?;
                touched.push(new_path.clone());
            }
            if let Some(old_path) = change.old_path.filter(|old| Some(old) != change.new_path.as_ref()) {
                disk::remove(root, &old_path)?;
                touched.push(old_path);
            }
        }
        Ok(touched)
    }

    /// A file's content as the checkout at `root` holds it on disk.
    async fn worktree_bytes(&self, root: &Path, path: &str, view: Option<FileView>) -> Result<Vec<u8>, CoreError> {
        let checkout = self.checkouts.values().find(|checkout| checkout.record.path == root).expect("checked out above");
        let repo = &checkout.repo;
        Ok(match view {
            Some(FileView::Text(text)) => repo.filters()?.to_worktree(path, text.as_bytes())?,
            Some(FileView::GitText(oid)) => repo.filters()?.to_worktree(path, &repo.blob(oid)?)?,
            Some(FileView::Blob(hash)) => self.blob(checkout.family, hash).await?,
            None => Vec::new(),
        })
    }
}

/// The thread's last conversation delta in `log`, which a revert keeps.
fn last_conversation_before(log: &[silt_log::Delta], thread: ThreadId) -> Option<DeltaId> {
    log.iter().rev().find(|delta| delta.thread == thread && matches!(delta.op, Op::Conv(_))).map(|delta| delta.span.last())
}
