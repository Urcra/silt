//! Where each line of a file came from (ADR 0042): the delta that inserted
//! it, with that delta's thread, turn and author.

use silt_crdt::TextOp;
use silt_types::{DeltaId, ThreadId};

use crate::delta::{Author, ConvOp, Delta, Op, TurnMark};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineOrigin {
    pub delta: DeltaId,
    pub thread: ThreadId,
    /// Counted as the thread's own turns are, from 1; none before the first.
    pub turn: Option<usize>,
    pub author: Author,
    pub time_ms: i64,
}

/// The family's text inserts, found by the ids of their characters.
#[derive(Debug, Default)]
pub struct Origins {
    /// Sorted by actor, then first lamport.
    inserts: Vec<(Delta, Option<usize>)>,
}

impl Origins {
    pub fn new(log: &[Delta]) -> Self {
        let mut turns: std::collections::HashMap<ThreadId, usize> = std::collections::HashMap::new();
        let mut inserts = Vec::new();
        for delta in log {
            match &delta.op {
                Op::Conv(ConvOp::Turn { mark: TurnMark::PromptSubmitted, .. }) => *turns.entry(delta.thread).or_default() += 1,
                Op::Text { op: TextOp::Insert { .. }, .. } => {
                    inserts.push((delta.clone(), turns.get(&delta.thread).copied()));
                }
                _ => {}
            }
        }
        inserts.sort_by_key(|(delta, _)| (delta.span.actor, delta.span.start));
        Self { inserts }
    }

    pub fn of(&self, id: DeltaId) -> Option<LineOrigin> {
        let after = self.inserts.partition_point(|(delta, _)| (delta.span.actor, delta.span.start) <= (id.actor, id.lamport));
        let (delta, turn) = self.inserts[..after].last().filter(|(delta, _)| delta.span.contains(id))?;
        Some(LineOrigin {
            delta: delta.span.first(),
            thread: delta.thread,
            turn: *turn,
            author: delta.author.clone(),
            time_ms: delta.time_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use silt_crdt::{InodeKind, Parent};
    use silt_types::{ActorId, Frontier};

    use super::*;
    use crate::{Content, Family, SessionRef};

    #[test]
    fn each_line_names_the_turn_and_author_that_wrote_it() {
        let thread = ThreadId(1);
        let mut family = Family::new();
        let session = SessionRef { agent: "claude-code".into(), id: "s".into() };
        let agent = Author::Agent { agent: "claude-code".into(), session: "s".into(), tool_use_id: None };
        let apply = |family: &mut Family, author: Author, op: Op| {
            let delta = family.delta(ActorId(1), thread, author, 7, family.thread_head(thread), op);
            let id = delta.span.first();
            family.apply(delta, &mut std::collections::HashMap::new()).expect("applies");
            id
        };
        let file = apply(
            &mut family,
            Author::User,
            Op::Create {
                kind: InodeKind::File,
                parent: Parent::Root,
                name: "a.rs".into(),
                content: Content::Empty,
                executable: false,
            },
        );
        let inode = silt_crdt::InodeId(file);
        let prompt = || Op::Conv(ConvOp::Turn { session: session.clone(), mark: TurnMark::PromptSubmitted });
        apply(&mut family, Author::User, prompt());
        let first = apply(&mut family, agent, Op::Text { inode, op: TextOp::Insert { pos: 0, content: "one\ntwo\n".into() } });
        apply(&mut family, Author::User, prompt());
        apply(&mut family, Author::User, Op::Text { inode, op: TextOp::Insert { pos: 4, content: "2".into() } });
        apply(&mut family, Author::User, Op::Text { inode, op: TextOp::Insert { pos: 9, content: "three\n".into() } });

        let vv = family.graph().version_vector(family.graph().frontier());
        let lines = family.lines_at("a.rs", &vv).expect("a text file");
        let origins = Origins::new(family.log());
        let shown: Vec<(&str, Option<usize>, bool)> = lines
            .iter()
            .map(|(text, id)| {
                let origin = origins.of(id.expect("written in Silt")).expect("an insert");
                (text.as_str(), origin.turn, matches!(origin.author, Author::Agent { .. }))
            })
            .collect();
        assert_eq!(shown, [("one", Some(1), true), ("2two", Some(2), false), ("three", Some(2), false)]);

        let before = family.graph().version_vector(&Frontier::single(DeltaId::new(ActorId(1), first.lamport.offset(7))));
        assert_eq!(family.lines_at("a.rs", &before).expect("there").len(), 2, "the text as it was then");
    }
}
