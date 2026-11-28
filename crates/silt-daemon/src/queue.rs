//! The prompts waiting for a thread's agent (ADR 0022), kept in the store
//! so they outlive the agent and the daemon. The front one is sent when a
//! turn ends. Their files wait beside the store, each queued prompt's in a
//! folder of its own, and go with it as uploads do (ADR 0121).

use std::path::{Path, PathBuf};

use silt_store::{QueuedFile, QueuedPrompt};
use silt_types::ThreadId;

use crate::acp::Given;
use crate::core::{Core, CoreError, now_ms};
use crate::media::Uploaded;
use crate::protocol::{BusEvent, Queued};

fn queued_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("queued")
}

/// Where a queued prompt's files wait, each by its place among them.
fn files_dir(data_dir: &Path, id: i64) -> PathBuf {
    queued_dir(data_dir).join(id.to_string())
}

/// Keeps `files` for the queued prompt `id`, in the order its row names them.
async fn keep_files(data_dir: &Path, id: i64, files: Given) -> std::io::Result<()> {
    let dir = files_dir(data_dir, id);
    tokio::fs::create_dir_all(&dir).await?;
    let mut place = 0..;
    for (attachment, at) in files.sent.iter().zip(&mut place) {
        tokio::fs::write(dir.join(at.to_string()), &attachment.data).await?;
    }
    for (upload, at) in files.uploaded.iter().zip(&mut place) {
        crate::media::move_file(&upload.path, &dir.join(at.to_string())).await?;
    }
    Ok(())
}

fn named(files: &Given) -> Vec<QueuedFile> {
    let sent = files.sent.iter().map(|file| QueuedFile { name: file.name.clone(), mime: file.mime.clone() });
    let uploaded = files.uploaded.iter().map(|file| QueuedFile { name: file.name.clone(), mime: file.mime.clone() });
    sent.chain(uploaded).collect()
}

/// A queued prompt's files, to send as files uploaded for it; one that's
/// gone is left out.
fn given(data_dir: &Path, queued: &QueuedPrompt) -> Given {
    let dir = files_dir(data_dir, queued.id);
    let uploaded = (0..)
        .zip(&queued.files)
        .map(|(at, file)| Uploaded { name: file.name.clone(), mime: file.mime.clone(), path: dir.join(at.to_string()) })
        .filter(|upload| upload.path.is_file())
        .collect();
    Given { sent: Vec::new(), uploaded }
}

/// Drops a queued prompt's files, once sent or removed.
fn drop_files(data_dir: &Path, id: i64) {
    let _ = std::fs::remove_dir_all(files_dir(data_dir, id));
}

impl Core {
    pub(crate) async fn queue(&self, thread: ThreadId) -> Result<Vec<Queued>, CoreError> {
        Ok(self
            .store
            .queue(thread)
            .await?
            .into_iter()
            .map(|queued| Queued {
                id: queued.id,
                author: queued.author,
                text: queued.text,
                created_ms: queued.created_ms,
                files: queued.files.into_iter().map(|file| file.name).collect(),
            })
            .collect())
    }

    /// Adds `text` by this machine's user to the end of the queue, or the
    /// front.
    pub(crate) async fn enqueue(&mut self, thread: ThreadId, text: &str, first: bool) -> Result<i64, CoreError> {
        self.enqueue_with(thread, text, Given::default(), first).await
    }

    /// [`Self::enqueue`] with the files the prompt goes with.
    pub(crate) async fn enqueue_with(
        &mut self,
        thread: ThreadId,
        text: &str,
        files: Given,
        first: bool,
    ) -> Result<i64, CoreError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(CoreError::Invalid("there is nothing to queue".into()));
        }
        let names = named(&files);
        let id = self.store.enqueue(thread, &self.own_user(thread), text, &names, now_ms(), first).await?;
        if !names.is_empty()
            && let Err(error) = keep_files(&self.data_dir, id, files).await
        {
            drop_files(&self.data_dir, id);
            self.store.take_queued(thread, id).await?;
            return Err(error.into());
        }
        self.queue_changed(thread);
        // An idle agent takes it at once.
        if self.acp.contains_key(&thread) && self.agent_state(thread) == Some(silt_proto::AgentState::Idle) {
            Box::pin(self.acp_prompt_next(thread)).await?;
        }
        Ok(id)
    }

    pub(crate) async fn queue_edit(&mut self, thread: ThreadId, id: i64, text: &str) -> Result<&'static str, CoreError> {
        if !self.store.edit_queued(thread, id, text.trim()).await? {
            return Err(gone());
        }
        self.queue_changed(thread);
        Ok("edited")
    }

    pub(crate) async fn queue_move(&mut self, thread: ThreadId, id: i64, to: usize) -> Result<&'static str, CoreError> {
        if !self.store.move_queued(thread, id, to).await? {
            return Err(gone());
        }
        self.queue_changed(thread);
        Ok("moved")
    }

    pub(crate) async fn queue_remove(&mut self, thread: ThreadId, id: i64) -> Result<&'static str, CoreError> {
        self.store.take_queued(thread, id).await?.ok_or_else(gone)?;
        drop_files(&self.data_dir, id);
        self.queue_changed(thread);
        Ok("removed")
    }

    /// Sends a queued prompt now, interrupting the agent if it works.
    pub(crate) async fn queue_send(&mut self, thread: ThreadId, id: i64) -> Result<&'static str, CoreError> {
        self.running(thread)?;
        let queued = self.store.take_queued(thread, id).await?.ok_or_else(gone)?;
        self.queue_changed(thread);
        let files = given(&self.data_dir, &queued);
        let sent = self.acp_steer(thread, &queued.text, files).await;
        drop_files(&self.data_dir, id);
        sent
    }

    /// Sends the queue's front to an idle agent.
    async fn acp_prompt_next(&mut self, thread: ThreadId) -> Result<(), CoreError> {
        let Some(next) = self.store.take_next(thread).await? else {
            return Ok(());
        };
        self.queue_changed(thread);
        let files = given(&self.data_dir, &next);
        let sent = self.acp_prompt_with(&thread.to_string(), &next.text, false, files).await;
        drop_files(&self.data_dir, next.id);
        sent.map(|_| ())
    }

    /// Takes the queue's front, with its files, to send now.
    pub(crate) async fn take_next_queued(&mut self, thread: ThreadId) -> Result<Option<(QueuedPrompt, Given)>, CoreError> {
        let Some(next) = self.store.take_next(thread).await? else {
            return Ok(None);
        };
        let files = given(&self.data_dir, &next);
        Ok(Some((next, files)))
    }

    /// Drops the files of prompts no longer queued, as a daemon stopped
    /// between taking a prompt and sending it leaves them.
    pub(crate) async fn drop_unqueued_files(&self) {
        let Ok(entries) = std::fs::read_dir(queued_dir(&self.data_dir)) else {
            return;
        };
        let queued: std::collections::HashSet<String> = match self.store.queued_ids().await {
            Ok(ids) => ids.into_iter().map(|id| id.to_string()).collect(),
            Err(error) => return tracing::warn!(%error, "could not read the queue to drop files it no longer has"),
        };
        for entry in entries.flatten() {
            if !queued.contains(&*entry.file_name().to_string_lossy()) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }

    /// Drops the files of the queued prompt `id`, once it's sent.
    pub(crate) fn sent_queued(&self, id: i64) {
        drop_files(&self.data_dir, id);
    }

    fn queue_changed(&self, thread: ThreadId) {
        self.announce(BusEvent::Queue { thread: thread.to_string() });
    }
}

fn gone() -> CoreError {
    CoreError::Invalid("that prompt was sent or removed already".into())
}
