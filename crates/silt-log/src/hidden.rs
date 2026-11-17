//! What a diff leaves out: git's edits (ADR 0069), other agents' (ADR 0086),
//! or both.

use std::collections::BTreeMap;

use silt_types::{ActorId, DeltaId, Lamport};

use crate::delta::{Author, Delta};
use crate::git_made::{GitMade, is_file_edit};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Hide {
    pub git: bool,
    pub others: bool,
}

impl Hide {
    pub fn any(self) -> bool {
        self.git || self.others
    }
}

/// The edits of a log a diff leaves out, as spans of ids.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hidden {
    /// Each span's last lamport, by its actor and first lamport.
    spans: BTreeMap<(ActorId, Lamport), Lamport>,
}

impl Hidden {
    /// `None` when nothing is to be hidden.
    pub fn of(log: &[Delta], hide: Hide) -> Option<Self> {
        if !hide.any() {
            return None;
        }
        let mut spans = if hide.git { GitMade::of(log).spans } else { BTreeMap::new() };
        if hide.others {
            spans.extend(
                log.iter()
                    .filter(|delta| matches!(delta.author, Author::Others { .. }) && is_file_edit(&delta.op))
                    .map(|delta| ((delta.span.actor, delta.span.start), delta.span.last().lamport)),
            );
        }
        Some(Self { spans })
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    pub fn contains(&self, id: DeltaId) -> bool {
        self.spans
            .range(..=(id.actor, id.lamport))
            .next_back()
            .is_some_and(|((actor, _), last)| *actor == id.actor && id.lamport <= *last)
    }
}

#[cfg(test)]
mod tests {
    use silt_crdt::{InodeId, TextOp};
    use silt_types::{DeltaSpan, Frontier, ThreadId};

    use super::*;
    use crate::delta::{ConvOp, GitOp, Op, SessionRef};

    const ACTOR: ActorId = ActorId(1);

    fn delta(start: u64, author: Author, op: Op) -> Delta {
        let span = DeltaSpan::new(ACTOR, Lamport(start), op.units() as u64);
        Delta { span, parents: Frontier::root(), thread: ThreadId(1), author, time_ms: 0, op }
    }

    fn typed(text: &str) -> Op {
        Op::Text { inode: InodeId(DeltaId::new(ACTOR, Lamport(0))), op: TextOp::Insert { pos: 0, content: text.into() } }
    }

    #[test]
    fn other_agents_edits_and_gits_are_hidden_as_asked() {
        let session = |id: &str| SessionRef { agent: "claude".into(), id: id.into() };
        let tool = |tool: &str| Author::Agent { agent: "claude".into(), session: "mine".into(), tool_use_id: Some(tool.into()) };
        let log = [
            delta(10, tool("edit"), typed("mine")),
            delta(20, Author::Others { sessions: vec![session("theirs")] }, typed("theirs")),
            delta(30, Author::Agents { sessions: vec![session("mine"), session("theirs")] }, typed("either")),
            delta(40, tool("pull"), typed("pulled")),
            delta(50, Author::Others { sessions: vec![session("theirs")] }, Op::Conv(ConvOp::Title("t".into()))),
            delta(60, Author::User, Op::Git(GitOp::Rewrote { session: session("mine"), tool_use_id: "pull".into() })),
        ];
        let id = |lamport| DeltaId::new(ACTOR, Lamport(lamport));
        assert_eq!(Hidden::of(&log, Hide::default()), None);
        let others = Hidden::of(&log, Hide { git: false, others: true }).expect("hides");
        assert!(others.contains(id(20)) && others.contains(id(25)));
        assert!(!others.contains(id(10)) && !others.contains(id(30)) && !others.contains(id(40)) && !others.contains(id(50)));
        let git = Hidden::of(&log, Hide { git: true, others: false }).expect("hides");
        assert!(git.contains(id(40)) && !git.contains(id(20)));
        let both = Hidden::of(&log, Hide { git: true, others: true }).expect("hides");
        assert!(both.contains(id(40)) && both.contains(id(20)) && !both.contains(id(10)) && !both.contains(id(30)));
    }
}
