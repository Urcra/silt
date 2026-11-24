use serde::{Deserialize, Serialize};

/// One step of a conversation, independent of which agent recorded it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Entry {
    User {
        text: String,
    },
    Assistant {
        text: String,
    },
    Thinking {
        text: String,
    },
    ToolCall {
        id: String,
        name: String,
        input: serde_json::Value,
        /// The kind its agent gave it, where it gives one (ACP agents do).
        #[serde(default)]
        kind: Option<String>,
    },
    ToolResult {
        tool_use_id: String,
        text: String,
        is_error: bool,
    },
    /// An image a prompt or a tool's result carries, base64-encoded as the
    /// agent recorded it.
    Image {
        media_type: String,
        data: String,
        /// A prompt's file kept as a blob, with its previews (ADR 0120).
        stored: Option<silt_proto::view::StoredMedia>,
    },
    /// What a message of the agent's cost, in tokens; an agent may record
    /// one message over several lines, each naming it.
    Usage {
        message: String,
        input: u64,
        output: u64,
    },
    /// The agent's plan as it now stands: each step and its status
    /// (`pending`, `in_progress` or `completed`).
    Plan {
        steps: Vec<(String, String)>,
    },
    /// The agent compacted its context here; earlier entries stay available.
    Compaction {
        summary: Option<String>,
    },
    /// The agent went on by itself, without a prompt, as Claude does once a
    /// background task it started ends: what follows is a turn it opened.
    Woke {
        text: String,
    },
    /// News of a subagent or a background command the agent started.
    Task(Task),
    /// What the agent showed the person through Silt (ADR 0068).
    Artifact(Box<silt_proto::view::ArtifactView>),
    /// Anything else the agent records (hooks, settings, file snapshots).
    Other {
        kind: String,
    },
}

impl Entry {
    /// A record that isn't the JSON its agent writes.
    pub fn unparseable() -> Self {
        Self::Other { kind: "unparseable".into() }
    }

    /// Short label for display and the search index.
    pub fn kind(&self) -> &str {
        match self {
            Self::User { .. } => "user",
            Self::Assistant { .. } => "assistant",
            Self::Thinking { .. } => "thinking",
            Self::ToolCall { .. } => "tool_call",
            Self::ToolResult { .. } => "tool_result",
            Self::Image { .. } => "image",
            Self::Usage { .. } => "usage",
            Self::Plan { .. } => "plan",
            Self::Compaction { .. } => "compaction",
            Self::Woke { .. } => "wake",
            Self::Task(_) => "task",
            Self::Artifact(_) => "artifact",
            Self::Other { kind } => kind,
        }
    }

    /// Text worth indexing for search, if any.
    pub fn searchable_text(&self) -> Option<String> {
        match self {
            Self::User { text } | Self::Assistant { text } | Self::Thinking { text } | Self::ToolResult { text, .. } => {
                Some(text.clone())
            }
            Self::ToolCall { name, input, .. } => Some(call_text(name, input)),
            Self::Plan { steps } => Some(plan_text(steps)),
            Self::Compaction { summary } => summary.clone(),
            Self::Task(task) => task.summary.clone(),
            Self::Artifact(artifact) => Some(artifact.title.clone()),
            Self::Image { .. } | Self::Usage { .. } | Self::Woke { .. } | Self::Other { .. } => None,
        }
    }

    /// `searchable_text`, taking the entry rather than copying its text,
    /// which for a tool's output can be long.
    pub fn into_searchable_text(self) -> Option<String> {
        match self {
            Self::User { text } | Self::Assistant { text } | Self::Thinking { text } | Self::ToolResult { text, .. } => {
                Some(text)
            }
            Self::ToolCall { name, input, .. } => Some(call_text(&name, &input)),
            Self::Plan { steps } => Some(plan_text(&steps)),
            Self::Compaction { summary } | Self::Task(Task { summary, .. }) => summary,
            Self::Artifact(artifact) => Some(artifact.title),
            Self::Image { .. } | Self::Usage { .. } | Self::Woke { .. } | Self::Other { .. } => None,
        }
    }
}

/// Work an agent started that runs beside its turn, as one record says it:
/// it started, or it ended and how. Later news of the same `id` says more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// The agent's id for it.
    pub id: String,
    /// The tool call that started it, when the record names it.
    pub call: Option<String>,
    /// Which kind, when the record says.
    pub kind: Option<TaskKind>,
    pub status: TaskStatus,
    /// A subagent's side transcript, by name.
    pub side: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskKind {
    Subagent,
    /// A command left running while the agent goes on.
    Background,
}

impl TaskKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Subagent => "subagent",
            Self::Background => "background",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Running,
    Completed,
    Failed,
    /// Stopped before it finished, by the agent or the person.
    Stopped,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }

    /// A status as agents word it: `completed`, `failed`, `killed`…
    pub fn parse(said: &str) -> Self {
        match said.to_ascii_lowercase().as_str() {
            "completed" | "complete" | "done" | "success" | "succeeded" | "finished" => Self::Completed,
            "failed" | "error" | "errored" | "failure" => Self::Failed,
            "killed" | "stopped" | "cancelled" | "canceled" | "aborted" | "interrupted" | "terminated" => Self::Stopped,
            _ => Self::Running,
        }
    }
}

/// A tool call as a search finds it: its name and the values it was given,
/// as `Bash cargo test -p silt-store`, not its input's JSON, whose keys and
/// escapes would match every call.
fn call_text(name: &str, input: &serde_json::Value) -> String {
    fn values(value: &serde_json::Value, into: &mut Vec<String>) {
        match value {
            serde_json::Value::String(text) => into.push(text.clone()),
            serde_json::Value::Number(number) => into.push(number.to_string()),
            serde_json::Value::Array(items) => items.iter().for_each(|item| values(item, into)),
            serde_json::Value::Object(fields) => fields.values().for_each(|field| values(field, into)),
            serde_json::Value::Bool(_) | serde_json::Value::Null => {}
        }
    }
    let mut said = vec![name.to_owned()];
    values(input, &mut said);
    said.join(" ")
}

fn plan_text(steps: &[(String, String)]) -> String {
    steps.iter().map(|(step, _)| step.as_str()).collect::<Vec<_>>().join("\n")
}
