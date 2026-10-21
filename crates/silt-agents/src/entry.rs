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
    },
    ToolResult {
        tool_use_id: String,
        text: String,
        is_error: bool,
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
            Self::Compaction { .. } => "compaction",
            Self::Other { kind } => kind,
        }
    }

    /// Text worth indexing for search, if any.
    pub fn searchable_text(&self) -> Option<String> {
        match self {
            Self::User { text } | Self::Assistant { text } | Self::Thinking { text } => Some(text.clone()),
            Self::ToolCall { name, input, .. } => Some(format!("{name} {input}")),
            Self::ToolResult { text, .. } => Some(text.clone()),
            Self::Compaction { summary } => summary.clone(),
            Self::Other { .. } => None,
        }
    }
}
