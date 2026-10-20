use std::collections::{BTreeMap, HashMap};

use silt_crdt::{InodeId, InodeKind, Parent};
use silt_log::{Author, BaseText, Content, Delta, Family, FileView, LogError, Op};
use silt_types::{ActorId, BlobHash, Frontier, ThreadId};

use crate::diff::text_edits;

/// What the checkout last read from or wrote to disk for one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskEntry {
    pub inode: InodeId,
    /// Hash of the bytes on disk, to recognise our own writes.
    pub hash: [u8; 32],
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
    pub hash: [u8; 32],
    pub size: u64,
    pub mtime_ns: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskContent {
    /// Git-canonical text.
    Text(String),
    /// Content kept whole, already put in the blob store.
    Blob(BlobHash),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskAction {
    Write {
        path: String,
        inode: InodeId,
        content: DiskContent,
        executable: bool,
        version: Frontier,
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
}

impl Bridge {
    pub fn new(actor: ActorId, thread: ThreadId, entries: BTreeMap<String, DiskEntry>) -> Self {
        Self {
            actor,
            thread,
            entries,
        }
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
        let mut writer = Writer {
            family,
            bases,
            thread: self.thread,
            author,
            time_ms,
            deltas: Vec::new(),
        };
        let view = writer.family.thread_view(self.thread);
        let paths = view.paths();
        // A file moved elsewhere since this checkout last saw it is still the
        // same file here until the move reaches the disk.
        let existing = paths.get(path).copied().or_else(|| {
            self.entries
                .get(path)
                .map(|entry| entry.inode)
                .filter(|inode| view.tree().is_live(*inode) && view.tree().kind(*inode) == Some(InodeKind::File))
        });
        let Some(disk) = disk else {
            if let Some(inode) = existing {
                writer.tree_op(self.actor, Op::Delete { inode })?;
            }
            self.entries.remove(path);
            return Ok(Ingested {
                deltas: writer.deltas,
                actions: Vec::new(),
            });
        };

        let inode = match existing {
            Some(inode) if writer.family.tree().kind(inode) == Some(InodeKind::Dir) => {
                return Err(BridgeError::DirectoryConflict(path.to_owned()));
            }
            Some(inode) => {
                let version = self
                    .entries
                    .get(path)
                    .filter(|entry| entry.inode == inode)
                    .map_or_else(|| writer.family.graph().frontier().clone(), |entry| entry.version.clone());
                writer.update(self.actor, inode, &disk, version)?;
                inode
            }
            None => writer.create(self.actor, &paths, path, &disk)?,
        };
        let deltas = writer.deltas;

        let version = family.thread_head(self.thread);
        let mut actions = Vec::new();
        if let Some(content) = current_content(family, self.thread, inode)
            && content != disk.content
        {
            actions.push(DiskAction::Write {
                path: path.to_owned(),
                inode,
                content,
                executable: family.is_executable(inode),
                version: version.clone(),
            });
        }
        self.entries.insert(
            path.to_owned(),
            DiskEntry {
                inode,
                hash: disk.hash,
                size: disk.size,
                mtime_ns: disk.mtime_ns,
                version,
            },
        );
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
        let paths = view.paths();
        let by_inode: HashMap<InodeId, &String> =
            self.entries.iter().map(|(path, entry)| (entry.inode, path)).collect();
        let version = view.version().clone();
        let mut actions = Vec::new();
        for inode in inodes {
            let live_path = view
                .tree()
                .is_live(inode)
                .then(|| view.tree().path_of(inode))
                .flatten()
                .filter(|path| paths.get(path) == Some(&inode));
            let old_path = by_inode.get(&inode).copied();
            if let Some(old) = old_path.filter(|old| Some(*old) != live_path.as_ref()) {
                actions.push(DiskAction::Remove { path: old.clone() });
            }
            let Some(path) = live_path else {
                continue;
            };
            let content = match view.file(inode) {
                Some(FileView::Text(text)) => DiskContent::Text(text),
                Some(FileView::Blob(hash)) => DiskContent::Blob(hash),
                Some(FileView::GitText(oid)) => match bases.text(oid) {
                    Some(text) => DiskContent::Text(text),
                    None => continue,
                },
                None => continue,
            };
            actions.push(DiskAction::Write {
                path,
                inode,
                content,
                executable: view.is_executable(inode),
                version: version.clone(),
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
        let Some(entry) = self.entries.remove(from) else {
            return Ok(None);
        };
        let mut writer = Writer {
            family,
            bases,
            thread: self.thread,
            author,
            time_ms,
            deltas: Vec::new(),
        };
        let (parent, name) = writer.parent_for(self.actor, &paths, to)?;
        writer.tree_op(self.actor, Op::Move { inode, parent, name })?;
        self.entries.insert(
            to.to_owned(),
            DiskEntry {
                inode,
                hash: disk.hash,
                size: disk.size,
                mtime_ns: disk.mtime_ns,
                ..entry
            },
        );
        Ok(Some(Ingested {
            deltas: writer.deltas,
            actions: Vec::new(),
        }))
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
}

impl Writer<'_> {
    fn apply(&mut self, actor: ActorId, parents: Frontier, op: Op) -> Result<Frontier, BridgeError> {
        let delta = self
            .family
            .delta(actor, self.thread, self.author.clone(), self.time_ms, parents, op);
        let last = delta.span.last();
        self.family.apply(delta.clone(), self.bases)?;
        self.deltas.push(delta);
        Ok(Frontier::single(last))
    }

    fn tree_op(&mut self, actor: ActorId, op: Op) -> Result<Frontier, BridgeError> {
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
        };
        let created = self.tree_op(
            actor,
            Op::Create {
                kind: InodeKind::File,
                parent,
                name,
                content,
                executable: disk.executable,
            },
        )?;
        let inode = InodeId(created.ids()[0]);
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
            parent = match paths.get(&prefix) {
                Some(dir) => Parent::Dir(*dir),
                None => {
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
                    Parent::Dir(InodeId(created.ids()[0]))
                }
            };
        }
        Ok((parent, name.to_owned()))
    }

    fn update(&mut self, actor: ActorId, inode: InodeId, disk: &DiskFile, version: Frontier) -> Result<(), BridgeError> {
        match (&disk.content, self.family.file(inode)) {
            (DiskContent::Text(text), Some(FileView::Text(_) | FileView::GitText(_))) => {
                let base = self.text_at(inode, &version)?;
                self.edit_text(actor, inode, &base, text, version)?;
            }
            (DiskContent::Text(text), _) => {
                let replaced = self.tree_op(
                    actor,
                    Op::SetContent {
                        inode,
                        content: Content::Empty,
                    },
                )?;
                self.edit_text(actor, inode, "", text, replaced)?;
            }
            (DiskContent::Blob(hash), Some(FileView::Blob(current))) if *hash == current => {}
            (DiskContent::Blob(hash), _) => {
                self.tree_op(
                    actor,
                    Op::SetContent {
                        inode,
                        content: Content::Blob(*hash),
                    },
                )?;
            }
        }
        if disk.executable != self.family.is_executable(inode) {
            self.tree_op(
                actor,
                Op::SetExecutable {
                    inode,
                    executable: disk.executable,
                },
            )?;
        }
        Ok(())
    }

    fn text_at(&mut self, inode: InodeId, version: &Frontier) -> Result<String, BridgeError> {
        match self.family.file(inode) {
            Some(FileView::GitText(oid)) => Ok(self.bases.text(oid).ok_or(LogError::MissingBase(oid))?),
            _ => Ok(self
                .family
                .text_doc(inode)
                .map(|doc| doc.text_at(self.family.graph(), version))
                .unwrap_or_default()),
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
