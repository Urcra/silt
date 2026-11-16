use serde::{Deserialize, Serialize};
use silt_crdt::{Anchor, CharId, InodeId, InodeKind, Parent, TextOp, is_valid_name};
use silt_types::{ArtifactId, BlobHash, CloudSessionId, DeltaId, DeltaSpan, DeviceId, Frontier, GitOid, ThreadId};

/// One entry in a family's log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delta {
    pub span: DeltaSpan,
    pub parents: Frontier,
    pub thread: ThreadId,
    pub author: Author,
    /// Wall-clock time in milliseconds since the Unix epoch, for display only.
    pub time_ms: i64,
    pub op: Op,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Author {
    User,
    Agent {
        agent: String,
        session: String,
        tool_use_id: Option<String>,
    },
    /// Content brought in from git or another existing state.
    Import,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    Create {
        kind: InodeKind,
        parent: Parent,
        name: String,
        content: Content,
        executable: bool,
    },
    Move {
        inode: InodeId,
        parent: Parent,
        name: String,
    },
    Delete {
        inode: InodeId,
    },
    Text {
        inode: InodeId,
        op: TextOp,
    },
    /// Replaces a file's whole content; last writer wins.
    SetContent {
        inode: InodeId,
        content: Content,
    },
    SetExecutable {
        inode: InodeId,
        executable: bool,
    },
    Conv(ConvOp),
    Git(GitOp),
    /// The device that may write to the thread from now on (ADR 0009).
    Lease {
        device: DeviceId,
        epoch: u64,
    },
    /// The first delta of a thread forked from `from`; its parents are the
    /// point it forked at, so it shares all history up to there.
    Fork {
        from: ThreadId,
    },
    /// Brings `from`'s work into this thread (ADR 0010). The parents are
    /// this thread's head and the merged point. A pull when `mr` is `None`,
    /// otherwise the approval of that merge request.
    Merge {
        from: ThreadId,
        mr: Option<DeltaId>,
    },
    Mr(MrOp),
    Comment(CommentOp),
    /// An artifact's page data (ADR 0072); last, as stored deltas name
    /// variants by position.
    Page(PageOp),
}

/// A write to an artifact's page data (ADR 0072): its documents, the files
/// people added (ADR 0074), and the text and grids of docs and sheets
/// (ADR 0077). JSON travels as text, which postcard can store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PageOp {
    /// The document at `path` becomes `value`, JSON.
    Set {
        artifact: ArtifactId,
        path: String,
        value: String,
    },
    /// The top-level fields of `fields`, a JSON object, go into the
    /// document at `path`; a null field removes it.
    Patched {
        artifact: ArtifactId,
        path: String,
        fields: String,
    },
    Deleted {
        artifact: ArtifactId,
        path: String,
    },
    /// A file someone added to the page, kept as `_assets/<asset>`.
    Asset {
        artifact: ArtifactId,
        asset: String,
        blob: BlobHash,
        name: String,
        media_type: String,
        size: u64,
    },
    /// An edit of a doc's text.
    Text {
        artifact: ArtifactId,
        op: TextOp,
    },
    /// An edit of a sheet's rows or columns, one character per line.
    Lines {
        artifact: ArtifactId,
        sheet: u16,
        axis: Axis,
        op: TextOp,
    },
    /// A sheet's cell, as typed, at a row and a column by their ids.
    Cell {
        artifact: ArtifactId,
        sheet: u16,
        row: CharId,
        column: CharId,
        raw: String,
    },
}

/// A sheet's two kinds of lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis {
    Rows,
    Columns,
}

impl PageOp {
    pub fn artifact(&self) -> ArtifactId {
        match self {
            Self::Set { artifact, .. }
            | Self::Patched { artifact, .. }
            | Self::Deleted { artifact, .. }
            | Self::Asset { artifact, .. }
            | Self::Text { artifact, .. }
            | Self::Lines { artifact, .. }
            | Self::Cell { artifact, .. } => *artifact,
        }
    }
}

/// A comment on a thread (ADR 0011), identified by its `Added` delta's id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommentOp {
    /// `by` is the commenting user's name; deltas don't carry users.
    Added {
        target: CommentTarget,
        text: String,
        by: String,
    },
    Resolved {
        comment: DeltaId,
    },
    /// An answer under the comment `parent`; a reply to a reply names the
    /// first comment, so each comment has one flat conversation (ADR 0025).
    Replied {
        parent: DeltaId,
        text: String,
        by: String,
    },
    /// The comments went to the thread's agent as one prompt, so they aren't
    /// sent again; they stay open until resolved (ADR 0055).
    Sent {
        comments: Vec<DeltaId>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommentTarget {
    /// A conversation delta.
    Message { delta: DeltaId },
    /// Lines of a file: `start` before their first character and `end`
    /// after their last, so the comment follows the code through edits.
    Code { inode: InodeId, start: Anchor, end: Anchor },
    /// Text of an artifact's doc, anchored as code is (ADR 0077); last, as
    /// stored deltas name variants by position.
    Page { artifact: ArtifactId, start: Anchor, end: Anchor },
    /// A sheet's cell, by its row's and column's ids, so it follows the cell
    /// as rows and columns go in or out; last, as above.
    Cell { artifact: ArtifactId, sheet: u16, row: silt_crdt::CharId, column: silt_crdt::CharId },
}

/// A merge request's life (ADR 0010). It is identified by the id of its
/// `Opened` delta, which is on the fork, so the delta's parents are the
/// fork's head being offered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MrOp {
    Opened {
        target: ThreadId,
        title: String,
        summary: String,
    },
    Approved {
        mr: DeltaId,
    },
    Declined {
        mr: DeltaId,
        reason: String,
    },
    /// Keeps the request open with what to change first (ADR 0025).
    ChangesRequested {
        mr: DeltaId,
        reason: String,
    },
    /// A walk-through of the request for its reviewers, by a person or an
    /// agent on someone's machine; the latest one shows (ADR 0025).
    Guided {
        mr: DeltaId,
        guide: String,
        by: String,
    },
}

impl Op {
    /// How many ids the operation consumes.
    pub fn units(&self) -> usize {
        match self {
            Self::Text { op, .. } | Self::Page(PageOp::Text { op, .. } | PageOp::Lines { op, .. }) => op.units(),
            _ => 1,
        }
    }
}

/// Why a delta is malformed regardless of what it applies to.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShapeError {
    #[error("span of {span_len} ids does not match an operation of {units} units")]
    SpanLengthMismatch { span_len: u64, units: usize },
    #[error("the span is empty or leaves the lamport clock")]
    SpanOutOfRange,
    #[error("a text edit's range overflows")]
    TextRange,
    #[error("{0:?} is not a valid file name")]
    InvalidName(String),
    #[error("a thread can't fork from or merge itself")]
    SelfReference,
    #[error("an artifact's page data is malformed: {0}")]
    Page(&'static str),
}

impl Delta {
    /// Checks what can be checked without the family: the delta's own
    /// fields. Deltas come from other people (ADR 0016), so the server runs
    /// this before accepting one and every replica before applying one.
    /// The blob the delta refers to, which has to travel with it.
    pub fn blob(&self) -> Option<BlobHash> {
        match &self.op {
            Op::Create { content: Content::Blob(hash), .. }
            | Op::SetContent { content: Content::Blob(hash), .. }
            | Op::Conv(ConvOp::Attachment { blob: hash, .. })
            | Op::Conv(ConvOp::SessionTranslated { lines: hash, .. })
            | Op::Git(GitOp::Bundle { blob: hash, .. })
            | Op::Page(PageOp::Asset { blob: hash, .. }) => Some(*hash),
            _ => None,
        }
    }

    pub fn check_shape(&self) -> Result<(), ShapeError> {
        let units = self.op.units();
        if self.span.len != units as u64 {
            return Err(ShapeError::SpanLengthMismatch { span_len: self.span.len, units });
        }
        if !self.span.is_in_range() {
            return Err(ShapeError::SpanOutOfRange);
        }
        match &self.op {
            Op::Text { op: TextOp::Delete { pos, len }, .. } if pos.checked_add(*len).is_none() => Err(ShapeError::TextRange),
            Op::Create { name, .. } | Op::Move { name, parent: Parent::Root | Parent::Dir(_), .. } if !is_valid_name(name) => {
                Err(ShapeError::InvalidName(name.clone()))
            }
            Op::Fork { from } | Op::Merge { from, .. } if *from == self.thread => Err(ShapeError::SelfReference),
            Op::Page(op) => page_shape(op),
            _ => Ok(()),
        }
    }
}

/// Where a file's content comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Content {
    Empty,
    /// A git blob that is edited as text once someone changes it.
    GitText(GitOid),
    /// Content kept whole rather than as text: binary or very large files.
    Blob(BlobHash),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionRef {
    pub agent: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionFile {
    Main,
    Subagent(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConvOp {
    /// An agent session began in `cwd`, inside the checkout at
    /// `checkout_root`; lets another machine relocate the session.
    SessionStarted {
        session: SessionRef,
        cwd: String,
        checkout_root: String,
    },
    /// One verbatim line of the agent's own session file.
    Native {
        session: SessionRef,
        file: SessionFile,
        line: String,
    },
    /// A file the agent stored next to its session, such as a large tool result.
    Attachment {
        session: SessionRef,
        name: String,
        blob: BlobHash,
    },
    Turn {
        session: SessionRef,
        mark: TurnMark,
    },
    /// The session runs as a Claude Code cloud session (ADR 0040).
    CloudSession {
        session: SessionRef,
        id: CloudSessionId,
    },
    Title(String),
    /// Where the agent kept the session's side files (subagent transcripts,
    /// tool results) on the recording machine. Transcripts name them by
    /// absolute path, so a restore rewrites `dir` to the new location.
    SessionDir {
        session: SessionRef,
        dir: String,
    },
    /// A merge request from `from` was merged into this thread. The fork's
    /// own conversation stays in the fork; this is the link and summary.
    Merged {
        from: ThreadId,
        mr: DeltaId,
        title: String,
        summary: String,
    },
    /// `session` continues `from` under a new id, as when a fork restores
    /// its source's session; restoring `session` again needs both.
    SessionContinued {
        session: SessionRef,
        from: SessionRef,
    },
    /// `session` was started from the thread's conversation translated into
    /// its agent's format; `lines` (a blob) is what came before its own
    /// lines when restoring it.
    SessionTranslated {
        session: SessionRef,
        lines: BlobHash,
    },
    /// The thread was reverted to the turn ending with `to` (ADR 0029): its
    /// conversation after `to` and before this mark is rewound, kept in the
    /// history but no longer the conversation. Last, as stored deltas name
    /// variants by position.
    Rewound {
        to: DeltaId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnMark {
    PromptSubmitted,
    ToolStarted { tool_use_id: String, tool: String },
    ToolFinished { tool_use_id: String },
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitOp {
    /// The family's files start as this commit's tree.
    Genesis {
        commit: GitOid,
        branch: Option<String>,
    },
    Committed {
        commit: GitOid,
        branch: Option<String>,
    },
    /// A git bundle in the blob store holding `commits` and their history
    /// that the origin remote doesn't have.
    Bundle {
        commits: Vec<GitOid>,
        blob: BlobHash,
    },
    /// The session's tool call ran git, which rewrote files: a checkout,
    /// pull, merge, rebase, reset or stash. The edits credited to that call
    /// are git's, which a diff may leave out (ADR 0069).
    Rewrote {
        session: SessionRef,
        tool_use_id: String,
    },
    /// Edits recorded as the person's that git made: git moved the
    /// checkout's refs between two looks, as a `git pull` run by hand
    /// between turns does (ADR 0069).
    Made {
        spans: Vec<DeltaSpan>,
    },
}

/// What an artifact's page op must be, whoever wrote it (ADR 0016).
fn page_shape(op: &PageOp) -> Result<(), ShapeError> {
    use crate::page::{CELL_BYTES, is_document, is_document_path, is_fields};
    match op {
        PageOp::Set { path, value, .. } if !is_document_path(path) || !is_document(value) => {
            Err(ShapeError::Page("a document is a path and a JSON value of at most 64 KiB"))
        }
        PageOp::Patched { path, fields, .. } if !is_document_path(path) || !is_fields(fields) => {
            Err(ShapeError::Page("a patch is a path and a JSON object of at most 64 KiB"))
        }
        PageOp::Deleted { path, .. } if !is_document_path(path) => Err(ShapeError::Page("not a document's path")),
        PageOp::Asset { asset, name, media_type, size, .. }
            if !(1..=64).contains(&asset.len())
                || !asset.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                || name.len() > 255
                || media_type.len() > 127
                || *size > 20 << 20 =>
        {
            Err(ShapeError::Page("an asset's id, name, type or size is out of bounds"))
        }
        PageOp::Text { op: TextOp::Delete { pos, len }, .. } | PageOp::Lines { op: TextOp::Delete { pos, len }, .. }
            if pos.checked_add(*len).is_none() =>
        {
            Err(ShapeError::TextRange)
        }
        PageOp::Lines { sheet, .. } | PageOp::Cell { sheet, .. } if *sheet >= 16 => Err(ShapeError::Page("a sheet is 0 to 15")),
        PageOp::Cell { raw, .. } if raw.len() > CELL_BYTES => Err(ShapeError::Page("a cell holds at most 8 KiB")),
        _ => Ok(()),
    }
}
