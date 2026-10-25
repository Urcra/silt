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
            Self::Image { .. } | Self::Usage { .. } | Self::Other { .. } => None,
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
            Self::Compaction { summary } => summary,
            Self::Image { .. } | Self::Usage { .. } | Self::Other { .. } => None,
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
