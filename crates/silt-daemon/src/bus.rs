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
        let length = self.local_log_len(family);
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
            self.announce(BusEvent::Agent { thread: thread.to_string(), status: self.shown_status(thread).map(Box::new) });
        }
    }

    pub(crate) fn announce_text(&self, thread: ThreadId, kind: TextKind, text: &str) {
        if self.heard() && !text.is_empty() {
            let thread = thread.to_string();
            self.announce(BusEvent::AgentText { thread, kind, text: text.to_owned(), tool_use_id: None, restart: false });
        }
    }

    /// What's new in a tool call's output, which ACP sends whole each time.
    pub(crate) fn announce_tool_output(&self, thread: ThreadId, tool_use_id: &str, before: &str, output: &str) {
        if !self.heard() {
            return;
        }
        let Some((text, restart)) = piece(before, output) else {
            return;
        };
        self.announce(BusEvent::AgentText {
            thread: thread.to_string(),
            kind: TextKind::Tool,
            text,
            tool_use_id: Some(tool_use_id.to_owned()),
            restart,
        });
    }
}

/// What `output` adds to `before`, or all of it when it replaced `before`
/// (`true`); nothing when it is the same.
fn piece(before: &str, output: &str) -> Option<(String, bool)> {
    if before == output {
        return None;
    }
    Some(match output.strip_prefix(before) {
        Some(added) => (added.to_owned(), false),
        None => (output.to_owned(), true),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_s_output_is_sent_a_piece_at_a_time() {
        assert_eq!(piece("", "compiling"), Some(("compiling".into(), false)));
        assert_eq!(piece("compiling", "compiling\nok"), Some(("\nok".into(), false)));
        assert_eq!(piece("compiling\nok", "compiling\nok"), None);
        assert_eq!(piece("step 1", "step 2"), Some(("step 2".into(), true)), "a replaced output starts over");
    }
}
