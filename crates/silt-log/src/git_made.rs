//! The edits git made rather than the agent: those credited to a tool call
//! a `GitOp::Rewrote` names, which a diff may leave out (ADR 0066).

use std::collections::{BTreeMap, HashSet};

use silt_types::{ActorId, DeltaId, Lamport};

use crate::delta::{Author, Delta, GitOp, Op};

/// The deltas of a log that git made, as spans of ids.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitMade {
    /// Each span's last lamport, by its actor and first lamport.
    spans: BTreeMap<(ActorId, Lamport), Lamport>,
}

impl GitMade {
    pub fn of(log: &[Delta]) -> Self {
        let rewrote: HashSet<(&str, &str)> = log
            .iter()
            .filter_map(|delta| match &delta.op {
                Op::Git(GitOp::Rewrote { session, tool_use_id }) => Some((session.id.as_str(), tool_use_id.as_str())),
                _ => None,
            })
            .collect();
        if rewrote.is_empty() {
            return Self::default();
        }
        let spans = log
            .iter()
            .filter(|delta| is_file_edit(&delta.op))
            .filter(|delta| match &delta.author {
                Author::Agent { session, tool_use_id: Some(tool), .. } => rewrote.contains(&(session.as_str(), tool.as_str())),
                _ => false,
            })
            .map(|delta| ((delta.span.actor, delta.span.start), delta.span.last().lamport))
            .collect();
        Self { spans }
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

fn is_file_edit(op: &Op) -> bool {
    matches!(
        op,
        Op::Create { .. }
            | Op::Move { .. }
            | Op::Delete { .. }
            | Op::Text { .. }
            | Op::SetContent { .. }
            | Op::SetExecutable { .. }
    )
}

#[cfg(test)]
mod tests {
    use silt_crdt::{InodeId, TextOp};
    use silt_types::{DeltaSpan, Frontier, ThreadId};

    use super::*;
    use crate::delta::{ConvOp, SessionRef};

    const ACTOR: ActorId = ActorId(1);

    fn delta(start: u64, author: Author, op: Op) -> Delta {
        let span = DeltaSpan::new(ACTOR, Lamport(start), op.units() as u64);
        Delta { span, parents: Frontier::root(), thread: ThreadId(1), author, time_ms: 0, op }
    }

    fn by_tool(tool: &str) -> Author {
        Author::Agent { agent: "claude".into(), session: "s".into(), tool_use_id: Some(tool.into()) }
    }

    fn typed(text: &str) -> Op {
        Op::Text { inode: InodeId(DeltaId::new(ACTOR, Lamport(0))), op: TextOp::Insert { pos: 0, content: text.into() } }
    }

    #[test]
    fn the_edits_of_a_tool_call_that_ran_git_are_gits() {
        let session = SessionRef { agent: "claude".into(), id: "s".into() };
        let log = [
            delta(10, by_tool("edit"), typed("mine")),
            delta(20, by_tool("pull"), typed("theirs")),
            delta(30, by_tool("pull"), Op::Conv(ConvOp::Title("t".into()))),
            delta(40, Author::User, Op::Git(GitOp::Rewrote { session, tool_use_id: "pull".into() })),
        ];
        let made = GitMade::of(&log);
        let id = |lamport| DeltaId::new(ACTOR, Lamport(lamport));
        assert!(!made.contains(id(10)) && !made.contains(id(13)));
        assert!(made.contains(id(20)) && made.contains(id(25)));
        assert!(!made.contains(id(26)) && !made.contains(id(30)));
        assert!(!made.contains(DeltaId::new(ActorId(2), Lamport(21))));
        assert!(GitMade::of(&log[..2]).is_empty());
    }
}
