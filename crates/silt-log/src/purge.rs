//! What survives purging threads (ADR 0013): every delta of another thread,
//! and every delta those build on, so the remaining threads stay whole.

use std::collections::{HashMap, HashSet};

use silt_types::{ActorId, DeltaId, ThreadId};

use crate::Delta;

/// For each delta of `log`, which is in causal order, whether it is kept
/// once the `purged` threads are gone.
pub fn kept_after_purge(log: &[Delta], purged: &HashSet<ThreadId>) -> Vec<bool> {
    let mut spans: HashMap<ActorId, Vec<(u64, u64, usize)>> = HashMap::new();
    for (index, delta) in log.iter().enumerate() {
        spans.entry(delta.span.actor).or_default().push((delta.span.first().lamport.0, delta.span.last().lamport.0, index));
    }
    for runs in spans.values_mut() {
        runs.sort_unstable();
    }
    let locate = |id: DeltaId| -> Option<usize> {
        let runs = spans.get(&id.actor)?;
        let at = runs.partition_point(|(start, _, _)| *start <= id.lamport.0);
        let (start, end, index) = *runs.get(at.checked_sub(1)?)?;
        (start..=end).contains(&id.lamport.0).then_some(index)
    };
    let mut kept: Vec<bool> = log.iter().map(|delta| !purged.contains(&delta.thread)).collect();
    for index in (0..log.len()).rev() {
        if kept[index] {
            for parent in log[index].parents.ids() {
                if let Some(found) = locate(*parent) {
                    kept[found] = true;
                }
            }
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use silt_crdt::{InodeKind, Parent};

    use super::*;
    use crate::{Author, Content, Family, Op};

    fn create(family: &mut Family, thread: ThreadId, name: &str) -> Delta {
        let delta = family.delta(
            ActorId(thread.0),
            thread,
            Author::User,
            0,
            family.thread_head(thread),
            Op::Create {
                kind: InodeKind::File,
                parent: Parent::Root,
                name: name.into(),
                content: Content::Empty,
                executable: false,
            },
        );
        family.apply(delta.clone(), &mut HashMap::new()).expect("applies");
        delta
    }

    fn fork(family: &mut Family, from: ThreadId, thread: ThreadId) -> Delta {
        let delta = family.delta(ActorId(thread.0), thread, Author::User, 0, family.thread_head(from), Op::Fork { from });
        family.apply(delta.clone(), &mut HashMap::new()).expect("applies");
        delta
    }

    #[test]
    fn a_purged_thread_leaves_what_others_build_on() {
        let (main, fork_thread, other) = (ThreadId(1), ThreadId(2), ThreadId(3));
        let mut family = Family::new();
        let mut log = vec![create(&mut family, main, "a"), create(&mut family, main, "b")];
        log.push(fork(&mut family, main, fork_thread));
        log.push(create(&mut family, fork_thread, "c"));
        log.push(create(&mut family, main, "d"));
        log.push(fork(&mut family, main, other));
        log.push(create(&mut family, other, "e"));

        let only_fork: HashSet<ThreadId> = [fork_thread].into();
        assert_eq!(kept_after_purge(&log, &only_fork), vec![true, true, false, false, true, true, true]);
        let main_gone: HashSet<ThreadId> = [main].into();
        assert_eq!(kept_after_purge(&log, &main_gone), vec![true; 7], "the forks build on all of main");
        let both: HashSet<ThreadId> = [main, fork_thread].into();
        assert_eq!(kept_after_purge(&log, &both), vec![true, true, false, false, true, true, true]);
        let all: HashSet<ThreadId> = [main, fork_thread, other].into();
        assert!(kept_after_purge(&log, &all).iter().all(|kept| !kept));
    }
}
