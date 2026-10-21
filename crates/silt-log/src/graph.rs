//! What the history graph draws, from a family's log: each thread's turns
//! and commits, where threads forked, the merge requests offered, and the
//! merges (ADR 0029, ADR 0030). Text such as a turn's prompt is the
//! caller's to add, from its transcript readers.

use silt_types::{DeltaId, GitOid, ThreadId};

use crate::delta::{ConvOp, Delta, GitOp, MrOp, Op, TurnMark};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphEvent {
    /// The thread's `number`th turn began, counted from 1.
    Turn {
        thread: ThreadId,
        number: usize,
    },
    Commit {
        thread: ThreadId,
        commit: GitOid,
        turn: Option<usize>,
    },
    Fork {
        thread: ThreadId,
        from: ThreadId,
    },
    /// A merge request from the fork `thread` into `target`.
    Request {
        thread: ThreadId,
        target: ThreadId,
        mr: DeltaId,
        title: String,
    },
    /// `thread` took in the fork `from`.
    Merged {
        thread: ThreadId,
        from: ThreadId,
        title: String,
    },
}

/// One event, with the delta it comes from and its time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphItem {
    pub event: GraphEvent,
    pub delta: DeltaId,
    pub time_ms: i64,
}

/// The family's events in the order the log took them.
pub fn graph_of(log: &[Delta]) -> Vec<GraphItem> {
    let mut turns: std::collections::HashMap<ThreadId, usize> = std::collections::HashMap::new();
    let mut items = Vec::new();
    for delta in log {
        let thread = delta.thread;
        let event = match &delta.op {
            Op::Conv(ConvOp::Turn { mark: TurnMark::PromptSubmitted, .. }) => {
                let number = turns.entry(thread).or_default();
                *number += 1;
                GraphEvent::Turn { thread, number: *number }
            }
            Op::Git(GitOp::Committed { commit, .. }) => {
                GraphEvent::Commit { thread, commit: *commit, turn: turns.get(&thread).copied() }
            }
            Op::Fork { from } => GraphEvent::Fork { thread, from: *from },
            Op::Mr(MrOp::Opened { target, title, .. }) => {
                GraphEvent::Request { thread, target: *target, mr: delta.span.first(), title: title.clone() }
            }
            Op::Conv(ConvOp::Merged { from, title, .. }) => GraphEvent::Merged { thread, from: *from, title: title.clone() },
            _ => continue,
        };
        items.push(GraphItem { event, delta: delta.span.first(), time_ms: delta.time_ms });
    }
    items
}

#[cfg(test)]
mod tests {
    use silt_types::{ActorId, Frontier};

    use super::*;
    use crate::{Author, Family, SessionRef};

    #[test]
    fn a_familys_history_is_its_turns_commits_forks_requests_and_merges() {
        let (main, fork) = (ThreadId(1), ThreadId(2));
        let family = Family::new();
        let session = SessionRef { agent: "claude-code".into(), id: "s".into() };
        let mut log = Vec::new();
        let mut write =
            |thread, time_ms, op| log.push(family.delta(ActorId(1), thread, Author::User, time_ms, Frontier::root(), op));
        let prompt = || Op::Conv(ConvOp::Turn { session: session.clone(), mark: TurnMark::PromptSubmitted });
        write(main, 1, prompt());
        write(main, 2, Op::Git(GitOp::Committed { commit: GitOid([7; 20]), branch: None }));
        write(fork, 3, Op::Fork { from: main });
        write(fork, 4, prompt());
        write(fork, 5, Op::Mr(MrOp::Opened { target: main, title: "Tidy".into(), summary: String::new() }));
        write(
            main,
            6,
            Op::Conv(ConvOp::Merged {
                from: fork,
                mr: DeltaId::new(ActorId(1), silt_types::Lamport(5)),
                title: "Tidy".into(),
                summary: String::new(),
            }),
        );

        let events: Vec<_> = graph_of(&log).into_iter().map(|item| (item.time_ms, item.event)).collect();
        assert_eq!(events[0], (1, GraphEvent::Turn { thread: main, number: 1 }));
        assert_eq!(events[1], (2, GraphEvent::Commit { thread: main, commit: GitOid([7; 20]), turn: Some(1) }));
        assert_eq!(events[2], (3, GraphEvent::Fork { thread: fork, from: main }));
        assert_eq!(events[3], (4, GraphEvent::Turn { thread: fork, number: 1 }), "each thread counts its own turns");
        assert!(
            matches!(&events[4].1, GraphEvent::Request { thread, target, title, .. } if *thread == fork && *target == main && title == "Tidy")
        );
        assert_eq!(events[5], (6, GraphEvent::Merged { thread: main, from: fork, title: "Tidy".into() }));
    }
}
