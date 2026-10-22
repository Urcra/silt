use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use serde::{Deserialize, Serialize};
use silt_crdt::{
    Added, CausalError, CausalGraph, FileTree, InodeId, InodeKind, Parent, Stamp, TextDoc, TextError, TextEvent, TreeError,
    TreeEvent, TreeOp, VersionVector,
};
use silt_types::{ActorId, BlobHash, DeltaId, DeltaSpan, DeviceId, Frontier, GitOid, ThreadId};

use crate::delta::{Author, CommentOp, CommentTarget, Content, Delta, MrOp, Op, SessionRef, TurnMark};

mod snapshot;
mod turns;

pub use snapshot::{SNAPSHOT_FORMAT, SnapshotError};
use turns::Turns;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
enum FileContent {
    Text(TextDoc),
    GitText(GitOid),
    Blob(BlobHash),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// What the family's log says of one thread; only applying deltas changes it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThreadState {
    head: Frontier,
    /// Indices into the family log of the thread's conversation deltas.
    conversation: Vec<usize>,
    title: Option<String>,
    title_stamp: Option<Stamp>,
    lease: Option<(DeviceId, u64)>,
    forked_from: Option<ThreadId>,
    fork_point: Option<Frontier>,
    /// Indices into the family log of the thread's merges.
    merges: Vec<usize>,
}

impl ThreadState {
    pub fn head(&self) -> &Frontier {
        &self.head
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// The device that may write to the thread, with the lease's epoch.
    pub fn lease(&self) -> Option<(DeviceId, u64)> {
        self.lease
    }

    /// The thread this one was forked from.
    pub fn forked_from(&self) -> Option<ThreadId> {
        self.forked_from
    }

    /// Where it forked: the fork delta's parents.
    pub fn fork_point(&self) -> Option<&Frontier> {
        self.fork_point.as_ref()
    }
}

/// One step of a thread's history as [`Family::history`] tells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryItem<'a> {
    pub delta: &'a Delta,
    pub source: HistorySource,
    /// A revert rewound it (ADR 0029): still history, no longer the
    /// conversation.
    pub rewound: bool,
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
    /// Oldest first, by causal order.
    pub replies: Vec<Reply>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub id: DeltaId,
    pub text: String,
    pub by: String,
    pub time_ms: i64,
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
    /// The latest request for changes made while it was open.
    pub changes_requested: Option<ChangesRequested>,
    /// The latest walk-through for reviewers, and who wrote it.
    pub guide: Option<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangesRequested {
    pub reason: String,
    pub at_ms: i64,
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
    shape: Arc<Shape>,
}

impl ThreadView<'_> {
    pub fn version(&self) -> &Frontier {
        &self.version
    }

    pub fn tree(&self) -> &FileTree {
        self.shape.tree.as_ref().unwrap_or(&self.family.tree)
    }

    pub fn paths(&self) -> Arc<BTreeMap<String, InodeId>> {
        Arc::clone(self.shape.paths.get_or_init(|| Arc::new(self.tree().paths())))
    }

    pub fn file(&self, inode: InodeId) -> Option<FileView> {
        let Some(seen) = &self.shape.seen else {
            return self.family.file(inode);
        };
        self.tree().kind(inode)?;
        self.family.file_at(inode, seen)
    }

    pub fn is_executable(&self, inode: InodeId) -> bool {
        match &self.shape.seen {
            None => self.family.is_executable(inode),
            Some(seen) => self.family.files.get(&inode).is_some_and(|state| state.executable_at(|id| seen.contains(id))),
        }
    }
}

/// What one [`Family::history`] call has worked out so far: each pull would
/// otherwise rebuild the whole history of the thread it pulled from.
#[derive(Default)]
struct Expansion<'a> {
    open: HashSet<ThreadId>,
    done: HashMap<ThreadId, Rc<[HistoryItem<'a>]>>,
}

/// What a view of one version holds beyond the family itself.
#[derive(Debug)]
struct Shape {
    /// What the thread has seen, unless it has seen everything.
    seen: Option<VersionVector>,
    /// The tree as of that version; `None` when it is the family's own.
    tree: Option<FileTree>,
    paths: OnceLock<Arc<BTreeMap<String, InodeId>>>,
}

/// The shapes of recently viewed versions. A thread's view replays the move
/// log and listing paths walks the whole tree, and recording one file asks
/// for its thread's view several times over.
#[derive(Debug, Default)]
struct Shapes(Mutex<Vec<(ShapeKey, Arc<Shape>)>>);

#[derive(Debug, PartialEq, Eq)]
enum ShapeKey {
    /// The family's own tree, after this many changes to it. Most deltas
    /// leave the tree as it was, so its paths are listed again only when
    /// one changes it, not after every line of a conversation.
    Whole { tree_changes: u64 },
    /// The thread's version, when it hasn't seen everything.
    At(Frontier),
}

const SHAPES_KEPT: usize = 8;

impl Clone for Shapes {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl Shapes {
    fn get_or_insert(&self, key: ShapeKey, make: impl FnOnce() -> Shape) -> Arc<Shape> {
        let mut shapes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, shape)) = shapes.iter().find(|(known, _)| *known == key) {
            return Arc::clone(shape);
        }
        let shape = Arc::new(make());
        if shapes.len() >= SHAPES_KEPT {
            shapes.remove(0);
        }
        shapes.push((key, Arc::clone(&shape)));
        shape
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
    turns: HashMap<SessionRef, Turns>,
    log: Vec<Delta>,
    shapes: Shapes,
    /// The fingerprints snapshots took of the log's first deltas. The log
    /// only grows, so each delta is fingerprinted once rather than by every
    /// snapshot, which made recording a long session quadratic.
    fingerprints: Vec<u64>,
    tree_changes: u64,
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

    /// The tool the session started last and hasn't finished, unless its
    /// turn ended since.
    pub fn open_tool(&self, session: &SessionRef) -> Option<&str> {
        self.turns.get(session).and_then(Turns::open_tool)
    }

    /// Whether the session's last turn mark leaves a turn going.
    pub fn in_turn(&self, session: &SessionRef) -> bool {
        self.turns.get(session).is_some_and(Turns::in_turn)
    }

    /// Whether the session's `mark` is recorded already, so recording it
    /// again would add nothing.
    pub fn has_turn_mark(&self, session: &SessionRef, mark: &TurnMark) -> bool {
        self.turns.get(session).is_some_and(|turns| turns.has(mark))
    }

    pub fn threads(&self) -> impl Iterator<Item = (&ThreadId, &ThreadState)> {
        self.threads.iter()
    }

    /// The thread's conversation as its agent and its readers should see it:
    /// for a fork, the source chain's conversation up to the fork point, then
    /// its own, with what each pull brought in from upstream just before it.
    pub fn history(&self, thread: ThreadId) -> Vec<HistoryItem<'_>> {
        self.history_within(thread, &mut Expansion::default()).0.to_vec()
    }

    /// `history`, with what this call already worked out. The flag says
    /// whether the history is whole rather than cut short where threads pull
    /// from each other both ways; only whole ones are reused.
    fn history_within<'a>(&'a self, thread: ThreadId, expansion: &mut Expansion<'a>) -> (Rc<[HistoryItem<'a>]>, bool) {
        if let Some(done) = expansion.done.get(&thread) {
            return (Rc::clone(done), true);
        }
        let Some(state) = self.threads.get(&thread) else {
            return (Rc::from([]), true);
        };
        if !expansion.open.insert(thread) {
            return (Rc::from([]), false);
        }
        let mut whole = true;
        let mut items = Vec::new();
        if let (Some(source), Some(point)) = (state.forked_from, &state.fork_point) {
            let before = self.graph.version_vector(point);
            let (inherited, inherited_whole) = self.history_within(source, expansion);
            whole &= inherited_whole;
            items.extend(inherited.iter().filter(|item| before.contains(item.delta.span.first())).map(|item| HistoryItem {
                // What the source pulled in stays upstream progress.
                source: match item.source {
                    HistorySource::Upstream => HistorySource::Upstream,
                    HistorySource::Own | HistorySource::Inherited => HistorySource::Inherited,
                },
                ..*item
            }));
        }
        let mut seen: HashSet<DeltaId> = items.iter().map(|item| item.delta.span.first()).collect();
        let mut own: Vec<usize> = state.conversation.iter().chain(&state.merges).copied().collect();
        own.sort_unstable();
        for index in own {
            let delta = &self.log[index];
            if let Op::Merge { from, mr: None } = delta.op {
                let merged = self.graph.version_vector(&delta.parents);
                let (upstream, upstream_whole) = self.history_within(from, expansion);
                whole &= upstream_whole;
                for item in upstream.iter() {
                    let id = item.delta.span.first();
                    if merged.contains(id) && seen.insert(id) {
                        items.push(HistoryItem { delta: item.delta, source: HistorySource::Upstream, rewound: item.rewound });
                    }
                }
            }
            seen.insert(delta.span.first());
            items.push(HistoryItem { delta, source: HistorySource::Own, rewound: false });
        }
        mark_rewound(&mut items);
        expansion.open.remove(&thread);
        let items: Rc<[HistoryItem<'a>]> = items.into();
        if whole {
            expansion.done.insert(thread, Rc::clone(&items));
        }
        (items, whole)
    }

    /// The comments on `thread`, oldest first by causal order, so every
    /// replica lists them alike whatever order the deltas arrived in.
    pub fn comments(&self, thread: ThreadId) -> Vec<Comment> {
        comments_in(&self.log, thread)
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
        merge_requests_in(&self.log)
    }
}

/// [`Family::comments`] over any deltas of a family, such as the ones a
/// server lets someone read.
pub fn comments_in(log: &[Delta], thread: ThreadId) -> Vec<Comment> {
    let ops = || {
        log.iter().filter(move |delta| delta.thread == thread).filter_map(|delta| match &delta.op {
            Op::Comment(op) => Some((delta, op)),
            _ => None,
        })
    };
    let resolved: HashSet<DeltaId> = ops()
        .filter_map(|(_, op)| match op {
            CommentOp::Resolved { comment } => Some(*comment),
            _ => None,
        })
        .collect();
    // A reply naming another reply, as an older or careless peer may
    // send, still lands under the first comment.
    let parent_of: HashMap<DeltaId, DeltaId> = ops()
        .filter_map(|(delta, op)| match op {
            CommentOp::Replied { parent, .. } => Some((delta.span.first(), *parent)),
            _ => None,
        })
        .collect();
    let root = |mut id: DeltaId| {
        for _ in 0..parent_of.len() {
            match parent_of.get(&id) {
                Some(parent) => id = *parent,
                None => break,
            }
        }
        id
    };
    let mut replies: HashMap<DeltaId, Vec<Reply>> = HashMap::new();
    for (delta, op) in ops() {
        if let CommentOp::Replied { parent, text, by } = op {
            let reply = Reply { id: delta.span.first(), text: text.clone(), by: by.clone(), time_ms: delta.time_ms };
            replies.entry(root(*parent)).or_default().push(reply);
        }
    }
    let mut comments: Vec<Comment> = ops()
        .filter_map(|(delta, op)| match op {
            CommentOp::Added { target, text, by } => {
                let id = delta.span.first();
                let mut replies = replies.remove(&id).unwrap_or_default();
                replies.sort_by_key(|reply| Stamp::from(reply.id));
                Some(Comment {
                    id,
                    target: *target,
                    text: text.clone(),
                    by: by.clone(),
                    time_ms: delta.time_ms,
                    resolved: resolved.contains(&id),
                    replies,
                })
            }
            _ => None,
        })
        .collect();
    comments.sort_by_key(|comment| Stamp::from(comment.id));
    comments
}

/// Marks what each revert in `items` rewound: everything after the delta it
/// names and before the revert itself (ADR 0029).
fn mark_rewound(items: &mut [HistoryItem<'_>]) {
    for at in 0..items.len() {
        let Op::Conv(crate::delta::ConvOp::Rewound { to }) = items[at].delta.op else {
            continue;
        };
        if let Some(from) = items[..at].iter().position(|item| item.delta.span.contains(to)) {
            for item in &mut items[from + 1..at] {
                item.rewound = true;
            }
        }
    }
}

/// The thread's own conversation deltas a revert rewound, from the log
/// alone, as a server sees it.
pub fn rewound_in(log: &[Delta], thread: ThreadId) -> HashSet<DeltaId> {
    let own: Vec<&Delta> = log.iter().filter(|delta| delta.thread == thread && matches!(delta.op, Op::Conv(_))).collect();
    let mut rewound = HashSet::new();
    for (at, delta) in own.iter().enumerate() {
        let Op::Conv(crate::delta::ConvOp::Rewound { to }) = delta.op else {
            continue;
        };
        if let Some(from) = own[..at].iter().position(|earlier| earlier.span.contains(to)) {
            rewound.extend(own[from + 1..at].iter().map(|earlier| earlier.span.first()));
        }
    }
    rewound
}

/// Moves a thread's `head` past `delta`, one of the thread's own.
fn advance_head(head: &mut Frontier, delta: &Delta, graph: &CausalGraph) {
    let parents_are_head = delta.parents.ids().iter().all(|parent| head.contains(parent));
    for parent in delta.parents.ids() {
        head.remove(parent);
    }
    head.insert(delta.span.last());
    if !parents_are_head && head.len() > 1 {
        *head = graph.minimize(head.ids().iter().copied());
    }
}

/// [`Family::thread_head`] over a family's deltas in the order they were
/// taken, with the graph that holds them, such as a server's room.
pub fn thread_head_in(log: &[Delta], graph: &CausalGraph, thread: ThreadId) -> Frontier {
    let mut own = log.iter().filter(|delta| delta.thread == thread).peekable();
    if own.peek().is_none() {
        return graph.frontier().clone();
    }
    let mut head = Frontier::default();
    for delta in own {
        advance_head(&mut head, delta, graph);
    }
    head
}

/// Where the thread's turn `turn` (from 1) ended (ADR 0029): its head just
/// before the next turn began, or its head now for its last turn. `None`
/// when it has no such turn.
///
/// The next turn begins at its prompt-submitted mark or at the line with its
/// prompt, whichever the log took first: agents record them in either order.
/// `is_prompt` says which of the thread's deltas hold a prompt; it sees them
/// in order.
pub fn turn_point_by(
    log: &[Delta],
    graph: &CausalGraph,
    thread: ThreadId,
    turn: usize,
    is_prompt: impl FnMut(&Delta) -> bool,
) -> Option<Frontier> {
    let end = turn_end_by(log, thread, turn, is_prompt)?;
    Some(thread_head_in(&log[..end], graph, thread))
}

/// How much of the log turn `turn` ended with: the index where the next
/// turn began, or the log's length for the last one. See [`turn_point_by`].
pub fn turn_end_by(log: &[Delta], thread: ThreadId, turn: usize, mut is_prompt: impl FnMut(&Delta) -> bool) -> Option<usize> {
    let (mut marks, mut prompts) = (Vec::new(), Vec::new());
    for (index, delta) in log.iter().enumerate().filter(|(_, delta)| delta.thread == thread) {
        if matches!(delta.op, Op::Conv(crate::delta::ConvOp::Turn { mark: TurnMark::PromptSubmitted, .. })) {
            marks.push(index);
        }
        if is_prompt(delta) {
            prompts.push(index);
        }
    }
    if turn == 0 || turn > marks.len() {
        return None;
    }
    Some([marks.get(turn), prompts.get(turn)].into_iter().flatten().min().copied().unwrap_or(log.len()))
}

/// [`turn_point_by`] where turns begin at their marks alone.
pub fn turn_point(log: &[Delta], graph: &CausalGraph, thread: ThreadId, turn: usize) -> Option<Frontier> {
    turn_point_by(log, graph, thread, turn, |_| false)
}

/// [`Family::merge_requests`] over any deltas of a family.
pub fn merge_requests_in(log: &[Delta]) -> Vec<MergeRequest> {
    let mut ops: Vec<(&Delta, &MrOp)> = log
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
                changes_requested: None,
                guide: None,
            }),
            MrOp::Guided { mr, guide, by } => {
                if let Some(request) = requests.iter_mut().find(|request| request.id == *mr) {
                    request.guide = Some((guide.clone(), by.clone()));
                }
            }
            MrOp::ChangesRequested { mr, reason } => {
                if let Some(request) = requests.iter_mut().find(|request| request.id == *mr)
                    && request.status == MrStatus::Open
                {
                    request.changes_requested = Some(ChangesRequested { reason: reason.clone(), at_ms: delta.time_ms });
                }
            }
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

impl Family {
    pub fn conversation(&self, thread: ThreadId) -> impl DoubleEndedIterator<Item = &Delta> {
        self.threads.get(&thread).into_iter().flat_map(|state| state.conversation.iter().map(|index| &self.log[*index]))
    }

    pub fn file(&self, inode: InodeId) -> Option<FileView> {
        let state = self.files.get(&inode)?;
        Some(match &state.content {
            FileContent::Text(doc) => FileView::Text(doc.text()),
            FileContent::GitText(oid) => FileView::GitText(*oid),
            FileContent::Blob(hash) => FileView::Blob(*hash),
        })
    }

    /// The file's content as of the version containing exactly the ids in
    /// `vv`, whether or not the tree there holds it.
    pub fn file_at(&self, inode: InodeId, vv: &VersionVector) -> Option<FileView> {
        let (_, content) = self.files.get(&inode)?.content_at(|id| vv.contains(id))?;
        Some(match content {
            FileContent::Text(doc) => FileView::Text(doc.text_at_vv(vv)),
            FileContent::GitText(oid) => FileView::GitText(*oid),
            FileContent::Blob(hash) => FileView::Blob(*hash),
        })
    }

    /// The file at `path` as of `at`, for a permalink into `thread`. `None`
    /// unless `at` is within the thread's history, so a link to one thread
    /// never shows another thread's work, or when no file was there then.
    pub fn file_in_thread_at(&self, thread: ThreadId, path: &str, at: &Frontier) -> Option<FileView> {
        let head = self.thread_head(thread);
        if at.is_empty() || !at.ids().iter().all(|id| self.graph.is_ancestor(*id, &head)) {
            return None;
        }
        let vv = self.graph.version_vector(at);
        let inode = *self.tree().at(|id| vv.contains(id)).paths().get(path)?;
        self.file_at(inode, &vv)
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
    pub fn delta(&self, actor: ActorId, thread: ThreadId, author: Author, time_ms: i64, parents: Frontier, op: Op) -> Delta {
        Delta { span: DeltaSpan::new(actor, self.graph.next_lamport(), op.units() as u64), parents, thread, author, time_ms, op }
    }

    /// What `thread` sees: everything up to its head. When the family has
    /// only this line of work, that is simply the current state.
    pub fn thread_view(&self, thread: ThreadId) -> ThreadView<'_> {
        let version = self.thread_head(thread);
        let whole = self.threads.len() <= 1 || version == *self.graph.frontier();
        let key = if whole { ShapeKey::Whole { tree_changes: self.tree_changes } } else { ShapeKey::At(version.clone()) };
        let shape = self.shapes.get_or_insert(key, || {
            let seen = (!whole).then(|| self.graph.version_vector(&version));
            Shape { tree: seen.as_ref().map(|seen| self.tree.at(|id| seen.contains(id))), seen, paths: OnceLock::new() }
        });
        ThreadView { family: self, version, shape }
    }

    /// The head a new delta on `thread` should use as its parents.
    pub fn thread_head(&self, thread: ThreadId) -> Frontier {
        self.threads.get(&thread).map_or_else(|| self.graph.frontier().clone(), |state| state.head.clone())
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
        // The graph's own checks first: `check` may turn a file from git into
        // text, which a delta the graph then refused would leave behind.
        self.graph.begin();
        let fits = self.graph.add(delta.span, &delta.parents);
        self.graph.roll_back();
        fits?;
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
                let checked =
                    doc.check(&self.graph, &TextEvent { span: delta.span, parents: delta.parents.clone(), op: op.clone() });
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
            Op::Comment(CommentOp::Added { target: CommentTarget::Code { inode, .. }, .. }) => {
                if self.tree.kind(*inode).is_none() {
                    return Err(LogError::NotAFile(*inode));
                }
            }
            Op::Conv(_) | Op::Git(_) | Op::Lease { .. } | Op::Mr(_) | Op::Comment(_) => {}
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
            Op::Create { kind, content, executable, .. } => {
                self.tree_changes += 1;
                self.tree.apply(&tree_event(id, &delta.op))?;
                if *kind != InodeKind::Dir {
                    self.files.insert(InodeId(id), FileState::new(file_content(*content, id), id, *executable));
                }
            }
            Op::Move { .. } | Op::Delete { .. } => {
                self.tree_changes += 1;
                self.tree.apply(&tree_event(id, &delta.op))?;
            }
            Op::Text { inode, op } => {
                if applies_text {
                    let event = TextEvent { span: delta.span, parents: delta.parents.clone(), op: op.clone() };
                    let graph = &self.graph;
                    if let Some(FileContent::Text(doc)) = self.files.get_mut(inode).map(|state| &mut state.content) {
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
            Op::Conv(_) | Op::Git(_) | Op::Lease { .. } | Op::Fork { .. } | Op::Merge { .. } | Op::Mr(_) | Op::Comment(_) => {}
        }
        Ok(())
    }

    fn record(&mut self, delta: Delta) {
        let index = self.log.len();
        let thread = self.threads.entry(delta.thread).or_default();
        advance_head(&mut thread.head, &delta, &self.graph);
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
        if let Op::Conv(crate::delta::ConvOp::Turn { session, mark }) = &delta.op {
            self.turns.entry(session.clone()).or_default().record(mark);
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
        Op::Create { kind, parent, name, .. } => TreeOp::Create { kind: *kind, parent: *parent, name: name.clone() },
        Op::Move { inode, parent, name } => TreeOp::Move { inode: *inode, parent: *parent, name: name.clone() },
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
