//! This person's state of each thread (ADR 0024): kept in the store, sent
//! to the server, and merged with what their other devices sent it.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use silt_proto::{Stamped, ThreadState};
use silt_types::ThreadId;
use tracing::warn;

use crate::core::{Core, CoreError, Event, now_ms};
use crate::protocol::{BusEvent, Mine, StateChange};

/// How often the server is asked for what other devices changed.
const STATES_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Default)]
pub(crate) struct Personal {
    pub(crate) states: HashMap<ThreadId, ThreadState>,
    /// Changed here, and not yet taken by the server.
    pending: HashSet<ThreadId>,
    last_sync: Option<Instant>,
    syncing: bool,
}

impl Personal {
    pub(crate) fn mine(&self, thread: ThreadId) -> Mine {
        mine(self.states.get(&thread))
    }
}

/// The state as the web UI reads it.
fn mine(state: Option<&ThreadState>) -> Mine {
    let value = |field: Option<Stamped<bool>>| field.is_some_and(|field| field.value);
    let stamped = state.and_then(|state| state.snooze);
    let snooze = stamped.map(|snooze| snooze.value);
    Mine {
        pinned: value(state.and_then(|state| state.pinned)),
        snoozed_until: snooze.and_then(|snooze| snooze.until),
        snoozed_at: stamped.filter(|snooze| snooze.value.until.is_some()).map(|snooze| snooze.at_ms),
        wake_on_activity: snooze.is_some_and(|snooze| snooze.wake_on_activity),
        settled: value(state.and_then(|state| state.settled)),
        seen_ms: state.and_then(|state| state.seen_ms).map(|seen| seen.value),
    }
}

/// The change the web UI asks for, stamped now.
fn stamped(thread: ThreadId, change: &StateChange, now: i64) -> ThreadState {
    let at = |value| Stamped { value, at_ms: now };
    ThreadState {
        thread,
        pinned: change.pinned.map(at),
        snooze: change.snooze.map(|snooze| Stamped { value: snooze, at_ms: now }),
        settled: change.settled.map(at),
        seen_ms: change.seen.then_some(Stamped { value: now, at_ms: now }),
    }
}

impl Core {
    pub(crate) async fn load_thread_states(&mut self) -> Result<(), CoreError> {
        for (thread, json, pending) in self.store.thread_states().await? {
            let Ok(state) = serde_json::from_str::<ThreadState>(&json) else {
                continue;
            };
            if pending {
                self.personal.pending.insert(thread);
            }
            self.personal.states.insert(thread, state);
        }
        Ok(())
    }

    /// Changes this person's state of the thread, here at once and on the
    /// server at the next sync.
    pub(crate) async fn change_thread_state(&mut self, selector: &str, change: StateChange) -> Result<Mine, CoreError> {
        let thread = self.find_thread(selector)?;
        let change = stamped(thread, &change, now_ms());
        let state = self.personal.states.entry(thread).or_insert_with(|| ThreadState::new(thread));
        state.merge(&change);
        let state = state.clone();
        self.personal.pending.insert(thread);
        self.keep_thread_state(&state, true).await;
        self.announce(BusEvent::Threads);
        // Sent now rather than at the next interval.
        self.personal.last_sync = None;
        self.drive_thread_states();
        Ok(mine(Some(&state)))
    }

    async fn keep_thread_state(&self, state: &ThreadState, pending: bool) {
        let Ok(json) = serde_json::to_string(state) else {
            return;
        };
        if let Err(error) = self.store.put_thread_state(state.thread, &json, pending).await {
            warn!(%error, "could not keep the thread's state");
        }
    }

    /// Sends what changed here and fetches what changed elsewhere, when due
    /// and not already under way.
    pub(crate) fn drive_thread_states(&mut self) {
        let Some(remote) = self.remote.clone() else {
            return;
        };
        let due = self.personal.last_sync.is_none_or(|last| last.elapsed() >= STATES_INTERVAL);
        if self.personal.syncing || !due {
            return;
        }
        self.personal.syncing = true;
        self.personal.last_sync = Some(Instant::now());
        let sending: Vec<ThreadState> =
            self.personal.pending.iter().filter_map(|thread| self.personal.states.get(thread).cloned()).collect();
        let events = self.events.clone();
        tokio::spawn(async move {
            let mut sent = Vec::new();
            for state in sending {
                match remote.change_thread_state(&state).await {
                    Ok(merged) => sent.push((state, merged)),
                    Err(error) => warn!(%error, "could not send a thread's state to the server"),
                }
            }
            let fetched = remote.thread_states().await.map_err(|error| error.to_string());
            let _ = events.send(Event::ThreadStates { sent, fetched });
        });
    }

    /// Takes in a sync's answer: what the server took, and every state it
    /// holds, merged into this machine's.
    pub(crate) async fn thread_states_synced(
        &mut self,
        sent: Vec<(ThreadState, ThreadState)>,
        fetched: Result<Vec<ThreadState>, String>,
    ) {
        self.personal.syncing = false;
        let mut kept = HashSet::new();
        let mut changed = false;
        for (state, merged) in &sent {
            // A change made since this one was sent stays pending.
            if self.personal.states.get(&state.thread) == Some(state) {
                self.personal.pending.remove(&state.thread);
                kept.insert(state.thread);
            }
            if self.take_thread_state(merged) {
                kept.insert(merged.thread);
                changed = true;
            }
        }
        match fetched {
            Ok(states) => {
                for state in &states {
                    if self.take_thread_state(state) {
                        kept.insert(state.thread);
                        changed = true;
                    }
                }
            }
            Err(error) => warn!(%error, "could not fetch thread states from the server"),
        }
        for thread in kept {
            if let Some(state) = self.personal.states.get(&thread) {
                self.keep_thread_state(state, self.personal.pending.contains(&thread)).await;
            }
        }
        if changed {
            self.announce(BusEvent::Threads);
        }
    }

    fn take_thread_state(&mut self, other: &ThreadState) -> bool {
        self.personal.states.entry(other.thread).or_insert_with(|| ThreadState::new(other.thread)).merge(other)
    }
}

#[cfg(test)]
mod tests {
    use silt_proto::Snooze;

    use super::*;

    #[test]
    fn a_change_from_the_web_ui_sets_only_what_it_names() {
        let thread = ThreadId(3);
        let change = StateChange { pinned: Some(true), snooze: None, settled: None, seen: true };
        let state = stamped(thread, &change, 50);
        assert_eq!(state.pinned, Some(Stamped { value: true, at_ms: 50 }));
        assert_eq!((state.snooze, state.settled), (None, None));
        assert_eq!(state.seen_ms, Some(Stamped { value: 50, at_ms: 50 }));

        let snoozed = StateChange { snooze: Some(Snooze { until: Some(900), wake_on_activity: true }), ..StateChange::default() };
        let mut both = state.clone();
        both.merge(&stamped(thread, &snoozed, 60));
        let shown = mine(Some(&both));
        assert!(shown.pinned && shown.wake_on_activity && !shown.settled);
        assert_eq!((shown.snoozed_until, shown.snoozed_at, shown.seen_ms), (Some(900), Some(60), Some(50)));
        assert_eq!(mine(None), Mine::default());
    }
}
