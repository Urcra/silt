use serde::{Deserialize, Serialize};
use silt_crdt::{InodeId, InodeKind, Parent, TextOp};
use silt_types::{BlobHash, DeltaId, DeltaSpan, DeviceId, Frontier, GitOid, ThreadId};

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
    Lease { device: DeviceId, epoch: u64 },
    /// The first delta of a thread forked from `from`; its parents are the
    /// point it forked at, so it shares all history up to there.
    Fork { from: ThreadId },
    /// Brings `from`'s work into this thread (ADR 0010). The parents are
    /// this thread's head and the merged point. A pull when `mr` is `None`,
    /// otherwise the approval of that merge request.
    Merge { from: ThreadId, mr: Option<DeltaId> },
    Mr(MrOp),
}

/// A merge request's life (ADR 0010). It is identified by the id of its
/// `Opened` delta, which is on the fork, so the delta's parents are the
/// fork's head being offered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MrOp {
    Opened { target: ThreadId, title: String, summary: String },
    Approved { mr: DeltaId },
    Declined { mr: DeltaId, reason: String },
}

impl Op {
    /// How many ids the operation consumes.
    pub fn units(&self) -> usize {
        match self {
            Self::Text { op, .. } => op.units(),
            _ => 1,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    Title(String),
    /// Where the agent kept the session's side files (subagent transcripts,
    /// tool results) on the recording machine. Transcripts name them by
    /// absolute path, so a restore rewrites `dir` to the new location.
    SessionDir { session: SessionRef, dir: String },
    /// A merge request from `from` was merged into this thread. The fork's
    /// own conversation stays in the fork; this is the link and summary.
    Merged {
        from: ThreadId,
        mr: DeltaId,
        title: String,
        summary: String,
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
    Genesis { commit: GitOid, branch: Option<String> },
    Committed { commit: GitOid, branch: Option<String> },
    /// A git bundle in the blob store holding `commits` and their history
    /// that the origin remote doesn't have.
    Bundle { commits: Vec<GitOid>, blob: BlobHash },
}
