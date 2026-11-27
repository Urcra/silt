//! The prompts waiting for a thread's agent (ADR 0022), kept in the store
//! so they outlive the agent and the daemon. The front one is sent when a
//! turn ends.

use silt_types::ThreadId;

use crate::core::{Core, CoreError, now_ms};
use crate::protocol::{BusEvent, Queued};

impl Core {
    pub(crate) async fn queue(&self, thread: ThreadId) -> Result<Vec<Queued>, CoreError> {
        Ok(self
            .store
            .queue(thread)
            .await?
            .into_iter()
            .map(|queued| Queued { id: queued.id, author: queued.author, text: queued.text, created_ms: queued.created_ms })
            .collect())
    }

    /// Adds `text` by this machine's user to the end of the queue, or the
    /// front.
    pub(crate) async fn enqueue(&mut self, thread: ThreadId, text: &str, first: bool) -> Result<i64, CoreError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(CoreError::Invalid("there is nothing to queue".into()));
        }
        let id = self.store.enqueue(thread, &self.own_user(thread), text, now_ms(), first).await?;
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
        self.queue_changed(thread);
        Ok("removed")
    }

    /// Sends a queued prompt now, interrupting the agent if it works.
    pub(crate) async fn queue_send(&mut self, thread: ThreadId, id: i64) -> Result<&'static str, CoreError> {
        self.running(thread)?;
        let queued = self.store.take_queued(thread, id).await?.ok_or_else(gone)?;
        self.queue_changed(thread);
        self.acp_steer(thread, &queued.text, crate::acp::Given::default()).await
    }

    /// Sends the queue's front to an idle agent.
    async fn acp_prompt_next(&mut self, thread: ThreadId) -> Result<(), CoreError> {
        let Some(next) = self.store.take_next(thread).await? else {
            return Ok(());
        };
        self.queue_changed(thread);
        self.acp_prompt(&thread.to_string(), &next.text, false).await.map(|_| ())
    }

    fn queue_changed(&self, thread: ThreadId) {
        self.announce(BusEvent::Queue { thread: thread.to_string() });
    }
}

fn gone() -> CoreError {
    CoreError::Invalid("that prompt was sent or removed already".into())
}
