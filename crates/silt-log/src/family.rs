use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};

use silt_crdt::{
    Added, CausalError, CausalGraph, FileTree, InodeId, InodeKind, TextDoc, TextError, TextEvent,
    TreeError, TreeEvent, TreeOp,
};
use silt_types::{ActorId, BlobHash, DeltaId, DeltaSpan, DeviceId, Frontier, GitOid, Lamport, ThreadId};

use crate::delta::{Author, Content, Delta, Op};

/// Supplies the text of git blobs, which the projection needs the first
/// time a file that came from git is edited.
pub trait BaseText {
    fn text(&mut self, oid: GitOid) -> Option<String>;
}

impl BaseText for HashMap<GitOid, String> {
    fn text(&mut self, oid: GitOid) -> Option<String> {
        self.get(&oid).cloned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LogError {
    #[error(transparent)]
    Causal(#[from] CausalError),
    #[error(transparent)]
    Text(#[from] TextError),
    #[error(transparent)]
    Tree(#[from] TreeError),
    #[error("span of {span_len} ids does not match an operation of {units} units")]
    SpanLengthMismatch { span_len: u64, units: usize },
    #[error("git blob {0} is needed to edit a file but is not available")]
    MissingBase(GitOid),
    #[error("inode {0:?} is not a file")]
    NotAFile(InodeId),
}

/// What a file currently holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileView {
    Text(String),
    /// Unchanged since it came from git.
    GitText(GitOid),
    Blob(BlobHash),
}

/// Last-writer-wins order: causally later deltas always have a higher
/// lamport; the actor breaks ties between concurrent ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Stamp(Lamport, ActorId);

impl From<DeltaId> for Stamp {
    fn from(id: DeltaId) -> Self {
        Self(id.lamport, id.actor)
    }
}

#[derive(Debug, Clone)]
enum FileContent {
    Text(TextDoc),
    GitText(GitOid),
    Blob(BlobHash),
}

#[derive(Debug, Clone)]
struct FileState {
    content: FileContent,
    /// The delta that set the current content; text edits only apply to the
    /// content they were made against.
    content_source: DeltaId,
    executable: bool,
    executable_stamp: Stamp,
}

#[derive(Debug, Clone, Default)]
pub struct ThreadState {
    pub head: Frontier,
    /// Indices into the family log of the thread's conversation deltas.
    pub conversation: Vec<usize>,
    pub title: Option<String>,
    /// The device that may write to the thread, with the lease's epoch.
    pub lease: Option<(DeviceId, u64)>,
    /// The thread this one was forked from.
    pub forked_from: Option<ThreadId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    New,
    AlreadyKnown,
}

/// One thread's view of its family: the files as of the thread's head,
/// leaving out what other threads (forks) changed.
#[derive(Debug, Clone)]
pub struct ThreadView<'a> {
    family: &'a Family,
    version: Frontier,
    tree: Cow<'a, FileTree>,
    whole: bool,
}

impl ThreadView<'_> {
    pub fn version(&self) -> &Frontier {
        &self.version
    }

    pub fn tree(&self) -> &FileTree {
        &self.tree
    }

    pub fn paths(&self) -> BTreeMap<String, InodeId> {
        self.tree.paths()
    }

    pub fn file(&self, inode: InodeId) -> Option<FileView> {
        if self.whole {
            return self.family.file(inode);
        }
        self.tree.kind(inode)?;
        match self.family.file(inode)? {
            FileView::Text(_) => {
                let doc = self.family.text_doc(inode)?;
                Some(FileView::Text(doc.text_at(&self.family.graph, &self.version)))
            }
            other => Some(other),
        }
    }

    pub fn is_executable(&self, inode: InodeId) -> bool {
        self.family.is_executable(inode)
    }
}

/// The state of a thread family, projected from its log: the file tree, each
/// file's content and every thread's conversation and head.
#[derive(Debug, Clone, Default)]
pub struct Family {
    graph: CausalGraph,
    tree: FileTree,
    files: HashMap<InodeId, FileState>,
    threads: BTreeMap<ThreadId, ThreadState>,
    log: Vec<Delta>,
}

impl Family {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn graph(&self) -> &CausalGraph {
        &self.graph
    }

    pub fn tree(&self) -> &FileTree {
        &self.tree
    }

    /// Every delta, in an order where each comes after its parents.
    pub fn log(&self) -> &[Delta] {
        &self.log
    }

    pub fn thread(&self, thread: ThreadId) -> Option<&ThreadState> {
        self.threads.get(&thread)
    }

    pub fn threads(&self) -> impl Iterator<Item = (&ThreadId, &ThreadState)> {
        self.threads.iter()
    }

    pub fn conversation(&self, thread: ThreadId) -> impl DoubleEndedIterator<Item = &Delta> {
        self.threads
            .get(&thread)
            .into_iter()
            .flat_map(|state| state.conversation.iter().map(|index| &self.log[*index]))
    }

    pub fn file(&self, inode: InodeId) -> Option<FileView> {
        let state = self.files.get(&inode)?;
        Some(match &state.content {
            FileContent::Text(doc) => FileView::Text(doc.text()),
            FileContent::GitText(oid) => FileView::GitText(*oid),
            FileContent::Blob(hash) => FileView::Blob(*hash),
        })
    }

    pub fn text_doc(&self, inode: InodeId) -> Option<&TextDoc> {
        match &self.files.get(&inode)?.content {
            FileContent::Text(doc) => Some(doc),
            _ => None,
        }
    }

    pub fn is_executable(&self, inode: InodeId) -> bool {
        self.files.get(&inode).is_some_and(|state| state.executable)
    }

    /// A delta for `op` by `actor` on `thread`, based on `parents`, using the
    /// next lamports of the family clock.
    pub fn delta(
        &self,
        actor: ActorId,
        thread: ThreadId,
        author: Author,
        time_ms: i64,
        parents: Frontier,
        op: Op,
    ) -> Delta {
        Delta {
            span: DeltaSpan::new(actor, self.graph.next_lamport(), op.units() as u64),
            parents,
            thread,
            author,
            time_ms,
            op,
        }
    }

    /// What `thread` sees: everything up to its head. When the family has
    /// only this line of work, that is simply the current state.
    pub fn thread_view(&self, thread: ThreadId) -> ThreadView<'_> {
        let version = self.thread_head(thread);
        let whole = self.threads.len() <= 1 || version == *self.graph.frontier();
        let tree = if whole {
            Cow::Borrowed(&self.tree)
        } else {
            let vv = self.graph.version_vector(&version);
            Cow::Owned(self.tree.at(|id| vv.contains(id)))
        };
        ThreadView {
            family: self,
            version,
            tree,
            whole,
        }
    }

    /// The head a new delta on `thread` should use as its parents.
    pub fn thread_head(&self, thread: ThreadId) -> Frontier {
        self.threads
            .get(&thread)
            .map_or_else(|| self.graph.frontier().clone(), |state| state.head.clone())
    }

    /// Applies a delta. Known deltas are a no-op, and an invalid delta is
    /// rejected without changing anything.
    pub fn apply(&mut self, delta: Delta, bases: &mut dyn BaseText) -> Result<Applied, LogError> {
        if self.graph.contains(delta.span.first()) {
            return Ok(Applied::AlreadyKnown);
        }
        let units = delta.op.units();
        if delta.span.len != units as u64 {
            return Err(LogError::SpanLengthMismatch {
                span_len: delta.span.len,
                units,
            });
        }
        if let Some(parent) = delta.parents.ids().iter().find(|id| !self.graph.contains(**id)) {
            return Err(CausalError::UnknownParent(*parent).into());
        }
        let applies_text = self.check(&delta, bases)?;
        match self.graph.add(delta.span, &delta.parents)? {
            Added::New(_) => {}
            Added::AlreadyKnown => return Ok(Applied::AlreadyKnown),
        }
        self.project(&delta, applies_text)?;
        self.record(delta);
        Ok(Applied::New)
    }

    /// Validates `delta` against the current state. Returns whether a text
    /// edit applies to the file's current content.
    fn check(&mut self, delta: &Delta, bases: &mut dyn BaseText) -> Result<bool, LogError> {
        let id = delta.span.first();
        match &delta.op {
            Op::Create { .. } | Op::Move { .. } | Op::Delete { .. } => {
                self.tree.check(&tree_event(id, &delta.op))?;
            }
            Op::Text { inode, op } => {
                if !self.based_on_current_content(*inode, &delta.parents)? {
                    return Ok(false);
                }
                let doc = materialize(&mut self.files, *inode, bases)?;
                doc.check(
                    &self.graph,
                    &TextEvent {
                        span: delta.span,
                        parents: delta.parents.clone(),
                        op: op.clone(),
                    },
                )?;
                return Ok(true);
            }
            Op::SetContent { inode, .. } | Op::SetExecutable { inode, .. } => {
                if !self.files.contains_key(inode) {
                    return Err(LogError::NotAFile(*inode));
                }
            }
            Op::Conv(_) | Op::Git(_) | Op::Lease { .. } | Op::Fork { .. } => {}
        }
        Ok(false)
    }

    fn based_on_current_content(&self, inode: InodeId, parents: &Frontier) -> Result<bool, LogError> {
        let state = self.files.get(&inode).ok_or(LogError::NotAFile(inode))?;
        if matches!(state.content, FileContent::Blob(_)) {
            return Ok(false);
        }
        Ok(self.graph.is_ancestor(state.content_source, parents))
    }

    fn project(&mut self, delta: &Delta, applies_text: bool) -> Result<(), LogError> {
        let id = delta.span.first();
        match &delta.op {
            Op::Create {
                kind,
                content,
                executable,
                ..
            } => {
                self.tree.apply(&tree_event(id, &delta.op))?;
                if *kind != InodeKind::Dir {
                    self.files.insert(
                        InodeId(id),
                        FileState {
                            content: file_content(*content, id),
                            content_source: id,
                            executable: *executable,
                            executable_stamp: Stamp::from(id),
                        },
                    );
                }
            }
            Op::Move { .. } | Op::Delete { .. } => {
                self.tree.apply(&tree_event(id, &delta.op))?;
            }
            Op::Text { inode, op } => {
                if applies_text {
                    let event = TextEvent {
                        span: delta.span,
                        parents: delta.parents.clone(),
                        op: op.clone(),
                    };
                    let graph = &self.graph;
                    if let Some(FileContent::Text(doc)) =
                        self.files.get_mut(inode).map(|state| &mut state.content)
                    {
                        doc.apply(graph, &event)?;
                    }
                }
            }
            Op::SetContent { inode, content } => {
                let state = self.files.get_mut(inode).expect("checked");
                if Stamp::from(id) > Stamp::from(state.content_source) {
                    state.content = file_content(*content, id);
                    state.content_source = id;
                }
            }
            Op::SetExecutable { inode, executable } => {
                let state = self.files.get_mut(inode).expect("checked");
                if Stamp::from(id) > state.executable_stamp {
                    state.executable = *executable;
                    state.executable_stamp = Stamp::from(id);
                }
            }
            Op::Conv(_) | Op::Git(_) | Op::Lease { .. } | Op::Fork { .. } => {}
        }
        Ok(())
    }

    fn record(&mut self, delta: Delta) {
        let index = self.log.len();
        let thread = self.threads.entry(delta.thread).or_default();
        let parents_are_head = delta.parents.ids().iter().all(|parent| thread.head.contains(parent));
        for parent in delta.parents.ids() {
            thread.head.remove(parent);
        }
        thread.head.insert(delta.span.last());
        if !parents_are_head && thread.head.len() > 1 {
            thread.head = self.graph.minimize(thread.head.ids().iter().copied());
        }
        if let Op::Fork { from } = delta.op {
            thread.forked_from = Some(from);
        }
        if let Op::Lease { device, epoch } = delta.op
            && thread.lease.is_none_or(|(_, current)| epoch > current)
        {
            thread.lease = Some((device, epoch));
        }
        if let Op::Conv(conv) = &delta.op {
            thread.conversation.push(index);
            if let crate::delta::ConvOp::Title(title) = conv {
                thread.title = Some(title.clone());
            }
        }
        self.log.push(delta);
    }
}

fn materialize<'a>(
    files: &'a mut HashMap<InodeId, FileState>,
    inode: InodeId,
    bases: &mut dyn BaseText,
) -> Result<&'a mut TextDoc, LogError> {
    let state = files.get_mut(&inode).ok_or(LogError::NotAFile(inode))?;
    if let FileContent::GitText(oid) = state.content {
        let text = bases.text(oid).ok_or(LogError::MissingBase(oid))?;
        state.content = FileContent::Text(TextDoc::from_base(state.content_source, &text));
    }
    match &mut state.content {
        FileContent::Text(doc) => Ok(doc),
        _ => Err(LogError::NotAFile(inode)),
    }
}

fn tree_event(id: DeltaId, op: &Op) -> TreeEvent {
    let op = match op {
        Op::Create { kind, parent, name, .. } => TreeOp::Create {
            kind: *kind,
            parent: *parent,
            name: name.clone(),
        },
        Op::Move { inode, parent, name } => TreeOp::Move {
            inode: *inode,
            parent: *parent,
            name: name.clone(),
        },
        Op::Delete { inode } => TreeOp::Delete { inode: *inode },
        _ => unreachable!("only tree operations become tree events"),
    };
    TreeEvent { id, op }
}

fn file_content(content: Content, source: DeltaId) -> FileContent {
    match content {
        Content::Empty => FileContent::Text(TextDoc::from_base(source, "")),
        Content::GitText(oid) => FileContent::GitText(oid),
        Content::Blob(hash) => FileContent::Blob(hash),
    }
}
