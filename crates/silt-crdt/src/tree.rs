use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

use silt_types::{ActorId, DeltaId, Lamport};

/// A file or directory, named by the delta that created it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct InodeId(pub DeltaId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InodeKind {
    Dir,
    File,
    Symlink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Parent {
    Root,
    Trash,
    Dir(InodeId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TreeOp {
    Create {
        kind: InodeKind,
        parent: Parent,
        name: String,
    },
    Move {
        inode: InodeId,
        parent: Parent,
        name: String,
    },
    Delete {
        inode: InodeId,
    },
}

/// One tree operation. A `Create` makes the inode `InodeId(id)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeEvent {
    pub id: DeltaId,
    pub op: TreeOp,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TreeError {
    #[error("inode {0:?} does not exist")]
    UnknownInode(InodeId),
    #[error("name {0:?} is not a valid file name")]
    InvalidName(String),
}

/// Total order on operations that agrees with causality: a later operation
/// always has a higher lamport, and the actor breaks ties between concurrent
/// ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
struct Stamp(Lamport, ActorId);

impl From<DeltaId> for Stamp {
    fn from(id: DeltaId) -> Self {
        Self(id.lamport, id.actor)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Placement {
    parent: Parent,
    name: String,
    placed_by: Stamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LogEntry {
    stamp: Stamp,
    inode: InodeId,
    target: Placement,
    previous: Option<Placement>,
}

/// The replicated directory structure of a worktree.
///
/// Uses the move operation of Kleppmann et al., "A highly-available move
/// operation for replicated trees": operations are applied in stamp order,
/// undoing and redoing later ones when an earlier one arrives late, and a move
/// that would put a directory inside itself is skipped. Deleting is a move to
/// the trash, so a concurrent edit or move never loses an inode.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileTree {
    kinds: HashMap<InodeId, InodeKind>,
    placements: HashMap<InodeId, Placement>,
    log: Vec<LogEntry>,
}

impl FileTree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn kind(&self, inode: InodeId) -> Option<InodeKind> {
        self.kinds.get(&inode).copied()
    }

    /// Every inode ever created, live or trashed, in id order.
    pub fn inodes(&self) -> Vec<InodeId> {
        let mut inodes: Vec<InodeId> = self.kinds.keys().copied().collect();
        inodes.sort();
        inodes
    }

    /// Whether the inode is reachable from the root rather than the trash.
    pub fn is_live(&self, inode: InodeId) -> bool {
        let mut current = inode;
        for _ in 0..=self.placements.len() {
            match self.placements.get(&current).map(|placement| placement.parent) {
                Some(Parent::Root) => return true,
                Some(Parent::Dir(parent)) => current = parent,
                Some(Parent::Trash) | None => return false,
            }
        }
        false
    }

    /// Checks that `event` would apply, without applying it.
    pub fn check(&self, event: &TreeEvent) -> Result<(), TreeError> {
        match &event.op {
            TreeOp::Create { name, .. } => {
                if !is_valid_name(name) {
                    return Err(TreeError::InvalidName(name.clone()));
                }
            }
            TreeOp::Move { inode, parent, name } => {
                if !self.kinds.contains_key(inode) {
                    return Err(TreeError::UnknownInode(*inode));
                }
                if *parent != Parent::Trash && !is_valid_name(name) {
                    return Err(TreeError::InvalidName(name.clone()));
                }
            }
            TreeOp::Delete { inode } => {
                if !self.kinds.contains_key(inode) {
                    return Err(TreeError::UnknownInode(*inode));
                }
            }
        }
        Ok(())
    }

    pub fn apply(&mut self, event: &TreeEvent) -> Result<(), TreeError> {
        let stamp = Stamp::from(event.id);
        if self.log.binary_search_by(|entry| entry.stamp.cmp(&stamp)).is_ok() {
            return Ok(());
        }
        let (inode, parent, name) = match &event.op {
            TreeOp::Create { kind, parent, name } => {
                if !is_valid_name(name) {
                    return Err(TreeError::InvalidName(name.clone()));
                }
                let inode = InodeId(event.id);
                self.kinds.insert(inode, *kind);
                (inode, *parent, name.clone())
            }
            TreeOp::Move { inode, parent, name } => (*inode, *parent, name.clone()),
            TreeOp::Delete { inode } => {
                let name = self
                    .placements
                    .get(inode)
                    .map(|placement| placement.name.clone())
                    .unwrap_or_default();
                (*inode, Parent::Trash, name)
            }
        };
        if !self.kinds.contains_key(&inode) {
            return Err(TreeError::UnknownInode(inode));
        }
        if parent != Parent::Trash && !is_valid_name(&name) {
            return Err(TreeError::InvalidName(name));
        }
        let target = Placement {
            parent,
            name,
            placed_by: stamp,
        };
        self.apply_in_order(stamp, inode, target);
        Ok(())
    }

    /// The tree as of the version containing exactly the ids for which
    /// `contains` holds: the move log replayed without later operations.
    pub fn at(&self, contains: impl Fn(DeltaId) -> bool) -> FileTree {
        let mut tree = FileTree {
            kinds: self
                .kinds
                .iter()
                .filter(|(inode, _)| contains(inode.0))
                .map(|(inode, kind)| (*inode, *kind))
                .collect(),
            placements: HashMap::new(),
            log: Vec::new(),
        };
        for entry in &self.log {
            let id = DeltaId::new(entry.stamp.1, entry.stamp.0);
            if contains(id) && tree.kinds.contains_key(&entry.inode) {
                let redone = tree.do_move(entry.stamp, entry.inode, entry.target.clone());
                tree.log.push(redone);
            }
        }
        tree
    }

    /// Every live file, symlink and directory by path. Names claimed by more
    /// than one inode keep the name for the earliest placement; the others get
    /// a deterministic ` (conflict <inode id>)` suffix.
    pub fn paths(&self) -> BTreeMap<String, InodeId> {
        let mut paths = BTreeMap::new();
        self.collect_paths(Parent::Root, "", &mut paths);
        paths
    }

    pub fn path_of(&self, inode: InodeId) -> Option<String> {
        self.paths()
            .into_iter()
            .find_map(|(path, found)| (found == inode).then_some(path))
    }

    /// Whether every inode is placed and reaches the root or the trash without
    /// passing through itself.
    pub fn is_well_formed(&self) -> bool {
        self.kinds.keys().all(|inode| {
            let mut current = *inode;
            for _ in 0..=self.placements.len() {
                match self.placements.get(&current).map(|placement| placement.parent) {
                    Some(Parent::Root | Parent::Trash) => return true,
                    Some(Parent::Dir(parent)) => current = parent,
                    None => return false,
                }
            }
            false
        })
    }

    fn apply_in_order(&mut self, stamp: Stamp, inode: InodeId, target: Placement) {
        let index = self.log.partition_point(|entry| entry.stamp < stamp);
        let later: Vec<LogEntry> = self.log.drain(index..).collect();
        for entry in later.iter().rev() {
            self.undo(entry);
        }
        let entry = self.do_move(stamp, inode, target);
        self.log.push(entry);
        for entry in later {
            let redone = self.do_move(entry.stamp, entry.inode, entry.target);
            self.log.push(redone);
        }
    }

    fn do_move(&mut self, stamp: Stamp, inode: InodeId, target: Placement) -> LogEntry {
        let previous = self.placements.get(&inode).cloned();
        if self.allowed(inode, &target) {
            self.placements.insert(inode, target.clone());
        }
        LogEntry {
            stamp,
            inode,
            target,
            previous,
        }
    }

    fn undo(&mut self, entry: &LogEntry) {
        match &entry.previous {
            Some(previous) => self.placements.insert(entry.inode, previous.clone()),
            None => self.placements.remove(&entry.inode),
        };
    }

    fn allowed(&self, inode: InodeId, target: &Placement) -> bool {
        let Parent::Dir(parent) = target.parent else {
            return true;
        };
        if self.kinds.get(&parent) != Some(&InodeKind::Dir) {
            return false;
        }
        !self.is_ancestor_or_self(inode, parent)
    }

    fn is_ancestor_or_self(&self, ancestor: InodeId, of: InodeId) -> bool {
        let mut current = of;
        for _ in 0..=self.placements.len() {
            if current == ancestor {
                return true;
            }
            match self.placements.get(&current).map(|placement| placement.parent) {
                Some(Parent::Dir(parent)) => current = parent,
                _ => return false,
            }
        }
        true
    }

    fn collect_paths(&self, dir: Parent, prefix: &str, paths: &mut BTreeMap<String, InodeId>) {
        let mut children: Children = HashMap::new();
        for (inode, placement) in &self.placements {
            children
                .entry(placement.parent)
                .or_default()
                .entry(placement.name.as_str())
                .or_default()
                .push((placement.placed_by, *inode));
        }
        self.collect_under(&mut children, &[dir], prefix, paths);
    }

    /// Lists what is in `dirs`, which are one directory: directories created
    /// under one name in one place, concurrently, hold their creators' files
    /// together rather than one of them being renamed.
    fn collect_under(
        &self,
        children: &mut Children<'_>,
        dirs: &[Parent],
        prefix: &str,
        paths: &mut BTreeMap<String, InodeId>,
    ) {
        let mut by_name: BTreeMap<&str, Vec<(Stamp, InodeId)>> = BTreeMap::new();
        for dir in dirs {
            for (name, claimants) in children.remove(dir).unwrap_or_default() {
                by_name.entry(name).or_default().extend(claimants);
            }
        }
        let mut taken: HashSet<String> = by_name.keys().map(|name| (*name).to_owned()).collect();
        for (name, claimants) in by_name {
            for (rank, entry) in entries(claimants, &self.kinds).into_iter().enumerate() {
                let first = entry[0];
                let name = if rank == 0 { name.to_owned() } else { conflict_name(name, first, &mut taken) };
                let path = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
                if self.kinds.get(&first) == Some(&InodeKind::Dir) {
                    let merged: Vec<Parent> = entry.iter().map(|inode| Parent::Dir(*inode)).collect();
                    self.collect_under(children, &merged, &path, paths);
                }
                paths.insert(path, first);
            }
        }
    }
}

/// The things claiming one name, earliest first: each file on its own, and
/// all directories together as one.
fn entries(mut claimants: Vec<(Stamp, InodeId)>, kinds: &HashMap<InodeId, InodeKind>) -> Vec<Vec<InodeId>> {
    claimants.sort();
    let mut entries: Vec<Vec<InodeId>> = Vec::new();
    let mut dirs: Option<usize> = None;
    for (_, inode) in claimants {
        if kinds.get(&inode) != Some(&InodeKind::Dir) {
            entries.push(vec![inode]);
            continue;
        }
        match dirs {
            Some(index) => entries[index].push(inode),
            None => {
                dirs = Some(entries.len());
                entries.push(vec![inode]);
            }
        }
    }
    entries
}

/// A name for a claimant that lost `name`, unlike any other in the
/// directory. Every replica sees the same names in the same order, so they
/// all pick the same one.
fn conflict_name(name: &str, inode: InodeId, taken: &mut HashSet<String>) -> String {
    let base = format!("{name} (conflict {})", inode.0);
    let mut candidate = base.clone();
    let mut again = 1;
    while !taken.insert(candidate.clone()) {
        again += 1;
        candidate = format!("{base} {again}");
    }
    candidate
}

/// Each directory's entries by name, with every inode claiming the name.
type Children<'a> = HashMap<Parent, BTreeMap<&'a str, Vec<(Stamp, InodeId)>>>;

/// Whether `name` may name a file or directory: no separators, no `.` or
/// `..`, nothing git would treat as its own directory.
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\0']) && !is_git_dir_name(name)
}

/// Names git itself refuses to track because a filesystem may treat them as
/// `.git`: any case, trailing dots or spaces (NTFS), and the 8.3 short name.
/// A peer must never be able to place files in a repository's git
/// directory, where hooks and config run code.
pub fn is_git_dir_name(name: &str) -> bool {
    let name = name.trim_end_matches(['.', ' ']).to_ascii_lowercase();
    name == ".git" || name == "git~1"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(actor: u128, lamport: u64) -> DeltaId {
        DeltaId::new(ActorId(actor), Lamport(lamport))
    }

    fn create(id: DeltaId, kind: InodeKind, parent: Parent, name: &str) -> TreeEvent {
        TreeEvent {
            id,
            op: TreeOp::Create {
                kind,
                parent,
                name: name.into(),
            },
        }
    }

    fn mv(id: DeltaId, inode: DeltaId, parent: Parent, name: &str) -> TreeEvent {
        TreeEvent {
            id,
            op: TreeOp::Move {
                inode: InodeId(inode),
                parent,
                name: name.into(),
            },
        }
    }

    fn apply_all(events: &[TreeEvent]) -> FileTree {
        let mut tree = FileTree::new();
        for event in events {
            tree.apply(event).expect("applies");
        }
        tree
    }

    fn path_list(tree: &FileTree) -> Vec<String> {
        tree.paths().into_keys().collect()
    }

    #[test]
    fn creates_nested_paths() {
        let tree = apply_all(&[
            create(id(1, 0), InodeKind::Dir, Parent::Root, "src"),
            create(id(1, 1), InodeKind::File, Parent::Dir(InodeId(id(1, 0))), "main.rs"),
            create(id(1, 2), InodeKind::File, Parent::Root, "README.md"),
        ]);
        assert_eq!(path_list(&tree), vec!["README.md", "src", "src/main.rs"]);
    }

    #[test]
    fn concurrent_moves_of_two_dirs_into_each_other_never_cycle() {
        let base = [
            create(id(1, 0), InodeKind::Dir, Parent::Root, "a"),
            create(id(1, 1), InodeKind::Dir, Parent::Root, "b"),
        ];
        let a_into_b = mv(id(1, 2), id(1, 0), Parent::Dir(InodeId(id(1, 1))), "a");
        let b_into_a = mv(id(2, 2), id(1, 1), Parent::Dir(InodeId(id(1, 0))), "b");

        let mut first = apply_all(&base);
        first.apply(&a_into_b).expect("applies");
        first.apply(&b_into_a).expect("applies");
        let mut second = apply_all(&base);
        second.apply(&b_into_a).expect("applies");
        second.apply(&a_into_b).expect("applies");

        assert!(first.is_well_formed());
        assert_eq!(first.paths(), second.paths());
        assert_eq!(path_list(&first), vec!["b", "b/a"]);
    }

    #[test]
    fn delete_moves_to_trash_and_hides_the_subtree() {
        let tree = apply_all(&[
            create(id(1, 0), InodeKind::Dir, Parent::Root, "old"),
            create(id(1, 1), InodeKind::File, Parent::Dir(InodeId(id(1, 0))), "x"),
            TreeEvent {
                id: id(1, 2),
                op: TreeOp::Delete {
                    inode: InodeId(id(1, 0)),
                },
            },
        ]);
        assert!(tree.paths().is_empty());
        assert!(tree.is_well_formed());
    }

    #[test]
    fn name_conflicts_keep_the_earliest_placement() {
        let tree = apply_all(&[
            create(id(2, 5), InodeKind::File, Parent::Root, "lib.rs"),
            create(id(1, 7), InodeKind::File, Parent::Root, "lib.rs"),
        ]);
        let paths = tree.paths();
        assert_eq!(paths["lib.rs"], InodeId(id(2, 5)));
        assert_eq!(
            paths["lib.rs (conflict 00000000000000000000000000000001.7)"],
            InodeId(id(1, 7))
        );
    }

    #[test]
    fn directories_made_at_once_under_one_name_are_one_directory() {
        let a = InodeId(id(1, 0));
        let b = InodeId(id(2, 0));
        let tree = apply_all(&[
            create(id(1, 0), InodeKind::Dir, Parent::Root, "src"),
            create(id(2, 0), InodeKind::Dir, Parent::Root, "src"),
            create(id(1, 1), InodeKind::File, Parent::Dir(a), "a.rs"),
            create(id(2, 1), InodeKind::File, Parent::Dir(b), "b.rs"),
            create(id(1, 2), InodeKind::File, Parent::Dir(a), "same.rs"),
            create(id(2, 2), InodeKind::File, Parent::Dir(b), "same.rs"),
        ]);
        let paths = tree.paths();
        assert_eq!(paths["src"], a);
        assert_eq!(paths["src/a.rs"], InodeId(id(1, 1)));
        assert_eq!(paths["src/b.rs"], InodeId(id(2, 1)));
        assert_eq!(paths["src/same.rs"], InodeId(id(1, 2)));
        assert_eq!(paths["src/same.rs (conflict 00000000000000000000000000000002.2)"], InodeId(id(2, 2)));
        assert_eq!(paths.len(), 5, "{paths:?}");
    }

    #[test]
    fn a_conflict_name_never_hides_a_real_file() {
        let tree = apply_all(&[
            create(id(2, 5), InodeKind::File, Parent::Root, "lib.rs"),
            create(id(1, 7), InodeKind::File, Parent::Root, "lib.rs"),
            create(id(3, 9), InodeKind::File, Parent::Root, "lib.rs (conflict 00000000000000000000000000000001.7)"),
        ]);
        let paths = tree.paths();
        assert_eq!(paths.len(), 3, "{paths:?}");
        assert_eq!(paths["lib.rs (conflict 00000000000000000000000000000001.7)"], InodeId(id(3, 9)));
        assert_eq!(paths["lib.rs (conflict 00000000000000000000000000000001.7) 2"], InodeId(id(1, 7)));
    }

    #[test]
    fn moves_into_files_are_ignored() {
        let tree = apply_all(&[
            create(id(1, 0), InodeKind::File, Parent::Root, "f"),
            create(id(1, 1), InodeKind::File, Parent::Root, "g"),
            mv(id(1, 2), id(1, 1), Parent::Dir(InodeId(id(1, 0))), "g"),
        ]);
        assert_eq!(path_list(&tree), vec!["f", "g"]);
    }

    #[test]
    fn rejects_unknown_inodes_and_bad_names() {
        let mut tree = FileTree::new();
        assert_eq!(
            tree.apply(&mv(id(1, 1), id(9, 9), Parent::Root, "x")),
            Err(TreeError::UnknownInode(InodeId(id(9, 9))))
        );
        assert_eq!(
            tree.apply(&create(id(1, 0), InodeKind::File, Parent::Root, "a/b")),
            Err(TreeError::InvalidName("a/b".into()))
        );
    }

    #[test]
    fn earlier_versions_leave_out_later_operations() {
        let tree = apply_all(&[
            create(id(1, 0), InodeKind::Dir, Parent::Root, "a"),
            create(id(1, 1), InodeKind::File, Parent::Root, "f"),
            mv(id(1, 2), id(1, 1), Parent::Dir(InodeId(id(1, 0))), "g"),
            create(id(2, 3), InodeKind::File, Parent::Root, "late"),
        ]);
        assert_eq!(path_list(&tree), vec!["a", "a/g", "late"]);
        let before_move = tree.at(|id| id.lamport.0 <= 1);
        assert_eq!(path_list(&before_move), vec!["a", "f"]);
        let without_actor_2 = tree.at(|id| id.actor == ActorId(1));
        assert_eq!(path_list(&without_actor_2), vec!["a", "a/g"]);
    }

    #[test]
    fn redelivery_is_idempotent() {
        let event = create(id(1, 0), InodeKind::File, Parent::Root, "f");
        let mut tree = FileTree::new();
        tree.apply(&event).expect("applies");
        tree.apply(&event).expect("applies again");
        assert_eq!(path_list(&tree), vec!["f"]);
    }

    #[test]
    fn git_directories_are_not_names() {
        for name in [".git", ".GIT", ".Git", ".git.", ".git ", "GIT~1"] {
            assert!(!is_valid_name(name), "{name}");
        }
        for name in [".gitignore", ".github", "git", "x.git"] {
            assert!(is_valid_name(name), "{name}");
        }
    }
}
