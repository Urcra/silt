use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use silt_crdt::{InodeId, InodeKind, Parent};
use silt_log::{Author, BaseText, Content, Delta, Family, FileView, LogError, Op, ViewKey};
use silt_types::{ActorId, BlobHash, Frontier, GitOid, ThreadId};

use crate::diff::text_edits;
use crate::policy::TEXT_LIMIT;

/// What the checkout last read from or wrote to disk for one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskEntry {
    pub inode: InodeId,
    /// Hash of the bytes on disk, to recognise our own writes.
    pub hash: BlobHash,
    pub size: u64,
    pub mtime_ns: i64,
    /// A family version whose content of this file is what's on disk.
    pub version: Frontier,
}

/// A file as found on disk, already converted to stored form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskFile {
    pub content: DiskContent,
    pub executable: bool,
    pub hash: BlobHash,
    pub size: u64,
    pub mtime_ns: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskContent {
    /// Git-canonical text.
    Text(String),
    /// Content kept whole, already put in the blob store.
    Blob(BlobHash),
    /// A git blob that isn't text, written byte for byte.
    GitBlob(GitOid),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskAction {
    Write {
        path: String,
        inode: InodeId,
        content: DiskContent,
        executable: bool,
        version: Frontier,
        /// Where the file was before it moved, removed only once it is
        /// written at `path`, so a failed write never loses it.
        moved_from: Option<String>,
    },
    Remove {
        path: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ingested {
    pub deltas: Vec<Delta>,
    /// Writes needed because the family merged in changes the disk lacks.
    pub actions: Vec<DiskAction>,
}

/// What [`Bridge::forget`] took out of the thread.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Forgotten {
    pub deltas: Vec<Delta>,
    pub paths: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error(transparent)]
    Log(#[from] LogError),
    #[error("{0} is a directory in the family but a file on disk")]
    DirectoryConflict(String),
}

/// Keeps one checkout and its thread's family in step (ADR 0008).
///
/// Disk changes become deltas based on the version the disk last matched,
/// so they merge with concurrent changes instead of overwriting them. Each
/// file's text edits come from an actor derived from the checkout and the
/// file, which keeps every actor's deltas a causal chain even though files
/// are based on different versions.
#[derive(Debug, Clone)]
pub struct Bridge {
    actor: ActorId,
    thread: ThreadId,
    entries: BTreeMap<String, DiskEntry>,
    /// The thread's paths as last listed, and the view they were listed in.
    /// Listing them walks the whole tree, so recording a batch of new files
    /// listed them again for each: in a checkout of 50,000 files, 35 ms a
    /// file. What this bridge itself adds or removes is patched in instead.
    paths: Option<(ViewKey, Arc<BTreeMap<String, InodeId>>)>,
}

impl Bridge {
    pub fn new(actor: ActorId, thread: ThreadId, entries: BTreeMap<String, DiskEntry>) -> Self {
        Self { actor, thread, entries, paths: None }
    }

    /// The thread's paths in `family` now.
    fn thread_paths(&mut self, family: &Family) -> Arc<BTreeMap<String, InodeId>> {
        let key = family.view_key(self.thread);
        if let Some((listed, paths)) = &self.paths
            && *listed == key
        {
            return Arc::clone(paths);
        }
        let paths = family.thread_view(self.thread).paths();
        self.paths = Some((key, Arc::clone(&paths)));
        paths
    }

    /// Keeps the paths listed up to date with what one `ingest` changed in
    /// the tree, if that's all that changed since they were listed and none
    /// of it can rename another path; else they're listed again when next
    /// asked for.
    fn patch_paths(&mut self, family: &Family, changes_before: u64, tree_ops: u64, listed: Vec<Listed>) {
        let Some((_, mut paths)) = self.paths.take() else {
            return;
        };
        if family.tree_changes() != changes_before + tree_ops {
            return;
        }
        let map = Arc::make_mut(&mut paths);
        for change in listed {
            match change {
                Listed::Added(path, inode) => {
                    if map.insert(path, inode).is_some() {
                        return;
                    }
                }
                // A name another inode shares goes to it once this one's gone.
                Listed::Removed(path) => {
                    let shared = format!("{path} (conflict ");
                    if map.range(shared.clone()..).next().is_some_and(|(other, _)| other.starts_with(&shared)) {
                        return;
                    }
                    map.remove(&path);
                }
            }
        }
        self.paths = Some((family.view_key(self.thread), paths));
    }

    pub fn actor(&self) -> ActorId {
        self.actor
    }

    pub fn entries(&self) -> &BTreeMap<String, DiskEntry> {
        &self.entries
    }

    /// Whether this checkout made `delta`, so its effect is already on disk.
    pub fn is_own(&self, delta: &Delta) -> bool {
        match &delta.op {
            Op::Text { inode, .. } => delta.span.actor == file_actor(self.actor, *inode),
            _ => delta.span.actor == self.actor,
        }
    }

    /// The file at `path` changed on disk (`None`: it's gone). Applies the
    /// matching deltas to `family`.
    pub fn ingest(
        &mut self,
        family: &mut Family,
        bases: &mut dyn BaseText,
        path: &str,
        disk: Option<DiskFile>,
        author: &Author,
        time_ms: i64,
    ) -> Result<Ingested, BridgeError> {
        let paths = self.thread_paths(family);
        let changes_before = family.tree_changes();
        let mut writer = Writer::new(family, bases, self.thread, author, time_ms);
        let view = writer.family.thread_view(self.thread);
        // A file moved elsewhere since this checkout last saw it is still the
        // same file here until the move reaches the disk.
        let existing = paths.get(path).copied().or_else(|| {
            self.entries
                .get(path)
                .map(|entry| entry.inode)
                .filter(|inode| view.tree().is_live(*inode) && view.tree().kind(*inode) == Some(InodeKind::File))
        });
        let Some(disk) = disk else {
            // Only a file this checkout had on disk was deleted here; one it
            // never wrote is just missing.
            let had = |inode: &InodeId| self.entries.get(path).is_some_and(|entry| entry.inode == *inode);
            if let Some(inode) = existing.filter(had) {
                writer.tree_op(self.actor, Op::Delete { inode })?;
                writer.listed.push(Listed::Removed(path.to_owned()));
            }
            self.entries.remove(path);
            let Writer { deltas, tree_ops, listed, .. } = writer;
            drop(paths);
            self.patch_paths(family, changes_before, tree_ops, listed);
            return Ok(Ingested { deltas, actions: Vec::new() });
        };

        let inode = match existing {
            Some(inode) if writer.family.tree().kind(inode) == Some(InodeKind::Dir) => {
                return Err(BridgeError::DirectoryConflict(path.to_owned()));
            }
            Some(inode) => {
                // A version naming deltas the family lacks, as one a lost
                // write left behind, would make every later edit unappliable.
                let known = |version: &Frontier| version.ids().iter().all(|id| writer.family.graph().contains(*id));
                let version = self
                    .entries
                    .get(path)
                    .filter(|entry| entry.inode == inode && known(&entry.version))
                    .map_or_else(|| writer.family.thread_head(self.thread), |entry| entry.version.clone());
                writer.update(self.actor, inode, &disk, version)?;
                inode
            }
            None => writer.create(self.actor, &paths, path, &disk)?,
        };
        let Writer { deltas, tree_ops, listed, .. } = writer;
        drop(paths);
        self.patch_paths(family, changes_before, tree_ops, listed);

        let version = family.thread_head(self.thread);
        let mut actions = Vec::new();
        if let Some(content) = current_content(family, self.thread, inode)
            && content != disk.content
        {
            actions.push(DiskAction::Write {
                path: path.to_owned(),
                inode,
                content,
                executable: family.thread_view(self.thread).is_executable(inode),
                version: version.clone(),
                moved_from: None,
            });
        }
        self.entries
            .insert(path.to_owned(), DiskEntry { inode, hash: disk.hash, size: disk.size, mtime_ns: disk.mtime_ns, version });
        Ok(Ingested { deltas, actions })
    }

    /// Disk actions that bring the checkout up to date with `inodes`, which
    /// other actors changed. Git-blob content is read through `bases`.
    pub fn reconcile(
        &self,
        family: &Family,
        bases: &mut dyn BaseText,
        inodes: impl IntoIterator<Item = InodeId>,
    ) -> Vec<DiskAction> {
        let view = family.thread_view(self.thread);
        // Paths as listed, with conflict names, rather than built from each
        // file's own name.
        let listed: HashMap<InodeId, String> = view.paths().iter().map(|(path, inode)| (*inode, path.clone())).collect();
        let by_inode: HashMap<InodeId, &String> = self.entries.iter().map(|(path, entry)| (entry.inode, path)).collect();
        let version = view.version().clone();
        let mut actions = Vec::new();
        for inode in inodes {
            let live_path = listed.get(&inode).cloned();
            let moved_from = by_inode.get(&inode).filter(|old| Some(**old) != live_path.as_ref()).map(|old| (*old).clone());
            let Some(path) = live_path else {
                actions.extend(moved_from.map(|path| DiskAction::Remove { path }));
                continue;
            };
            // Without content to write, the file stays where it was rather
            // than being removed from there.
            let content = match view.file(inode) {
                Some(FileView::Text(text)) => DiskContent::Text(text),
                Some(FileView::Blob(hash)) => DiskContent::Blob(hash),
                Some(FileView::GitText(oid)) => bases.text(oid).map_or(DiskContent::GitBlob(oid), DiskContent::Text),
                None => continue,
            };
            actions.push(DiskAction::Write {
                path,
                inode,
                content,
                executable: view.is_executable(inode),
                version: version.clone(),
                moved_from,
            });
        }
        actions.retain(|action| match action {
            DiskAction::Write { path, .. } | DiskAction::Remove { path } => crate::is_writable(path),
        });
        actions
    }

    /// The file recorded at `from` is now at `to` with the same bytes:
    /// records a move rather than a delete and a create, so the file keeps
    /// its history. Returns `None` when `from` isn't a file of the thread or
    /// `to` already is one.
    #[expect(clippy::too_many_arguments, reason = "mirrors `ingest`, plus the source path")]
    pub fn rename(
        &mut self,
        family: &mut Family,
        bases: &mut dyn BaseText,
        from: &str,
        to: &str,
        disk: &DiskFile,
        author: &Author,
        time_ms: i64,
    ) -> Result<Option<Ingested>, BridgeError> {
        let paths = family.thread_view(self.thread).paths();
        let Some(inode) = paths.get(from).copied() else {
            return Ok(None);
        };
        if paths.contains_key(to) || family.tree().kind(inode) != Some(InodeKind::File) {
            return Ok(None);
        }
        let Some(entry) = self.entries.get(from).cloned() else {
            return Ok(None);
        };
        let mut writer = Writer::new(family, bases, self.thread, author, time_ms);
        let (parent, name) = writer.parent_for(self.actor, &paths, to)?;
        writer.tree_op(self.actor, Op::Move { inode, parent, name })?;
        self.entries.remove(from);
        self.entries
            .insert(to.to_owned(), DiskEntry { inode, hash: disk.hash, size: disk.size, mtime_ns: disk.mtime_ns, ..entry });
        Ok(Some(Ingested { deltas: writer.deltas, actions: Vec::new() }))
    }

    /// Takes those of `paths` this checkout brought into the thread back out
    /// of it, leaving the disk as it is, as when git came to ignore them
    /// (ADR 0060). Files others brought in stay: what this machine's own
    /// excludes leave out is no reason to take them from everyone. Returns
    /// the deltas and the paths taken out.
    pub fn forget(
        &mut self,
        family: &mut Family,
        bases: &mut dyn BaseText,
        paths: &[String],
        author: &Author,
        time_ms: i64,
    ) -> Result<Forgotten, BridgeError> {
        let view = family.thread_view(self.thread);
        let thread_paths = view.paths();
        let ours: Vec<(String, InodeId)> = paths
            .iter()
            .filter_map(|path| Some((path.clone(), *thread_paths.get(path)?)))
            .filter(|(_, inode)| inode.0.actor == self.actor && view.tree().kind(*inode) == Some(InodeKind::File))
            .collect();
        let mut writer = Writer::new(family, bases, self.thread, author, time_ms);
        let mut forgotten = Vec::with_capacity(ours.len());
        for (path, inode) in ours {
            writer.tree_op(self.actor, Op::Delete { inode })?;
            self.entries.remove(&path);
            forgotten.push(path);
        }
        Ok(Forgotten { deltas: writer.deltas, paths: forgotten })
    }

    pub fn record_written(&mut self, path: &str, entry: DiskEntry) {
        self.entries.insert(path.to_owned(), entry);
    }

    pub fn record_removed(&mut self, path: &str) {
        self.entries.remove(path);
    }
}

/// The actor for one checkout's text edits to one file.
pub fn file_actor(checkout: ActorId, inode: InodeId) -> ActorId {
    checkout.derive(inode.0.actor.0).derive(u128::from(inode.0.lamport.0))
}

fn current_content(family: &Family, thread: ThreadId, inode: InodeId) -> Option<DiskContent> {
    match family.thread_view(thread).file(inode)? {
        FileView::Text(text) => Some(DiskContent::Text(text)),
        FileView::Blob(hash) => Some(DiskContent::Blob(hash)),
        FileView::GitText(_) => None,
    }
}

struct Writer<'a> {
    family: &'a mut Family,
    bases: &'a mut dyn BaseText,
    thread: ThreadId,
    author: &'a Author,
    time_ms: i64,
    deltas: Vec<Delta>,
    /// Creates, moves and deletes applied.
    tree_ops: u64,
    /// The paths those added and removed.
    listed: Vec<Listed>,
}

/// A path a `Writer` added to the thread's tree, or removed from it.
enum Listed {
    Added(String, InodeId),
    Removed(String),
}

impl<'a> Writer<'a> {
    fn new(family: &'a mut Family, bases: &'a mut dyn BaseText, thread: ThreadId, author: &'a Author, time_ms: i64) -> Self {
        Self { family, bases, thread, author, time_ms, deltas: Vec::new(), tree_ops: 0, listed: Vec::new() }
    }

    fn apply(&mut self, actor: ActorId, parents: Frontier, op: Op) -> Result<Frontier, BridgeError> {
        let delta = self.family.delta(actor, self.thread, self.author.clone(), self.time_ms, parents, op);
        let last = delta.span.last();
        self.family.apply(delta.clone(), self.bases)?;
        self.deltas.push(delta);
        Ok(Frontier::single(last))
    }

    fn tree_op(&mut self, actor: ActorId, op: Op) -> Result<Frontier, BridgeError> {
        if matches!(op, Op::Create { .. } | Op::Move { .. } | Op::Delete { .. }) {
            self.tree_ops += 1;
        }
        let head = self.family.thread_head(self.thread);
        self.apply(actor, head, op)
    }

    fn create(
        &mut self,
        actor: ActorId,
        paths: &BTreeMap<String, InodeId>,
        path: &str,
        disk: &DiskFile,
    ) -> Result<InodeId, BridgeError> {
        let (parent, name) = self.parent_for(actor, paths, path)?;
        let content = match &disk.content {
            DiskContent::Text(_) => Content::Empty,
            DiskContent::Blob(hash) => Content::Blob(*hash),
            DiskContent::GitBlob(oid) => Content::GitText(*oid),
        };
        let created =
            self.tree_op(actor, Op::Create { kind: InodeKind::File, parent, name, content, executable: disk.executable })?;
        let inode = InodeId(created.ids()[0]);
        self.listed.push(Listed::Added(path.to_owned(), inode));
        if let DiskContent::Text(text) = &disk.content {
            self.edit_text(actor, inode, "", text, created)?;
        }
        Ok(inode)
    }

    /// The directory `path` goes in, creating missing ones, and its name.
    fn parent_for(
        &mut self,
        actor: ActorId,
        paths: &BTreeMap<String, InodeId>,
        path: &str,
    ) -> Result<(Parent, String), BridgeError> {
        let mut parent = Parent::Root;
        let mut prefix = String::new();
        let mut components: Vec<&str> = path.split('/').collect();
        let name = components.pop().expect("split yields at least one component");
        for component in components {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            let existing = paths.get(&prefix).copied();
            parent = match existing {
                Some(dir) if self.family.tree().kind(dir) == Some(InodeKind::Dir) => Parent::Dir(dir),
                _ => {
                    // A file where the disk now has a directory was replaced
                    // by it.
                    if let Some(file) = existing {
                        self.tree_op(actor, Op::Delete { inode: file })?;
                        self.listed.push(Listed::Removed(prefix.clone()));
                    }
                    let created = self.tree_op(
                        actor,
                        Op::Create {
                            kind: InodeKind::Dir,
                            parent,
                            name: component.to_owned(),
                            content: Content::Empty,
                            executable: false,
                        },
                    )?;
                    let dir = InodeId(created.ids()[0]);
                    self.listed.push(Listed::Added(prefix.clone(), dir));
                    Parent::Dir(dir)
                }
            };
        }
        Ok((parent, name.to_owned()))
    }

    fn update(&mut self, actor: ActorId, inode: InodeId, disk: &DiskFile, version: Frontier) -> Result<(), BridgeError> {
        let editable = match self.family.file(inode) {
            Some(FileView::Text(_)) => true,
            // A git blob that isn't text (it was binary until now), or is
            // too long to hold as text, isn't edited; the new text replaces it.
            Some(FileView::GitText(oid)) => self.bases.text(oid).is_some_and(|base| base.len() <= TEXT_LIMIT),
            _ => false,
        };
        match &disk.content {
            DiskContent::Text(text) if editable => {
                let base = self.text_at(inode, &version)?;
                self.edit_text(actor, inode, &base, text, version)?;
            }
            DiskContent::Text(text) => {
                let replaced = self.tree_op(actor, Op::SetContent { inode, content: Content::Empty })?;
                self.edit_text(actor, inode, "", text, replaced)?;
            }
            DiskContent::Blob(hash) if self.family.file(inode) == Some(FileView::Blob(*hash)) => {}
            DiskContent::Blob(hash) => {
                self.tree_op(actor, Op::SetContent { inode, content: Content::Blob(*hash) })?;
            }
            DiskContent::GitBlob(oid) if self.family.file(inode) == Some(FileView::GitText(*oid)) => {}
            DiskContent::GitBlob(oid) => {
                self.tree_op(actor, Op::SetContent { inode, content: Content::GitText(*oid) })?;
            }
        }
        if disk.executable != self.family.thread_view(self.thread).is_executable(inode) {
            self.tree_op(actor, Op::SetExecutable { inode, executable: disk.executable })?;
        }
        Ok(())
    }

    fn text_at(&mut self, inode: InodeId, version: &Frontier) -> Result<String, BridgeError> {
        match self.family.file(inode) {
            Some(FileView::GitText(oid)) => Ok(self.bases.text(oid).ok_or(LogError::MissingBase(oid))?),
            _ => Ok(self.family.text_doc(inode).map(|doc| doc.text_at(self.family.graph(), version)).unwrap_or_default()),
        }
    }

    fn edit_text(
        &mut self,
        checkout: ActorId,
        inode: InodeId,
        base: &str,
        text: &str,
        mut parents: Frontier,
    ) -> Result<(), BridgeError> {
        let actor = file_actor(checkout, inode);
        for op in text_edits(base, text) {
            parents = self.apply(actor, parents, Op::Text { inode, op })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THREAD: ThreadId = ThreadId(1);

    fn text(text: &str) -> DiskFile {
        let mut hash = [0u8; 32];
        for (index, byte) in text.bytes().enumerate() {
            hash[index % 32] = hash[index % 32].wrapping_mul(31).wrapping_add(byte);
        }
        DiskFile {
            content: DiskContent::Text(text.to_owned()),
            executable: false,
            hash: BlobHash(hash),
            size: text.len() as u64,
            mtime_ns: 0,
        }
    }

    /// Records `path` and checks the paths the bridge keeps are those the
    /// thread lists.
    fn ingest(bridge: &mut Bridge, family: &mut Family, path: &str, disk: Option<DiskFile>) {
        bridge.ingest(family, &mut HashMap::new(), path, disk, &Author::User, 0).expect("ingests");
        if let Some((key, paths)) = &bridge.paths
            && *key == family.view_key(THREAD)
        {
            assert_eq!(**paths, *family.thread_view(THREAD).paths(), "after {path}");
        }
    }

    #[test]
    fn the_paths_kept_across_a_batch_are_those_the_thread_lists() {
        let mut family = Family::new();
        let mut bridge = Bridge::new(ActorId(1), THREAD, BTreeMap::new());
        let mut other = Bridge::new(ActorId(2), THREAD, BTreeMap::new());
        for at in 0..20 {
            ingest(&mut bridge, &mut family, &format!("content/{}/asset{at}.txt", at % 3), Some(text("a")));
        }
        assert!(bridge.paths.as_ref().is_some_and(|(key, _)| *key == family.view_key(THREAD)), "kept, not listed again");
        ingest(&mut bridge, &mut family, "content/0/asset0.txt", None);
        ingest(&mut bridge, &mut family, "top.txt", Some(text("t")));
        other.ingest(&mut family, &mut HashMap::new(), "elsewhere.txt", Some(text("e")), &Author::User, 0).expect("ingests");
        ingest(&mut bridge, &mut family, "after-another.txt", Some(text("x")));
        assert_eq!(bridge.paths.as_ref().expect("listed again").1, family.thread_view(THREAD).paths());
        ingest(&mut bridge, &mut family, "file", Some(text("f")));
        ingest(&mut bridge, &mut family, "file/now-a-dir.txt", Some(text("d")));
        ingest(&mut bridge, &mut family, "content/1/edited.txt", Some(text("one")));
        ingest(&mut bridge, &mut family, "content/1/edited.txt", Some(text("two")));
        assert!(bridge.paths.as_ref().is_some_and(|(key, _)| *key == family.view_key(THREAD)), "kept through all of it");
    }

    #[test]
    fn a_name_another_file_shares_is_listed_again_once_one_goes() {
        let mut family = Family::new();
        let mut bridge = Bridge::new(ActorId(1), THREAD, BTreeMap::new());
        ingest(&mut bridge, &mut family, "notes.md", Some(text("mine")));
        let head = family.thread_head(THREAD);
        let theirs = family.delta(
            ActorId(9),
            THREAD,
            Author::User,
            0,
            head,
            Op::Create {
                kind: InodeKind::File,
                parent: Parent::Root,
                name: "notes.md".into(),
                content: Content::Empty,
                executable: false,
            },
        );
        family.apply(theirs, &mut HashMap::new()).expect("applies");
        ingest(&mut bridge, &mut family, "other.md", Some(text("o")));
        assert!(family.thread_view(THREAD).paths().keys().any(|path| path.starts_with("notes.md (conflict ")));
        ingest(&mut bridge, &mut family, "notes.md", None);
        assert_eq!(
            bridge.thread_paths(&family).as_ref(),
            family.thread_view(THREAD).paths().as_ref(),
            "the conflict name became the plain one"
        );
    }
}
