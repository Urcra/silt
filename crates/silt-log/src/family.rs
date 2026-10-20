use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};

use silt_crdt::{
    Added, CausalError, CausalGraph, FileTree, InodeId, InodeKind, Parent, TextDoc, TextError, TextEvent,
    TreeError, TreeEvent, TreeOp, VersionVector,
};
use silt_types::{ActorId, BlobHash, DeltaId, DeltaSpan, DeviceId, Frontier, GitOid, Lamport, ThreadId};

use crate::delta::{Author, CommentOp, CommentTarget, Content, Delta, MrOp, Op};

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
    #[error(transparent)]
    Shape(#[from] crate::delta::ShapeError),
    #[error("{0} must be the first delta of a new thread, forked from a known one")]
    MisplacedFork(DeltaId),
    #[error("{0} merges a thread this family doesn't have")]
    UnknownMergeSource(DeltaId),
    #[error("{0} refers to a file or directory outside its own history")]
    NotInHistory(DeltaId),
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
    /// The other contents the file was given, replaced by `content` or
    /// set concurrently with it, for views of threads that haven't seen
    /// `content_source`.
    earlier: Vec<(DeltaId, FileContent)>,
    executable: bool,
    executable_stamp: Stamp,
    /// Every executable bit it was given, for the same views.
    executable_changes: Vec<(DeltaId, bool)>,
}

impl FileState {
    fn new(content: FileContent, source: DeltaId, executable: bool) -> Self {
        Self {
            content,
            content_source: source,
            earlier: Vec::new(),
            executable,
            executable_stamp: Stamp::from(source),
            executable_changes: vec![(source, executable)],
        }
    }

    /// The content as of a version: the latest one set within it.
    fn content_at(&self, contains: impl Fn(DeltaId) -> bool) -> Option<(DeltaId, &FileContent)> {
        std::iter::once((self.content_source, &self.content))
            .chain(self.earlier.iter().map(|(source, content)| (*source, content)))
            .filter(|(source, _)| contains(*source))
            .max_by_key(|(source, _)| Stamp::from(*source))
    }

    fn executable_at(&self, contains: impl Fn(DeltaId) -> bool) -> bool {
        self.executable_changes
            .iter()
            .filter(|(source, _)| contains(*source))
            .max_by_key(|(source, _)| Stamp::from(*source))
            .is_some_and(|(_, executable)| *executable)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ThreadState {
    pub head: Frontier,
    /// Indices into the family log of the thread's conversation deltas.
    pub conversation: Vec<usize>,
    pub title: Option<String>,
    title_stamp: Option<Stamp>,
    /// The device that may write to the thread, with the lease's epoch.
    pub lease: Option<(DeviceId, u64)>,
    /// The thread this one was forked from.
    pub forked_from: Option<ThreadId>,
    /// Where it forked: the fork delta's parents.
    pub fork_point: Option<Frontier>,
    /// Indices into the family log of the thread's merges.
    pub merges: Vec<usize>,
}

/// One step of a thread's history as [`Family::history`] tells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryItem<'a> {
    pub delta: &'a Delta,
    pub source: HistorySource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistorySource {
    /// Recorded in the thread itself, including its merges.
    Own,
    /// From the thread's source chain, before the fork point.
    Inherited,
    /// From the thread a pull brought in; listed just before that pull. Not
    /// part of the thread's own agent session.
    Upstream,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub id: DeltaId,
    pub target: CommentTarget,
    pub text: String,
    pub by: String,
    pub time_ms: i64,
    pub resolved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeRequest {
    pub id: DeltaId,
    pub fork: ThreadId,
    pub target: ThreadId,
    pub title: String,
    pub summary: String,
    pub opened_ms: i64,
    pub status: MrStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MrStatus {
    Open,
    Approved,
    Declined { reason: String },
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
    /// What the thread has seen, unless it has seen everything.
    seen: Option<VersionVector>,
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
        let Some(seen) = &self.seen else {
            return self.family.file(inode);
        };
        self.tree.kind(inode)?;
        let state = self.family.files.get(&inode)?;
        let (_, content) = state.content_at(|id| seen.contains(id))?;
        Some(match content {
            FileContent::Text(doc) => FileView::Text(doc.text_at(&self.family.graph, &self.version)),
            FileContent::GitText(oid) => FileView::GitText(*oid),
            FileContent::Blob(hash) => FileView::Blob(*hash),
        })
    }

    pub fn is_executable(&self, inode: InodeId) -> bool {
        match &self.seen {
            None => self.family.is_executable(inode),
            Some(seen) => self
                .family
                .files
                .get(&inode)
                .is_some_and(|state| state.executable_at(|id| seen.contains(id))),
        }
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

    /// The thread's conversation as its agent and its readers should see it:
    /// for a fork, the source chain's conversation up to the fork point, then
    /// its own, with what each pull brought in from upstream just before it.
    pub fn history(&self, thread: ThreadId) -> Vec<HistoryItem<'_>> {
        self.history_within(thread, &mut HashSet::new())
    }

    /// `history`, skipping threads already being expanded: threads may pull
    /// from each other both ways.
    fn history_within(&self, thread: ThreadId, expanding: &mut HashSet<ThreadId>) -> Vec<HistoryItem<'_>> {
        let Some(state) = self.threads.get(&thread) else {
            return Vec::new();
        };
        if !expanding.insert(thread) {
            return Vec::new();
        }
        let mut items = Vec::new();
        if let (Some(source), Some(point)) = (state.forked_from, &state.fork_point) {
            let before = self.graph.version_vector(point);
            items.extend(
                self.history_within(source, expanding)
                    .into_iter()
                    .filter(|item| before.contains(item.delta.span.first()))
                    .map(|item| HistoryItem {
                        // What the source pulled in stays upstream progress.
                        source: match item.source {
                            HistorySource::Upstream => HistorySource::Upstream,
                            HistorySource::Own | HistorySource::Inherited => HistorySource::Inherited,
                        },
                        ..item
                    }),
            );
        }
        let mut seen: HashSet<DeltaId> =
            items.iter().map(|item| item.delta.span.first()).collect();
        let mut own: Vec<usize> = state.conversation.iter().chain(&state.merges).copied().collect();
        own.sort_unstable();
        for index in own {
            let delta = &self.log[index];
            if let Op::Merge { from, mr: None } = delta.op {
                let merged = self.graph.version_vector(&delta.parents);
                for item in self.history_within(from, expanding) {
                    let id = item.delta.span.first();
                    if merged.contains(id) && seen.insert(id) {
                        items.push(HistoryItem {
                            delta: item.delta,
                            source: HistorySource::Upstream,
                        });
                    }
                }
            }
            seen.insert(delta.span.first());
            items.push(HistoryItem {
                delta,
                source: HistorySource::Own,
            });
        }
        expanding.remove(&thread);
        items
    }

    /// The comments on `thread`, oldest first by causal order, so every
    /// replica lists them alike whatever order the deltas arrived in.
    pub fn comments(&self, thread: ThreadId) -> Vec<Comment> {
        let ops = || {
            self.log
                .iter()
                .filter(move |delta| delta.thread == thread)
                .filter_map(|delta| match &delta.op {
                    Op::Comment(op) => Some((delta, op)),
                    _ => None,
                })
        };
        let resolved: HashSet<DeltaId> = ops()
            .filter_map(|(_, op)| match op {
                CommentOp::Resolved { comment } => Some(*comment),
                CommentOp::Added { .. } => None,
            })
            .collect();
        let mut comments: Vec<Comment> = ops()
            .filter_map(|(delta, op)| match op {
                CommentOp::Added { target, text, by } => Some(Comment {
                    id: delta.span.first(),
                    target: *target,
                    text: text.clone(),
                    by: by.clone(),
                    time_ms: delta.time_ms,
                    resolved: resolved.contains(&delta.span.first()),
                }),
                CommentOp::Resolved { .. } => None,
            })
            .collect();
        comments.sort_by_key(|comment| Stamp::from(comment.id));
        comments
    }

    /// The file's text document, turning a git-lazy file into one first, as
    /// a first edit would. Its characters get the same ids on every replica.
    pub fn text_doc_or_base(&mut self, inode: InodeId, bases: &mut dyn BaseText) -> Result<&TextDoc, LogError> {
        Ok(materialize(&mut self.files, inode, bases)?)
    }

    /// Every merge request in the family, oldest first. Requests and
    /// decisions are taken in causal order, so the first decision wins on
    /// every replica whatever order the deltas arrived in.
    pub fn merge_requests(&self) -> Vec<MergeRequest> {
        let mut ops: Vec<(&Delta, &MrOp)> = self
            .log
            .iter()
            .filter_map(|delta| match &delta.op {
                Op::Mr(op) => Some((delta, op)),
                _ => None,
            })
            .collect();
        ops.sort_by_key(|(delta, _)| Stamp::from(delta.span.first()));
        let mut requests: Vec<MergeRequest> = Vec::new();
        for (delta, op) in ops {
            match op {
                MrOp::Opened { target, title, summary } => requests.push(MergeRequest {
                    id: delta.span.first(),
                    fork: delta.thread,
                    target: *target,
                    title: title.clone(),
                    summary: summary.clone(),
                    opened_ms: delta.time_ms,
                    status: MrStatus::Open,
                }),
                MrOp::Approved { mr } | MrOp::Declined { mr, .. } => {
                    if let Some(request) = requests.iter_mut().find(|request| request.id == *mr)
                        && request.status == MrStatus::Open
                    {
                        request.status = match op {
                            MrOp::Declined { reason, .. } => MrStatus::Declined { reason: reason.clone() },
                            _ => MrStatus::Approved,
                        };
                    }
                }
            }
        }
        requests
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
        let seen = (!whole).then(|| self.graph.version_vector(&version));
        let tree = match &seen {
            None => Cow::Borrowed(&self.tree),
            Some(seen) => Cow::Owned(self.tree.at(|id| seen.contains(id))),
        };
        ThreadView {
            family: self,
            version,
            tree,
            seen,
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
        delta.check_shape()?;
        if let Some(parent) = delta.parents.ids().iter().find(|id| !self.graph.contains(**id)) {
            return Err(CausalError::UnknownParent(*parent).into());
        }
        let applies_text = self.check(&delta, bases)?;
        // Kept as a guard: the first id was new, so the span can't be known.
        if let Added::AlreadyKnown = self.graph.add(delta.span, &delta.parents)? {
            return Ok(Applied::AlreadyKnown);
        }
        // `check` covered what projecting needs, so this doesn't fail. If it
        // did, the graph already has the delta, so it is recorded all the
        // same; every replica runs the same code and fails alike.
        let projected = self.project(&delta, applies_text);
        debug_assert!(projected.is_ok(), "a checked delta failed to project: {projected:?}");
        self.record(delta);
        Ok(Applied::New)
    }

    /// A tree operation may only use files and directories its author had
    /// seen: otherwise replicas that received the deltas in a different
    /// order would place the file differently.
    fn check_tree_history(&self, delta: &Delta) -> Result<(), LogError> {
        let seen = |inode: InodeId| self.graph.is_ancestor(inode.0, &delta.parents);
        let parent_ok = |parent: &Parent| match parent {
            Parent::Dir(dir) => self.tree.kind(*dir) == Some(InodeKind::Dir) && seen(*dir),
            Parent::Root | Parent::Trash => true,
        };
        let ok = match &delta.op {
            Op::Create { parent, .. } => parent_ok(parent),
            Op::Move { inode, parent, .. } => seen(*inode) && parent_ok(parent),
            Op::Delete { inode } => seen(*inode),
            _ => true,
        };
        if ok { Ok(()) } else { Err(LogError::NotInHistory(delta.span.first())) }
    }

    /// Validates `delta` against the current state. Returns whether a text
    /// edit applies to the file's current content.
    fn check(&mut self, delta: &Delta, bases: &mut dyn BaseText) -> Result<bool, LogError> {
        let id = delta.span.first();
        match &delta.op {
            Op::Create { .. } | Op::Move { .. } | Op::Delete { .. } => {
                self.check_tree_history(delta)?;
                self.tree.check(&tree_event(id, &delta.op))?;
            }
            Op::Fork { from } => {
                if self.threads.contains_key(&delta.thread) || !self.threads.contains_key(from) {
                    return Err(LogError::MisplacedFork(id));
                }
            }
            Op::Merge { from, .. } => {
                if !self.threads.contains_key(from) {
                    return Err(LogError::UnknownMergeSource(id));
                }
            }
            Op::Text { inode, op } => {
                if !self.based_on_current_content(*inode, &delta.parents)? {
                    return Ok(false);
                }
                let from_git = match self.files.get(inode).map(|state| &state.content) {
                    Some(FileContent::GitText(oid)) => Some(*oid),
                    _ => None,
                };
                let doc = materialize(&mut self.files, *inode, bases)?;
                let checked = doc.check(
                    &self.graph,
                    &TextEvent {
                        span: delta.span,
                        parents: delta.parents.clone(),
                        op: op.clone(),
                    },
                );
                if let (Err(_), Some(oid)) = (&checked, from_git) {
                    // A refused delta leaves the file as it found it.
                    self.files.get_mut(inode).expect("materialized above").content = FileContent::GitText(oid);
                }
                checked?;
                return Ok(true);
            }
            Op::SetContent { inode, .. } | Op::SetExecutable { inode, .. } => {
                if !self.files.contains_key(inode) {
                    return Err(LogError::NotAFile(*inode));
                }
            }
            Op::Comment(CommentOp::Added {
                target: CommentTarget::Code { inode, .. },
                ..
            }) => {
                if self.tree.kind(*inode).is_none() {
                    return Err(LogError::NotAFile(*inode));
                }
            }
            Op::Conv(_)
            | Op::Git(_)
            | Op::Lease { .. }
            | Op::Mr(_)
            | Op::Comment(_) => {}
        }
        Ok(false)
    }

    fn based_on_current_content(&self, inode: InodeId, parents: &Frontier) -> Result<bool, LogError> {
        let state = self.files.get(&inode).ok_or(LogError::NotAFile(inode))?;
        if matches!(state.content, FileContent::Blob(_)) {
            return Ok(false);
        }
        // Every applied text event descends from the content source, so a
        // recent one in the parents' history settles it without walking back
        // to the (usually old) source.
        let recent = match &state.content {
            FileContent::Text(doc) => doc.heads(),
            _ => &[],
        };
        Ok(recent.iter().any(|head| self.graph.is_ancestor(*head, parents))
            || self.graph.is_ancestor(state.content_source, parents))
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
                    self.files.insert(InodeId(id), FileState::new(file_content(*content, id), id, *executable));
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
                let new = file_content(*content, id);
                if Stamp::from(id) > Stamp::from(state.content_source) {
                    let old = std::mem::replace(&mut state.content, new);
                    state.earlier.push((state.content_source, old));
                    state.content_source = id;
                } else {
                    state.earlier.push((id, new));
                }
            }
            Op::SetExecutable { inode, executable } => {
                let state = self.files.get_mut(inode).expect("checked");
                state.executable_changes.push((id, *executable));
                if Stamp::from(id) > state.executable_stamp {
                    state.executable = *executable;
                    state.executable_stamp = Stamp::from(id);
                }
            }
            Op::Conv(_)
            | Op::Git(_)
            | Op::Lease { .. }
            | Op::Fork { .. }
            | Op::Merge { .. }
            | Op::Mr(_)
            | Op::Comment(_) => {}
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
            thread.fork_point = Some(delta.parents.clone());
        }
        if let Op::Merge { .. } = delta.op {
            thread.merges.push(index);
        }
        if let Op::Lease { device, epoch } = delta.op
            && thread.lease.is_none_or(|(_, current)| epoch > current)
        {
            thread.lease = Some((device, epoch));
        }
        if let Op::Conv(conv) = &delta.op {
            thread.conversation.push(index);
            let stamp = Some(Stamp::from(delta.span.first()));
            if let crate::delta::ConvOp::Title(title) = conv
                && stamp > thread.title_stamp
            {
                thread.title = Some(title.clone());
                thread.title_stamp = stamp;
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
