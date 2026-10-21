//! Taking in what other writers stored (ADR 0018). A hook that captured
//! without the daemon wrote deltas, disk state and transcript offsets that
//! the daemon holds older copies of. Writers take turns through the capture
//! lock, so the daemon only has to look at the start of each turn, and only
//! when the store changed since the end of its last one.

use std::collections::HashSet;

use silt_checkout::Bridge;
use silt_log::{Applied, Delta};
use silt_sync::ClientSync;
use silt_types::{DeltaId, FamilyId, ThreadId};
use tracing::warn;

use crate::core::{Core, CoreError, RepoBases, stored_entries};
use crate::load;
use crate::sessions::read_recorded;

impl Core {
    /// Takes in what others stored since this daemon's last turn.
    pub(crate) async fn catch_up_with_store(&mut self) -> Result<(), CoreError> {
        if !self.store.changed().await? {
            return Ok(());
        }
        for record in self.store.threads().await? {
            self.threads.entry(record.id).or_insert(record);
        }
        let mut grown = Vec::new();
        for (family_id, position) in self.store.positions().await? {
            let known = self.positions.get(&family_id).copied();
            if known.is_some_and(|known| known >= position) || !self.follows(family_id) {
                continue;
            }
            let deltas = self.store.deltas_after(family_id, known.unwrap_or(-1)).await?;
            let fresh = self.take_in(family_id, known.is_none(), deltas).await?;
            self.positions.insert(family_id, position);
            grown.push((family_id, fresh));
        }
        // A checkout held already may have been given to another thread.
        for record in self.store.checkouts().await? {
            let held = self.checkouts.get(&record.id).map(|checkout| &checkout.record);
            if held != Some(&record) && record.path.exists() && self.follows_thread(record.thread) {
                let entries = stored_entries(&self.store, record.id).await?;
                self.start_checkout(record, entries)?;
            }
        }
        let known: HashSet<(String, String)> =
            self.sessions.iter().map(|session| (session.record.agent.clone(), session.record.session_id.clone())).collect();
        let new: Vec<_> = self
            .store
            .sessions()
            .await?
            .into_iter()
            .filter(|record| {
                !known.contains(&(record.agent.clone(), record.session_id.clone())) && self.follows_thread(record.thread)
            })
            .collect();
        let taken = self.sessions.len();
        self.take_sessions(new).await?;
        for (family_id, deltas) in grown {
            self.refresh(family_id, &deltas, taken).await?;
        }
        let stored = self.store.secrets().await?;
        let learned: Vec<String> = stored.into_iter().filter(|secret| self.known_secrets.insert(secret.clone())).collect();
        if !learned.is_empty() {
            self.redactor = self.redactor.clone().with(learned);
        }
        Ok(())
    }

    /// Whether this core takes in what others store of the family: the
    /// daemon all of it, any other process only the families it holds.
    fn follows(&self, family: FamilyId) -> bool {
        self.resident || self.families.contains_key(&family) || self.raw.contains_key(&family)
    }

    fn follows_thread(&self, thread: ThreadId) -> bool {
        self.threads.get(&thread).is_some_and(|record| self.follows(record.family))
    }

    /// Remembers what the store holds at the end of this daemon's turn, so
    /// what it wrote itself isn't taken in again.
    pub(crate) async fn end_turn(&mut self) -> Result<(), CoreError> {
        if self.store.changed().await? {
            self.positions = self.store.positions().await?;
        }
        Ok(())
    }

    /// Adds `deltas`, stored by another writer, to the family here, or loads
    /// a family `new` to this daemon. Returns those it didn't hold yet: a
    /// daemon loading without its turn may have read some already.
    async fn take_in(&mut self, family_id: FamilyId, new: bool, deltas: Vec<Delta>) -> Result<Vec<Delta>, CoreError> {
        if new || !(self.families.contains_key(&family_id) || self.raw.contains_key(&family_id)) {
            let repo = self.family_repo(family_id).ok();
            let loaded = load::family(&self.store, family_id, repo.as_ref()).await?;
            self.take_loaded(family_id, loaded);
            let state = self.store.sync_state(family_id).await?;
            self.syncs
                .entry(family_id)
                .or_insert_with(|| ClientSync::new(state.generation, state.pushed, state.pulled, state.last_pulled, state.view));
            return Ok(Vec::new());
        }
        if let Some(raw) = self.raw.get_mut(&family_id) {
            let held: HashSet<DeltaId> = raw.iter().map(|delta| delta.span.first()).collect();
            let fresh: Vec<Delta> = deltas.into_iter().filter(|delta| !held.contains(&delta.span.first())).collect();
            raw.extend_from_slice(&fresh);
            return Ok(fresh);
        }
        let repo = self.family_repo(family_id)?;
        let family = self.families.get_mut(&family_id).expect("checked above");
        let mut fresh = Vec::new();
        for delta in deltas {
            match family.apply(delta.clone(), &mut RepoBases(&repo)) {
                Ok(Applied::New) => fresh.push(delta),
                Ok(Applied::AlreadyKnown) => {}
                Err(error) => warn!(%error, "skipping a delta another writer stored that doesn't apply here"),
            }
        }
        Ok(fresh)
    }

    /// Brings what this daemon keeps beside a family in line with the store
    /// after others wrote `deltas` to it: its checkouts' records of the
    /// disk, its sessions' read positions and readers, its sync progress.
    /// Sessions from `taken` on were just loaded with all of it.
    async fn refresh(&mut self, family_id: FamilyId, deltas: &[Delta], taken: usize) -> Result<(), CoreError> {
        let checkouts: Vec<_> =
            self.checkouts.iter().filter(|(_, checkout)| checkout.family == family_id).map(|(id, _)| *id).collect();
        for id in checkouts {
            let entries = stored_entries(&self.store, id).await?;
            let checkout = self.checkouts.get_mut(&id).expect("listed above");
            checkout.bridge = Bridge::new(checkout.record.actor, checkout.record.thread, entries);
        }
        let sessions: Vec<usize> = (0..taken).filter(|index| self.sessions[*index].family == family_id).collect();
        for index in sessions {
            let offsets = self.stored_offsets(&self.sessions[index].record).await?;
            let session = &mut self.sessions[index];
            session.transcript = offsets.get("main").map(|offset| offset.path.clone()).or(session.transcript.take());
            session.offsets = offsets;
        }
        read_recorded(&mut self.sessions[..taken], deltas);
        let state = self.store.sync_state(family_id).await?;
        if let Some(sync) = self.syncs.get_mut(&family_id) {
            sync.caught_up(&ClientSync::new(state.generation, state.pushed, state.pulled, state.last_pulled, state.view));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use silt_log::{Author, ConvOp, Family, Op};
    use silt_store::{Store, ThreadRecord};
    use silt_types::{ActorId, ThreadId};
    use tokio::sync::mpsc;

    use super::*;

    const THREAD: ThreadId = ThreadId(7);
    const FAMILY: FamilyId = FamilyId(9);

    /// What a hook writing as actor 2 would record next, having read `log`.
    fn titled(log: &[Delta], title: &str) -> Delta {
        let mut family = Family::new();
        for delta in log {
            family.apply(delta.clone(), &mut std::collections::HashMap::new()).expect("applies");
        }
        let head = family.thread_head(THREAD);
        family.delta(ActorId(2), THREAD, Author::User, 0, head, Op::Conv(ConvOp::Title(title.into())))
    }

    /// A store with one thread in a clone; the directories go with it.
    async fn store_with_thread() -> (tempfile::TempDir, tempfile::TempDir, Store) {
        let (data, repo) = (tempfile::tempdir().expect("temp dir"), tempfile::tempdir().expect("temp dir"));
        assert!(Command::new("git").args(["init", "-q"]).current_dir(repo.path()).status().expect("git").success());
        let store = Store::open(data.path()).await.expect("opens");
        let thread = ThreadRecord {
            id: THREAD,
            family: FAMILY,
            repo_root: repo.path().to_owned(),
            repo: "example.com/repo".into(),
            created_ms: 1,
            owner: String::new(),
        };
        store.create_thread(&thread).await.expect("thread");
        (data, repo, store)
    }

    #[tokio::test]
    async fn what_a_load_read_already_is_not_taken_in_again() {
        let (data, _repo, store) = store_with_thread().await;
        let early = titled(&[], "stored while the daemon loaded");
        store.append_deltas(FAMILY, std::slice::from_ref(&early)).await.expect("appends");
        let (events, _inbox) = mpsc::unbounded_channel();
        let mut core = Core::load(store, ActorId(1), data.path(), events).await.expect("loads");
        // As if the load noted the store's positions before that was stored.
        core.positions.insert(FAMILY, -1);
        core.catch_up_with_store().await.expect("takes in");
        assert_eq!(core.families[&FAMILY].log(), [early]);
    }

    #[tokio::test]
    async fn what_another_writer_stored_is_taken_in_once() {
        let (data, _repo, store) = store_with_thread().await;
        let (events, _inbox) = mpsc::unbounded_channel();
        let mut core = Core::load(store, ActorId(1), data.path(), events).await.expect("loads");
        core.catch_up_with_store().await.expect("nothing to take in");

        let hook = Store::open(data.path()).await.expect("another process opens it");
        let stored = titled(&[], "from the hook");
        hook.append_deltas(FAMILY, std::slice::from_ref(&stored)).await.expect("appends");
        core.catch_up_with_store().await.expect("takes in");
        assert_eq!(core.families[&FAMILY].log(), [stored]);
        assert_eq!(core.families[&FAMILY].thread(THREAD).and_then(|state| state.title()), Some("from the hook"));

        core.append_conv(FAMILY, THREAD, Author::User, ConvOp::Title("from the daemon".into())).await.expect("appends");
        core.end_turn().await.expect("notes");
        core.catch_up_with_store().await.expect("nothing new");
        assert_eq!(core.families[&FAMILY].log().len(), 2, "its own write isn't taken in again");
        let later = titled(core.families[&FAMILY].log(), "later");
        hook.append_deltas(FAMILY, std::slice::from_ref(&later)).await.expect("appends");
        core.catch_up_with_store().await.expect("takes in");
        assert_eq!(core.families[&FAMILY].log().len(), 3);
    }
}
