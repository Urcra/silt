//! Telling browsers what changed (ADR 0021): the core sends events to a
//! broadcast channel that every `/api/events` socket reads and filters by
//! what it subscribed to.

use silt_log::{Delta, Op};
use silt_types::{FamilyId, ThreadId};

use crate::core::Core;
use crate::protocol::{BusEvent, TextKind};

/// Events a socket may fall behind by before it is told it lagged.
pub(crate) const CAPACITY: usize = 1024;

impl Core {
    /// Whether any browser listens, so no event is built for nobody.
    pub(crate) fn heard(&self) -> bool {
        self.bus.receiver_count() > 0
    }

    pub(crate) fn announce(&self, event: BusEvent) {
        if self.heard() {
            let _ = self.bus.send(event);
        }
    }

    /// `deltas` were stored for the family: each of its threads changed,
    /// and the listing may have.
    pub(crate) fn announce_family(&self, family: FamilyId, deltas: &[Delta]) {
        if !self.heard() {
            return;
        }
        let length = self.local_log(family).len();
        for thread in self.threads.values().filter(|thread| thread.family == family) {
            self.announce(BusEvent::Thread { thread: thread.id.to_string(), deltas: length });
        }
        if deltas.iter().any(|delta| matches!(delta.op, Op::Mr(_))) {
            self.announce(BusEvent::Mrs);
        }
        self.announce(BusEvent::Threads);
    }

    pub(crate) fn announce_agent(&self, thread: ThreadId) {
        if self.heard() {
            self.announce(BusEvent::Agent { thread: thread.to_string(), status: self.agent_status(thread) });
        }
    }

    pub(crate) fn announce_text(&self, thread: ThreadId, kind: TextKind, text: &str) {
        if self.heard() && !text.is_empty() {
            self.announce(BusEvent::AgentText { thread: thread.to_string(), kind, text: text.to_owned() });
        }
    }
}
