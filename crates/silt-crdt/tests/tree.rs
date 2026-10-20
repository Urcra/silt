//! Randomised schedules of concurrent creates, moves and deletes across
//! replicas. After full delivery every replica must have the same paths, no
//! inode may be lost or caught in a cycle, and every live file has one path.

use std::collections::HashSet;

use proptest::prelude::*;
use silt_crdt::{CausalGraph, FileTree, InodeId, InodeKind, Parent, TreeEvent, TreeOp};
use silt_types::{ActorId, DeltaId, DeltaSpan, Frontier};

#[derive(Debug, Clone)]
struct Replica {
    actor: ActorId,
    graph: CausalGraph,
    tree: FileTree,
    events: Vec<(TreeEvent, Frontier)>,
}

impl Replica {
    fn new(actor: ActorId) -> Self {
        Self {
            actor,
            graph: CausalGraph::new(),
            tree: FileTree::new(),
            events: Vec::new(),
        }
    }

    fn local(&mut self, op: TreeOp) {
        let id = DeltaId::new(self.actor, self.graph.next_lamport());
        let event = TreeEvent { id, op };
        let parents = self.graph.frontier().clone();
        self.receive(&event, &parents);
    }

    fn receive(&mut self, event: &TreeEvent, parents: &Frontier) {
        if self.graph.contains(event.id) {
            return;
        }
        self.graph
            .add(DeltaSpan::new(event.id.actor, event.id.lamport, 1), parents)
            .expect("causal delivery");
        self.tree.apply(event).expect("tree op applies");
        self.events.push((event.clone(), parents.clone()));
    }
}

#[derive(Debug, Clone)]
enum Action {
    Create { replica: usize, dir: bool, parent: u8, name: u8 },
    Move { replica: usize, inode: u8, parent: u8, name: u8 },
    Delete { replica: usize, inode: u8 },
    Deliver { from: usize, to: usize, fraction: u8 },
}

const NAMES: [&str; 3] = ["a", "b", "c"];

fn action(replicas: usize) -> impl Strategy<Value = Action> {
    prop_oneof![
        3 => (0..replicas, any::<bool>(), any::<u8>(), 0u8..3)
            .prop_map(|(replica, dir, parent, name)| Action::Create { replica, dir, parent, name }),
        4 => (0..replicas, any::<u8>(), any::<u8>(), 0u8..3)
            .prop_map(|(replica, inode, parent, name)| Action::Move { replica, inode, parent, name }),
        1 => (0..replicas, any::<u8>()).prop_map(|(replica, inode)| Action::Delete { replica, inode }),
        3 => (0..replicas, 0..replicas, any::<u8>())
            .prop_map(|(from, to, fraction)| Action::Deliver { from, to, fraction }),
    ]
}

fn pick_parent(tree: &FileTree, choice: u8) -> Parent {
    let dirs: Vec<_> = tree
        .inodes()
        .into_iter()
        .filter(|inode| tree.kind(*inode) == Some(InodeKind::Dir))
        .collect();
    match usize::from(choice) % (dirs.len() + 1) {
        0 => Parent::Root,
        index => Parent::Dir(dirs[index - 1]),
    }
}

fn deliver(from: &Replica, to: &mut Replica, count: usize) {
    for (event, parents) in from.events.iter().take(count) {
        to.receive(event, parents);
    }
}

fn run(replica_count: usize, actions: &[Action]) -> Result<(), TestCaseError> {
    let mut replicas: Vec<Replica> =
        (0..replica_count).map(|index| Replica::new(ActorId(index as u128 + 1))).collect();
    let mut created = 0;
    for action in actions {
        match action {
            Action::Create { replica, dir, parent, name } => {
                let replica = &mut replicas[*replica];
                let kind = if *dir { InodeKind::Dir } else { InodeKind::File };
                let parent = pick_parent(&replica.tree, *parent);
                replica.local(TreeOp::Create { kind, parent, name: NAMES[usize::from(*name)].into() });
                created += 1;
            }
            Action::Move { replica, inode, parent, name } => {
                let replica = &mut replicas[*replica];
                let inodes = replica.tree.inodes();
                if inodes.is_empty() {
                    continue;
                }
                let inode = inodes[usize::from(*inode) % inodes.len()];
                let parent = pick_parent(&replica.tree, *parent);
                replica.local(TreeOp::Move { inode, parent, name: NAMES[usize::from(*name)].into() });
            }
            Action::Delete { replica, inode } => {
                let replica = &mut replicas[*replica];
                let inodes = replica.tree.inodes();
                if inodes.is_empty() {
                    continue;
                }
                let inode = inodes[usize::from(*inode) % inodes.len()];
                replica.local(TreeOp::Delete { inode });
            }
            Action::Deliver { from, to, fraction } => {
                if from == to {
                    continue;
                }
                let source = replicas[*from].clone();
                let count = source.events.len() * usize::from(*fraction) / 255;
                deliver(&source, &mut replicas[*to], count);
            }
        }
        for replica in &replicas {
            prop_assert!(replica.tree.is_well_formed(), "cycle or orphan mid-run");
        }
    }

    let snapshot = replicas.clone();
    for replica in &mut replicas {
        for other in snapshot.iter().rev() {
            deliver(other, replica, other.events.len());
        }
    }

    let paths = replicas[0].tree.paths();
    for replica in &replicas {
        prop_assert_eq!(&replica.tree.paths(), &paths);
        prop_assert!(replica.tree.is_well_formed());
        prop_assert_eq!(replica.tree.inodes().len(), created, "inode lost");
        let tree = &replica.tree;
        let live: Vec<InodeId> = tree.inodes().into_iter().filter(|inode| tree.is_live(*inode)).collect();
        let is_dir = |inode: &InodeId| tree.kind(*inode) == Some(InodeKind::Dir);
        let live_files = live.iter().filter(|inode| !is_dir(inode)).count();
        prop_assert_eq!(live_files, paths.values().filter(|inode| !is_dir(inode)).count(), "every live file has exactly one path");
        prop_assert!(paths.values().all(|inode| tree.is_live(*inode)), "every path names a live inode");
        // Directories made at once under one name are listed as one, so some
        // live directories have no path of their own; their files do.
        let listed: HashSet<InodeId> = paths.values().copied().collect();
        prop_assert_eq!(listed.len(), paths.len(), "no inode has two paths");
        prop_assert!(listed.iter().filter(|inode| is_dir(inode)).count() <= live.iter().filter(|inode| is_dir(inode)).count());
    }
    Ok(())
}

proptest! {
    #[test]
    fn trees_converge_without_cycles_or_lost_inodes(
        (replicas, actions) in (2usize..=4).prop_flat_map(|replicas| {
            (Just(replicas), prop::collection::vec(action(replicas), 1..60))
        })
    ) {
        run(replicas, &actions)?;
    }
}
