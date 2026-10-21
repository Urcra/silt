//! Random deltas, many of them malformed or hostile (ADR 0016): a family
//! never panics on them, a refused one changes nothing, and the deltas it
//! took replay to the same family anywhere.

use std::collections::HashMap;

use proptest::prelude::*;
use silt_crdt::{InodeId, InodeKind, Parent, TextOp};
use silt_log::{Author, Content, ConvOp, Delta, Family, Op};
use silt_types::{ActorId, DeltaId, DeltaSpan, Frontier, GitOid, Lamport, ThreadId};

const THREADS: [ThreadId; 2] = [ThreadId(7), ThreadId(8)];
const NAMES: [&str; 9] = ["a", "b", "src", ".git", ".GIT", "..", "x/y", "", "café"];

/// One delta to try, with choices resolved against what was taken so far.
#[derive(Debug, Clone)]
struct Step {
    actor: u8,
    thread: usize,
    parents: Parents,
    lamport: Option<u64>,
    len: Option<u64>,
    op: OpChoice,
}

#[derive(Debug, Clone)]
enum Parents {
    Head,
    Root,
    Taken(usize),
    Unknown(u64),
}

#[derive(Debug, Clone)]
enum OpChoice {
    Create { dir: bool, parent: Option<usize>, name: usize, git: bool },
    Move { inode: usize, parent: Option<usize>, name: usize },
    Delete { inode: usize },
    Insert { inode: usize, pos: usize, text: String },
    Remove { inode: usize, pos: usize, len: usize },
    Executable { inode: usize },
    Title(String),
    Fork,
    Merge,
}

fn big() -> impl Strategy<Value = u64> {
    prop_oneof![0..64u64, Just(u64::MAX), Just(i64::MAX as u64), any::<u64>()]
}

fn step() -> impl Strategy<Value = Step> {
    let parents = prop_oneof![
        4 => Just(Parents::Head),
        1 => Just(Parents::Root),
        2 => any::<usize>().prop_map(Parents::Taken),
        1 => big().prop_map(Parents::Unknown),
    ];
    let position = prop_oneof![6 => 0..8usize, 1 => Just(usize::MAX), 1 => any::<usize>()];
    // Mostly names that are allowed, so families grow deep enough to matter.
    let name = || prop_oneof![6 => 0..3usize, 1 => 3..NAMES.len()];
    let op = prop_oneof![
        3 => (any::<bool>(), proptest::option::of(any::<usize>()), name(), any::<bool>())
            .prop_map(|(dir, parent, name, git)| OpChoice::Create { dir, parent, name, git }),
        2 => (any::<usize>(), proptest::option::of(any::<usize>()), name())
            .prop_map(|(inode, parent, name)| OpChoice::Move { inode, parent, name }),
        1 => any::<usize>().prop_map(|inode| OpChoice::Delete { inode }),
        3 => (any::<usize>(), position.clone(), "[a-zé\n]{0,6}").prop_map(|(inode, pos, text)| OpChoice::Insert { inode, pos, text }),
        2 => (any::<usize>(), position.clone(), position).prop_map(|(inode, pos, len)| OpChoice::Remove { inode, pos, len }),
        1 => any::<usize>().prop_map(|inode| OpChoice::Executable { inode }),
        1 => "[a-z ]{0,8}".prop_map(OpChoice::Title),
        1 => Just(OpChoice::Fork),
        1 => Just(OpChoice::Merge),
    ];
    (
        0..4u8,
        0..THREADS.len(),
        parents,
        prop_oneof![8 => Just(None), 1 => big().prop_map(Some)],
        prop_oneof![10 => Just(None), 1 => big().prop_map(Some)],
        op,
    )
        .prop_map(|(actor, thread, parents, lamport, len, op)| Step { actor, thread, parents, lamport, len, op })
}

/// Picks one of `taken` by `index`, or an id nothing has when there are none.
fn pick(taken: &[DeltaId], index: usize) -> DeltaId {
    if taken.is_empty() { DeltaId::new(ActorId(99), Lamport(index as u64)) } else { taken[index % taken.len()] }
}

fn delta(family: &Family, taken: &[DeltaId], inodes: &[DeltaId], step: &Step) -> Delta {
    let thread = THREADS[step.thread];
    let other = THREADS[1 - step.thread];
    let inode = |index: usize| InodeId(pick(inodes, index));
    let parent = |choice: &Option<usize>| match choice {
        None => Parent::Root,
        Some(index) if index % 5 == 0 => Parent::Trash,
        Some(index) => Parent::Dir(inode(*index)),
    };
    let op = match &step.op {
        OpChoice::Create { dir, parent: at, name, git } => Op::Create {
            kind: if *dir { InodeKind::Dir } else { InodeKind::File },
            parent: parent(at),
            name: NAMES[*name].to_owned(),
            content: if *git { Content::GitText(GitOid([1; 20])) } else { Content::Empty },
            executable: false,
        },
        OpChoice::Move { inode: moved, parent: at, name } => {
            Op::Move { inode: inode(*moved), parent: parent(at), name: NAMES[*name].to_owned() }
        }
        OpChoice::Delete { inode: deleted } => Op::Delete { inode: inode(*deleted) },
        OpChoice::Insert { inode: edited, pos, text } => {
            Op::Text { inode: inode(*edited), op: TextOp::Insert { pos: *pos, content: text.clone() } }
        }
        OpChoice::Remove { inode: edited, pos, len } => {
            Op::Text { inode: inode(*edited), op: TextOp::Delete { pos: *pos, len: *len } }
        }
        OpChoice::Executable { inode: changed } => Op::SetExecutable { inode: inode(*changed), executable: true },
        OpChoice::Title(title) => Op::Conv(ConvOp::Title(title.clone())),
        OpChoice::Fork => Op::Fork { from: other },
        OpChoice::Merge => Op::Merge { from: other, mr: None },
    };
    let parents = match step.parents {
        Parents::Head => family.thread_head(thread),
        Parents::Root => Frontier::root(),
        Parents::Taken(index) => Frontier::single(pick(taken, index)),
        Parents::Unknown(lamport) => Frontier::single(DeltaId::new(ActorId(77), Lamport(lamport))),
    };
    let actor = ActorId(u128::from(step.actor) + 1);
    let mut delta = family.delta(actor, thread, Author::User, 0, parents, op);
    let start = step.lamport.map_or(delta.span.start, Lamport);
    delta.span = DeltaSpan::new(actor, start, step.len.unwrap_or(delta.span.len));
    delta
}

/// What two families must agree on.
fn state(family: &Family) -> (Vec<(String, InodeId)>, Vec<String>) {
    let paths: Vec<(String, InodeId)> = family.tree().paths().into_iter().collect();
    let files = paths.iter().map(|(_, inode)| format!("{:?}", family.file(*inode))).collect();
    (paths, files)
}

fn bases() -> HashMap<GitOid, String> {
    HashMap::from([(GitOid([1; 20]), "fn main() {}\n".to_owned())])
}

proptest! {
    #[test]
    fn random_deltas_are_taken_or_refused_cleanly(steps in proptest::collection::vec(step(), 1..60)) {
        let mut family = Family::new();
        let mut taken: Vec<DeltaId> = Vec::new();
        let mut inodes: Vec<DeltaId> = Vec::new();
        for step in &steps {
            let delta = delta(&family, &taken, &inodes, step);
            let before = (family.log().len(), family.graph().frontier().clone(), state(&family));
            match family.apply(delta.clone(), &mut bases()) {
                Ok(_) => {
                    if family.log().len() > before.0 {
                        taken.push(delta.span.last());
                        if let Op::Create { .. } = delta.op {
                            inodes.push(delta.span.first());
                        }
                    }
                }
                Err(_) => {
                    prop_assert_eq!(family.log().len(), before.0, "a refused delta isn't recorded");
                    prop_assert_eq!(family.graph().frontier(), &before.1);
                    prop_assert_eq!(state(&family), before.2, "nor does it change the files");
                }
            }
        }
        for thread in THREADS {
            for path in family.thread_view(thread).paths().keys() {
                prop_assert!(path.split('/').all(silt_crdt::is_valid_name), "{} could lead outside a checkout", path);
            }
        }
        let mut replayed = Family::new();
        for delta in family.log() {
            prop_assert!(replayed.apply(delta.clone(), &mut bases()).is_ok(), "{:?}", delta);
        }
        prop_assert_eq!(state(&replayed), state(&family));
        for thread in THREADS {
            let paths = |family: &Family| family.thread_view(thread).paths().keys().cloned().collect::<Vec<_>>();
            prop_assert_eq!(paths(&replayed), paths(&family));
        }
    }

    /// Stored deltas, refused ones too, give the same family whether
    /// replayed or read from a snapshot taken partway and the rest applied.
    #[test]
    fn a_snapshot_and_what_follows_is_a_replay(steps in proptest::collection::vec(step(), 1..60), cut in any::<prop::sample::Index>()) {
        let mut family = Family::new();
        let mut taken: Vec<DeltaId> = Vec::new();
        let mut inodes: Vec<DeltaId> = Vec::new();
        let mut stored = Vec::new();
        for step in &steps {
            let delta = delta(&family, &taken, &inodes, step);
            let before = family.log().len();
            if family.apply(delta.clone(), &mut bases()).is_ok() && family.log().len() > before {
                taken.push(delta.span.last());
                if let Op::Create { .. } = delta.op {
                    inodes.push(delta.span.first());
                }
            }
            stored.push(delta);
        }
        let replay = |deltas: &[Delta], family: &mut Family| {
            for delta in deltas {
                let _ = family.apply(delta.clone(), &mut bases());
            }
        };
        let cut = cut.index(stored.len() + 1);
        let mut partway = Family::new();
        replay(&stored[..cut], &mut partway);
        let snapshot = partway.snapshot().expect("encodes");
        let (mut restored, rest) = Family::from_snapshot(&snapshot, &stored).expect("matches");
        replay(rest, &mut restored);
        let mut whole = Family::new();
        replay(&stored, &mut whole);

        prop_assert_eq!(restored.log(), whole.log());
        prop_assert_eq!(state(&restored), state(&whole));
        // Neither type compares, and their debug output shows every field.
        let threads = |family: &Family| format!("{:?}", family.threads().collect::<Vec<_>>());
        prop_assert_eq!(threads(&restored), threads(&whole));
        prop_assert_eq!(format!("{:?}", restored.graph()), format!("{:?}", whole.graph()));
        for thread in THREADS {
            let view = |family: &Family| {
                let history: Vec<DeltaId> = family.history(thread).iter().map(|item| item.delta.span.first()).collect();
                (family.thread_view(thread).paths().keys().cloned().collect::<Vec<_>>(), family.thread_head(thread), history)
            };
            prop_assert_eq!(view(&restored), view(&whole));
        }
    }
}
