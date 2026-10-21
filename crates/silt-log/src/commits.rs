//! The commits a thread's checkout made, each with the turn it came in
//! (ADR 0030), as the log records them.

use silt_types::{DeltaId, GitOid, ThreadId};

use crate::delta::{ConvOp, Delta, GitOp, Op, TurnMark};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadCommit {
    pub commit: GitOid,
    pub branch: Option<String>,
    /// The turn, counted as the thread's own turns are from 1, whose prompt
    /// came last before the commit; none before the first.
    pub turn: Option<usize>,
    pub time_ms: i64,
    pub delta: DeltaId,
}

/// The thread's commits in the order the log took them.
pub fn commits_in(log: &[Delta], thread: ThreadId) -> Vec<ThreadCommit> {
    let mut turn = 0;
    let mut commits = Vec::new();
    for delta in log.iter().filter(|delta| delta.thread == thread) {
        match &delta.op {
            Op::Conv(ConvOp::Turn { mark: TurnMark::PromptSubmitted, .. }) => turn += 1,
            Op::Git(GitOp::Committed { commit, branch }) => commits.push(ThreadCommit {
                commit: *commit,
                branch: branch.clone(),
                turn: (turn > 0).then_some(turn),
                time_ms: delta.time_ms,
                delta: delta.span.first(),
            }),
            _ => {}
        }
    }
    commits
}

#[cfg(test)]
mod tests {
    use silt_types::{ActorId, Frontier};

    use super::*;
    use crate::{Author, Family, SessionRef};

    #[test]
    fn a_commit_belongs_to_the_turn_it_came_in() {
        let (thread, other) = (ThreadId(1), ThreadId(2));
        let family = Family::new();
        let session = SessionRef { agent: "claude-code".into(), id: "s".into() };
        let mut log = Vec::new();
        let mut write = |thread: ThreadId, op: Op| {
            let delta = family.delta(ActorId(1), thread, Author::User, 0, Frontier::root(), op);
            log.push(delta);
        };
        let committed = |byte: u8| Op::Git(GitOp::Committed { commit: GitOid([byte; 20]), branch: Some("main".into()) });
        let prompt = || Op::Conv(ConvOp::Turn { session: session.clone(), mark: TurnMark::PromptSubmitted });
        write(thread, committed(1));
        write(thread, prompt());
        write(thread, committed(2));
        write(other, prompt());
        write(thread, prompt());
        write(thread, committed(3));
        write(other, committed(4));

        let commits = commits_in(&log, thread);
        let turns: Vec<_> = commits.iter().map(|commit| (commit.commit.0[0], commit.turn)).collect();
        assert_eq!(turns, [(1, None), (2, Some(1)), (3, Some(2))], "another thread's turns and commits don't count");
    }
}
