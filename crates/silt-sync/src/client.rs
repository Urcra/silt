use silt_log::Delta;
use silt_types::DeltaId;

/// Most deltas sent in one push.
pub const PUSH_BATCH: u64 = 500;

/// Push local deltas `from..to` (positions in the local log).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Push {
    pub from: u64,
    pub to: u64,
}

/// Pull deltas after the server `seq` `after`, whose delta was `last`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pull {
    pub after: u64,
    pub last: Option<DeltaId>,
}

/// How the server answered.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Pushed,
    Pulled(Vec<(u64, Delta)>),
    /// The server doesn't have the history this client synced with.
    UnknownCursor,
    MissingParents,
    Conflict(DeltaId),
    /// No answer; the request may or may not have been applied.
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Apply these deltas from the server, in order, each with its `seq`.
    /// `after` is where the pull started, to come back to with
    /// [`ClientSync::rewind_pull`].
    Apply { after: (u64, Option<DeltaId>), deltas: Vec<(u64, Delta)> },
    /// This client's deltas collide with different ones on the server.
    Diverged(DeltaId),
}

/// One family's sync progress. Push and pull each have at most one request
/// in flight; any failure just leaves the slot free for a retry. Answers
/// carry the server's generation, which changes when the server restarts;
/// on a change everything is pushed and pulled again, so a server restored
/// from an older state gets back whatever it lost.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientSync {
    generation: Option<u64>,
    pushed: u64,
    pulled: u64,
    last_pulled: Option<DeltaId>,
    push_in_flight: Option<Push>,
    pull_in_flight: Option<Pull>,
    /// Set once the server holds a different delta under one of ours. Later
    /// local deltas build on it, so pushing them could only fail.
    diverged: Option<DeltaId>,
    /// The server's fingerprint of what this client may read of the family.
    view: u64,
}

impl ClientSync {
    pub fn new(generation: Option<u64>, pushed: u64, pulled: u64, last_pulled: Option<DeltaId>, view: u64) -> Self {
        Self { generation, pushed, pulled, last_pulled, view, ..Self::default() }
    }

    pub fn generation(&self) -> Option<u64> {
        self.generation
    }

    pub fn pushed(&self) -> u64 {
        self.pushed
    }

    pub fn pulled(&self) -> u64 {
        self.pulled
    }

    pub fn last_pulled(&self) -> Option<DeltaId> {
        self.last_pulled
    }

    pub fn diverged(&self) -> Option<DeltaId> {
        self.diverged
    }

    pub fn view(&self) -> u64 {
        self.view
    }

    /// Takes the server's current view fingerprint. When it changed, deltas
    /// skipped earlier may be readable now, so pulling starts over. So it
    /// does when the first view comes after pulling: those pulls were read
    /// under a view this client never saw.
    pub fn observe_view(&mut self, view: u64) -> bool {
        if view == self.view {
            return false;
        }
        let changed = self.view != 0 || self.pulled > 0;
        self.view = view;
        if changed {
            self.pulled = 0;
            self.last_pulled = None;
        }
        changed
    }

    /// The next push, given how many deltas the local log has.
    pub fn next_push(&mut self, local_len: u64) -> Option<Push> {
        if self.push_in_flight.is_some() || self.diverged.is_some() || self.pushed >= local_len {
            return None;
        }
        let push = Push { from: self.pushed, to: local_len.min(self.pushed + PUSH_BATCH) };
        self.push_in_flight = Some(push);
        Some(push)
    }

    pub fn next_pull(&mut self) -> Option<Pull> {
        if self.pull_in_flight.is_some() {
            return None;
        }
        let pull = Pull { after: self.pulled, last: self.last_pulled };
        self.pull_in_flight = Some(pull);
        Some(pull)
    }

    /// The answer to the push in flight, from a server of `generation`
    /// (`None` when there was no answer).
    pub fn on_push(&mut self, generation: Option<u64>, outcome: Outcome) -> Vec<Effect> {
        let Some(push) = self.push_in_flight.take() else {
            return Vec::new();
        };
        if self.restarted(generation) {
            return Vec::new();
        }
        match outcome {
            // A push sent before a reset only counts if it covers the cursor.
            Outcome::Pushed if push.from <= self.pushed => {
                self.pushed = self.pushed.max(push.to);
                Vec::new()
            }
            Outcome::Pushed => Vec::new(),
            Outcome::UnknownCursor | Outcome::MissingParents => {
                self.reset();
                Vec::new()
            }
            Outcome::Conflict(id) => {
                self.diverged = Some(id);
                vec![Effect::Diverged(id)]
            }
            Outcome::Failed | Outcome::Pulled(_) => Vec::new(),
        }
    }

    pub fn on_pull(&mut self, generation: Option<u64>, outcome: Outcome) -> Vec<Effect> {
        let Some(pull) = self.pull_in_flight.take() else {
            return Vec::new();
        };
        if self.restarted(generation) {
            return Vec::new();
        }
        match outcome {
            // A pull sent before a reset continues a history this client no
            // longer builds on, so its deltas may lack parents here.
            Outcome::Pulled(_) if pull.after != self.pulled => Vec::new(),
            Outcome::Pulled(deltas) => {
                let after = (self.pulled, self.last_pulled);
                if let Some((seq, delta)) = deltas.last() {
                    self.pulled = *seq;
                    self.last_pulled = Some(delta.span.first());
                }
                if deltas.is_empty() { Vec::new() } else { vec![Effect::Apply { after, deltas }] }
            }
            Outcome::UnknownCursor => {
                self.reset();
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Pulls again from the server `seq` `after`, whose delta was `last`:
    /// what came after it couldn't be applied yet.
    pub fn rewind_pull(&mut self, after: u64, last: Option<DeltaId>) {
        if after < self.pulled {
            self.pulled = after;
            self.last_pulled = last;
        }
    }

    /// Deltas from the server were appended to the local log right after
    /// position `at`; when nothing local is waiting before them they don't
    /// need pushing back.
    pub fn appended_from_server(&mut self, at: u64, count: u64) {
        if self.pushed == at && self.push_in_flight.is_none() {
            self.pushed += count;
        }
    }

    /// Whether the answer came from a server generation other than the
    /// known one; if so, sync starts over. A first answer just sets it.
    fn restarted(&mut self, generation: Option<u64>) -> bool {
        let Some(generation) = generation else {
            return false;
        };
        match self.generation.replace(generation) {
            Some(known) if known != generation => {
                self.reset();
                true
            }
            _ => false,
        }
    }

    /// A restarted server may have lost the delta this client collided
    /// with, so pushing is tried again too.
    fn reset(&mut self) {
        self.pushed = 0;
        self.pulled = 0;
        self.last_pulled = None;
        self.diverged = None;
    }
}

#[cfg(test)]
mod tests {
    use silt_types::{ActorId, Lamport};

    use super::*;

    #[test]
    fn a_conflict_stops_pushing_but_not_pulling() {
        let mut sync = ClientSync::default();
        assert_eq!(sync.next_push(3), Some(Push { from: 0, to: 3 }));
        let id = DeltaId::new(ActorId(1), Lamport(0));
        assert_eq!(sync.on_push(Some(7), Outcome::Conflict(id)), vec![Effect::Diverged(id)]);
        assert_eq!(sync.diverged(), Some(id));
        assert_eq!(sync.next_push(4), None);
        assert!(sync.next_pull().is_some());
    }

    #[test]
    fn a_restarted_server_gets_pushes_again_after_a_conflict() {
        let mut sync = ClientSync::default();
        sync.next_push(3);
        let id = DeltaId::new(ActorId(1), Lamport(0));
        sync.on_push(Some(7), Outcome::Conflict(id));
        sync.next_pull();
        sync.on_pull(Some(8), Outcome::Pulled(Vec::new()));
        assert_eq!(sync.diverged(), None, "the new server may not have the colliding delta");
        assert_eq!(sync.next_push(3), Some(Push { from: 0, to: 3 }));
    }

    #[test]
    fn a_new_view_pulls_again_from_the_start() {
        let mut sync = ClientSync::new(Some(1), 4, 0, None, 0);
        assert!(!sync.observe_view(7), "the first view before pulling is just remembered");
        let mut sync = ClientSync::new(Some(1), 4, 9, Some(DeltaId::new(ActorId(1), Lamport(3))), 7);
        assert!(!sync.observe_view(7));
        assert_eq!(sync.pulled(), 9);
        assert!(sync.observe_view(8));
        assert_eq!((sync.pulled(), sync.last_pulled(), sync.pushed()), (0, None, 4));
    }

    #[test]
    fn a_first_view_after_pulling_pulls_again() {
        let mut sync = ClientSync::new(Some(1), 4, 9, Some(DeltaId::new(ActorId(1), Lamport(3))), 0);
        assert!(sync.observe_view(7), "what was pulled was read under a view never seen");
        assert_eq!((sync.pulled(), sync.last_pulled()), (0, None));
    }
}
