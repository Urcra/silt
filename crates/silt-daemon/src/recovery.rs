//! Recovery forks (ADR 0009). When this machine's history of a family
//! differs from the server's, as after restoring Silt's data from an old
//! backup, the server's history wins. The local work moves into a
//! recovery thread forked from the server's version, so nothing is lost.

use std::collections::{BTreeMap, HashSet};

use silt_checkout::Bridge;
use silt_log::{ConvOp, Delta, Op};
use silt_sync::ClientSync;
use silt_types::{ActorId, DeltaId, FamilyId, ThreadId};
use tracing::warn;

use crate::core::{Core, CoreError};

impl Core {
    pub(crate) async fn recover_diverged(&mut self, family_id: FamilyId) -> Result<(), CoreError> {
        let Some(family) = self.families.get(&family_id) else {
            return Ok(());
        };
        let repo = self.family_repo(family_id)?;
        // The threads with local work: those checked out or recording here,
        // with their own conversation as this machine has it.
        let threads: HashSet<ThreadId> = self
            .checkouts
            .values()
            .filter(|checkout| checkout.family == family_id)
            .map(|checkout| checkout.record.thread)
            .chain(
                self.sessions
                    .iter()
                    .filter(|session| session.family == family_id)
                    .map(|session| session.record.thread),
            )
            .collect();
        let conversations: BTreeMap<ThreadId, Vec<Delta>> = threads
            .iter()
            .map(|thread| (*thread, family.conversation(*thread).cloned().collect()))
            .collect();

        self.families.remove(&family_id);
        self.raw.remove(&family_id);
        self.store.clear_deltas(family_id).await?;
        self.syncs.insert(family_id, ClientSync::default());
        self.catch_up(family_id).await?;
        self.project_family(family_id, &repo)?;

        for (thread, conversation) in conversations {
            let recovery = self.fork_thread(thread).await?;
            self.register_thread_now(recovery).await?;
            let server_ids: HashSet<DeltaId> = self.families[&family_id].log().iter().map(|delta| delta.span.first()).collect();
            for delta in conversation.iter().filter(|delta| !server_ids.contains(&delta.span.first())) {
                if let Op::Conv(op) = &delta.op
                    && !matches!(op, ConvOp::SessionStarted { .. } | ConvOp::SessionDir { .. })
                {
                    self.append_conv(family_id, recovery, delta.author.clone(), op.clone()).await?;
                }
            }
            self.move_work(thread, recovery).await?;
            let note = format!(
                "thread {thread} differed from the server's copy (was Silt's data here restored from a backup?); \
                 the server's version is kept and the work from this machine continues in recovery thread {recovery}"
            );
            warn!("{note}");
            self.notices.entry(recovery).or_default().push(format!("Silt: {note}."));
            self.recoveries.push(note);
        }
        Ok(())
    }

    /// Moves `from`'s checkouts and sessions here to `to`, with fresh actors,
    /// and records what is on disk as `to`'s changes.
    async fn move_work(&mut self, from: ThreadId, to: ThreadId) -> Result<(), CoreError> {
        let ids: Vec<i64> = self
            .checkouts
            .iter()
            .filter(|(_, checkout)| checkout.record.thread == from)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            let actor = ActorId(uuid::Uuid::new_v4().as_u128());
            self.store.move_checkout(id, to).await?;
            self.store.rebind_checkout(id, to, actor).await?;
            let checkout = self.checkouts.get_mut(&id).expect("listed above");
            checkout.record.thread = to;
            checkout.record.actor = actor;
            checkout.bridge = Bridge::new(actor, to, BTreeMap::new());
            for session in self.sessions.iter_mut().filter(|session| session.record.checkout_id == id) {
                session.record.thread = to;
            }
            self.rescan(id).await?;
        }
        for session in self.sessions.iter_mut().filter(|session| session.record.thread == from) {
            session.record.thread = to;
        }
        Ok(())
    }
}
