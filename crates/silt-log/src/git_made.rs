//! The edits git made rather than the agent or the person: those credited
//! to a tool call a `GitOp::Rewrote` names, and those a `GitOp::Made` lists,
//! which a diff may leave out (ADR 0069).

use std::collections::{BTreeMap, HashSet};

use silt_crdt::VersionVector;
use silt_types::{ActorId, DeltaId, DeltaSpan, Lamport, ThreadId};

use crate::delta::{Author, Delta, GitOp, Op};

/// The deltas of a log that git made, as spans of ids.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitMade {
    /// Each span's last lamport, by its actor and first lamport.
    pub(crate) spans: BTreeMap<(ActorId, Lamport), Lamport>,
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
        let mut spans: BTreeMap<(ActorId, Lamport), Lamport> = log
            .iter()
            .flat_map(|delta| match &delta.op {
                Op::Git(GitOp::Made { spans }) => spans.as_slice(),
                _ => &[],
            })
            .filter(|span| span.len > 0)
            .map(|span| ((span.actor, span.start), span.last().lamport))
            .collect();
        if !rewrote.is_empty() {
            spans.extend(
                log.iter()
                    .filter(|delta| is_file_edit(&delta.op))
                    .filter(|delta| match &delta.author {
                        Author::Agent { session, tool_use_id: Some(tool), .. } => {
                            rewrote.contains(&(session.as_str(), tool.as_str()))
                        }
                        _ => false,
                    })
                    .map(|delta| ((delta.span.actor, delta.span.start), delta.span.last().lamport)),
            );
        }
        Self { spans }
    }

    /// The edits of `thread` recorded as the person's after `since`: those
    /// git made when it moved the refs between two looks.
    pub fn persons_edits_since(log: &[Delta], thread: ThreadId, since: &VersionVector) -> Vec<DeltaSpan> {
        log.iter()
            .filter(|delta| delta.thread == thread && delta.author == Author::User && is_file_edit(&delta.op))
            .filter(|delta| !since.contains(delta.span.first()))
            .map(|delta| delta.span)
            .collect()
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

pub(crate) fn is_file_edit(op: &Op) -> bool {
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

    #[test]
    fn the_persons_edits_since_a_look_can_be_marked_as_gits() {
        let log = [
            delta(10, Author::User, typed("before")),
            delta(20, Author::User, typed("pulled")),
            delta(30, by_tool("edit"), typed("mine")),
            delta(40, Author::User, Op::Conv(ConvOp::Title("t".into()))),
        ];
        let since = VersionVector::default();
        let spans = GitMade::persons_edits_since(&log, ThreadId(1), &since);
        assert_eq!(spans, vec![log[0].span, log[1].span]);
        let marked = [delta(50, Author::User, Op::Git(GitOp::Made { spans: vec![log[1].span] }))];
        let made = GitMade::of(&[log.as_slice(), &marked].concat());
        assert!(made.contains(DeltaId::new(ACTOR, Lamport(22))));
        assert!(!made.contains(DeltaId::new(ACTOR, Lamport(12))) && !made.contains(DeltaId::new(ACTOR, Lamport(30))));
    }
}
