//! Deltas a hostile or buggy peer could send (ADR 0016): each is refused
//! without crashing, hanging or changing the family.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use silt_crdt::{InodeId, InodeKind, Parent, TextOp};
use silt_log::{Applied, Author, Content, ConvOp, Delta, Family, LogError, Op};
use silt_types::{ActorId, DeltaSpan, Frontier, Lamport, ThreadId};

const ALICE: ActorId = ActorId(1);
const MALLORY: ActorId = ActorId(66);
const MAIN: ThreadId = ThreadId(7);
const FORK: ThreadId = ThreadId(8);

fn apply(family: &mut Family, delta: Delta) -> Result<Applied, LogError> {
    family.apply(delta, &mut HashMap::new())
}

fn write(family: &mut Family, actor: ActorId, thread: ThreadId, op: Op) -> Delta {
    let head = family.thread_head(thread);
    let delta = family.delta(actor, thread, Author::User, 0, head, op);
    assert_eq!(apply(family, delta.clone()), Ok(Applied::New));
    delta
}

fn create(parent: Parent, name: &str, kind: InodeKind) -> Op {
    Op::Create {
        kind,
        parent,
        name: name.into(),
        content: Content::Empty,
        executable: false,
    }
}

/// A family with a `src` directory and a file in it, on the main thread.
fn family() -> (Family, InodeId, InodeId) {
    let mut family = Family::new();
    let src = write(&mut family, ALICE, MAIN, create(Parent::Root, "src", InodeKind::Dir));
    let src = InodeId(src.span.first());
    let file = write(&mut family, ALICE, MAIN, create(Parent::Dir(src), "a.txt", InodeKind::File));
    (family, src, InodeId(file.span.first()))
}

fn from_mallory(family: &Family, thread: ThreadId, op: Op) -> Delta {
    family.delta(MALLORY, thread, Author::User, 0, family.thread_head(MAIN), op)
}

fn assert_refused(family: &mut Family, delta: Delta) {
    let before = family.log().len();
    let started = Instant::now();
    assert!(apply(family, delta).is_err());
    assert!(started.elapsed() < Duration::from_secs(1), "refusing took {:?}", started.elapsed());
    assert_eq!(family.log().len(), before);
}

#[test]
fn files_in_a_git_directory_are_refused() {
    let (mut family, src, _) = family();
    for name in [".git", ".GIT", ".git.", "git~1"] {
        let delta = from_mallory(&family, MAIN, create(Parent::Root, name, InodeKind::Dir));
        assert_refused(&mut family, delta);
        let delta = from_mallory(&family, MAIN, create(Parent::Dir(src), name, InodeKind::File));
        assert_refused(&mut family, delta);
    }
}

#[test]
fn spans_that_are_huge_or_leave_the_clock_are_refused_quickly() {
    let (mut family, _, file) = family();
    let mut huge = from_mallory(&family, MAIN, Op::Text { inode: file, op: TextOp::Delete { pos: 0, len: 1 << 62 } });
    huge.span.len = 1 << 62;
    assert_refused(&mut family, huge);

    let mut wrapping = from_mallory(&family, MAIN, Op::Text { inode: file, op: TextOp::Delete { pos: usize::MAX, len: 1 } });
    wrapping.span.len = 1;
    assert_refused(&mut family, wrapping);

    for start in [u64::MAX, u64::MAX / 2, 1 << 50] {
        let mut late = from_mallory(&family, MAIN, Op::Conv(ConvOp::Title("x".into())));
        late.span = DeltaSpan::new(MALLORY, Lamport(start), 1);
        assert_refused(&mut family, late);
    }
    let next = write(&mut family, ALICE, MAIN, Op::Conv(ConvOp::Title("still works".into())));
    assert!(next.span.start.0 < 100);
}

#[test]
fn forks_and_merges_of_a_thread_into_itself_are_refused() {
    let (mut family, _, _) = family();
    let self_fork = from_mallory(&family, MAIN, Op::Fork { from: MAIN });
    assert_refused(&mut family, self_fork);
    let self_merge = from_mallory(&family, MAIN, Op::Merge { from: MAIN, mr: None });
    assert_refused(&mut family, self_merge);
    let late_fork = from_mallory(&family, MAIN, Op::Fork { from: FORK });
    assert_refused(&mut family, late_fork);
    let unknown_merge = from_mallory(&family, MAIN, Op::Merge { from: ThreadId(99), mr: None });
    assert_refused(&mut family, unknown_merge);
}

#[test]
fn threads_that_pull_from_each_other_have_a_finite_history() {
    let (mut family, _, _) = family();
    let head = family.thread_head(MAIN);
    let fork = family.delta(ALICE, FORK, Author::User, 0, head, Op::Fork { from: MAIN });
    assert_eq!(apply(&mut family, fork), Ok(Applied::New));
    write(&mut family, ALICE, FORK, Op::Conv(ConvOp::Title("fork".into())));
    let parents: Frontier = family.thread_head(MAIN).ids().iter().chain(family.thread_head(FORK).ids()).copied().collect();
    let pull = family.delta(ALICE, MAIN, Author::User, 0, parents, Op::Merge { from: FORK, mr: None });
    assert_eq!(apply(&mut family, pull), Ok(Applied::New));
    let parents: Frontier = family.thread_head(FORK).ids().iter().chain(family.thread_head(MAIN).ids()).copied().collect();
    let back = family.delta(ALICE, FORK, Author::User, 0, parents, Op::Merge { from: MAIN, mr: None });
    assert_eq!(apply(&mut family, back), Ok(Applied::New));
    assert!(!family.history(MAIN).is_empty());
    assert!(!family.history(FORK).is_empty());
}

#[test]
fn moves_into_directories_the_author_never_saw_are_refused() {
    let (mut family, _, file) = family();
    let before_dir = family.thread_head(MAIN);
    let dir = write(&mut family, ALICE, MAIN, create(Parent::Root, "lib", InodeKind::Dir));
    let unseen = family.delta(
        MALLORY,
        MAIN,
        Author::User,
        0,
        before_dir,
        Op::Move {
            inode: file,
            parent: Parent::Dir(InodeId(dir.span.first())),
            name: "a.txt".into(),
        },
    );
    assert_refused(&mut family, unseen);
}
