//! This person's state of each thread and seen marks on merge request
//! files (ADR 0024): kept in the store, sent to the server, and merged with
//! what their other devices sent it.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use silt_proto::{MrSeen, SeenMark, ThreadState};
use silt_types::ThreadId;
use tracing::warn;

use crate::core::{Core, CoreError, Event, now_ms};
use crate::protocol::{BusEvent, Mine, StateChange};
use crate::remote::Remote;

/// How often the server is asked for what other devices changed.
const STATES_INTERVAL: Duration = Duration::from_secs(10);

/// A merge request, by its `DeltaId`, and one of its files.
type SeenKey = (String, String);

#[derive(Default)]
pub(crate) struct Personal {
    pub(crate) states: HashMap<ThreadId, ThreadState>,
    /// Changed here, and not yet taken by the server.
    pending: HashSet<ThreadId>,
    seen: HashMap<SeenKey, SeenMark>,
    seen_pending: HashSet<SeenKey>,
    last_sync: Option<Instant>,
    syncing: bool,
}

impl Personal {
    pub(crate) fn mine(&self, thread: ThreadId) -> Mine {
        Mine::of(self.states.get(&thread))
    }

    /// Whether the file was looked at as it is now, its `hash`.
    pub(crate) fn has_seen(&self, mr: &str, path: &str, hash: &str) -> bool {
        self.seen.get(&(mr.to_owned(), path.to_owned())).is_some_and(|mark| mark.seen && mark.hash == hash)
    }

    fn take_mark(&mut self, mr: &str, mark: SeenMark) -> bool {
        let key = (mr.to_owned(), mark.path.clone());
        let newer = self.seen.get(&key).is_none_or(|known| mark.newer_than(known));
        if newer {
            self.seen.insert(key, mark);
        }
        newer
    }
}

/// What a sync brought back (ADR 0024).
/// What one server is sent in a sync of this person's state: its own
/// threads' states and its own requests' marks.
struct Round {
    remote: Remote,
    sending: Vec<ThreadState>,
    marking: Vec<(String, SeenMark)>,
}

pub(crate) struct Synced {
    /// Thread states the server took, each with how it then stood.
    sent: Vec<(ThreadState, ThreadState)>,
    fetched: Result<Vec<ThreadState>, String>,
    /// Seen marks the server took.
    marks_sent: Vec<(String, SeenMark)>,
    marks: Result<Vec<MrSeen>, String>,
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
        for record in self.store.seen_marks().await? {
            let key = (record.mr, record.path);
            if record.pending {
                self.personal.seen_pending.insert(key.clone());
            }
            let mark = SeenMark { path: key.1.clone(), hash: record.hash, seen: record.seen, at_ms: record.at_ms };
            self.personal.seen.insert(key, mark);
        }
        Ok(())
    }

    /// Marks a file of a merge request seen, or not, as it is now; here at
    /// once and on the server at the next sync.
    pub(crate) async fn mark_seen(&mut self, selector: &str, path: &str, seen: bool) -> Result<&'static str, CoreError> {
        let (family_id, request) = self.find_mr(selector)?;
        let hash = self
            .request_files(family_id, &request)
            .into_iter()
            .find(|(file, _)| file == path)
            .map(|(_, hash)| hash)
            .ok_or_else(|| CoreError::Invalid(format!("{path} isn't a file this merge request changes")))?;
        let mr = request.id.to_string();
        let mark = SeenMark { path: path.to_owned(), hash, seen, at_ms: now_ms() };
        self.personal.take_mark(&mr, mark.clone());
        self.personal.seen_pending.insert((mr.clone(), path.to_owned()));
        self.keep_mark(&mr, &mark, true).await;
        self.announce(BusEvent::Mrs);
        self.personal.last_sync = None;
        self.drive_thread_states();
        Ok("marked")
    }

    async fn keep_mark(&self, mr: &str, mark: &SeenMark, pending: bool) {
        let record = silt_store::SeenRecord {
            mr: mr.to_owned(),
            path: mark.path.clone(),
            hash: mark.hash.clone(),
            seen: mark.seen,
            at_ms: mark.at_ms,
            pending,
        };
        if let Err(error) = self.store.put_seen_mark(&record).await {
            warn!(%error, "could not keep a seen mark");
        }
    }

    /// Changes this person's state of the thread, here at once and on the
    /// server at the next sync.
    pub(crate) async fn change_thread_state(&mut self, selector: &str, change: StateChange) -> Result<Mine, CoreError> {
        let thread = self.find_thread(selector)?;
        let change = change.stamped(thread, now_ms());
        let state = self.personal.states.entry(thread).or_insert_with(|| ThreadState::new(thread));
        state.merge(&change);
        let state = state.clone();
        self.personal.pending.insert(thread);
        self.keep_thread_state(&state, true).await;
        self.announce(BusEvent::Threads);
        // Sent now rather than at the next interval.
        self.personal.last_sync = None;
        self.drive_thread_states();
        Ok(Mine::of(Some(&state)))
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
    /// and not already under way: each server gets only what is of its own
    /// threads (ADR 0032), and what all of them hold is merged here.
    pub(crate) fn drive_thread_states(&mut self) {
        if self.remotes.is_empty() {
            return;
        }
        let due = self.personal.last_sync.is_none_or(|last| last.elapsed() >= STATES_INTERVAL);
        if self.personal.syncing || !due {
            return;
        }
        self.personal.syncing = true;
        self.personal.last_sync = Some(Instant::now());
        let rounds: Vec<Round> = self
            .remotes
            .iter()
            .map(|remote| {
                let server = remote.config.url.as_str();
                let sending = self
                    .personal
                    .pending
                    .iter()
                    .filter(|thread| self.homed_at(**thread, server))
                    .filter_map(|thread| self.personal.states.get(thread).cloned())
                    .collect();
                let marking = self
                    .personal
                    .seen_pending
                    .iter()
                    .filter(|key| {
                        self.find_mr(&key.0)
                            .is_ok_and(|(family, _)| self.remote_of(family).is_some_and(|at| at.config.url == server))
                    })
                    .filter_map(|key| self.personal.seen.get(key).map(|mark| (key.0.clone(), mark.clone())))
                    .collect();
                Round { remote: remote.clone(), sending, marking }
            })
            .collect();
        let events = self.events.clone();
        tokio::spawn(async move {
            let mut synced = Synced { sent: Vec::new(), fetched: Ok(Vec::new()), marks_sent: Vec::new(), marks: Ok(Vec::new()) };
            for Round { remote, sending, marking } in rounds {
                let server = remote.config.url.clone();
                for state in sending {
                    match remote.change_thread_state(&state).await {
                        Ok(merged) => synced.sent.push((state, merged)),
                        Err(error) => warn!(%error, server, "could not send a thread's state to the server"),
                    }
                }
                for (mr, mark) in marking {
                    match remote.mark_seen(&mr, &mark).await {
                        Ok(()) => synced.marks_sent.push((mr, mark)),
                        Err(error) => warn!(%error, server, "could not send a seen mark to the server"),
                    }
                }
                match (remote.thread_states().await, &mut synced.fetched) {
                    (Ok(states), Ok(all)) => all.extend(states),
                    (Err(error), _) => warn!(%error, server, "could not fetch thread states from the server"),
                    (Ok(_), Err(_)) => {}
                }
                match (remote.seen().await, &mut synced.marks) {
                    (Ok(marks), Ok(all)) => all.extend(marks),
                    (Err(error), _) => warn!(%error, server, "could not fetch seen marks from the server"),
                    (Ok(_), Err(_)) => {}
                }
            }
            let _ = events.send(Event::Personal(Box::new(synced)));
        });
    }

    /// Takes in a sync's answer: what the server took, and every state and
    /// mark it holds, merged into this machine's.
    pub(crate) async fn personal_synced(&mut self, synced: Synced) {
        let Synced { sent, fetched, marks_sent, marks } = synced;
        self.marks_synced(marks_sent, marks).await;
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

    async fn marks_synced(&mut self, sent: Vec<(String, SeenMark)>, fetched: Result<Vec<MrSeen>, String>) {
        let mut kept: HashSet<SeenKey> = HashSet::new();
        for (mr, mark) in sent {
            let key = (mr, mark.path.clone());
            // A mark made since this one was sent stays pending.
            if self.personal.seen.get(&key) == Some(&mark) {
                self.personal.seen_pending.remove(&key);
                kept.insert(key);
            }
        }
        let mut changed = false;
        match fetched {
            Ok(requests) => {
                for request in requests {
                    let mr = request.mr.to_string();
                    for mark in request.marks {
                        let key = (mr.clone(), mark.path.clone());
                        if self.personal.take_mark(&mr, mark) {
                            kept.insert(key);
                            changed = true;
                        }
                    }
                }
            }
            Err(error) => warn!(%error, "could not fetch seen marks from the server"),
        }
        for key in kept {
            if let Some(mark) = self.personal.seen.get(&key) {
                self.keep_mark(&key.0, mark, self.personal.seen_pending.contains(&key)).await;
            }
        }
        if changed {
            self.announce(BusEvent::Mrs);
        }
    }
}
