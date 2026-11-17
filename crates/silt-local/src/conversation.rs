//! A conversation with a local model: what is sent each turn, the session
//! file it's kept in, and compaction as it nears the model's context.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const SYSTEM: &str = "You are Silt's local assistant, answering on a model that runs on this machine. \
Answer plainly and to the point, in Markdown where it helps.";

pub const SYSTEM_WITH_TOOLS: &str = "You are Silt's local assistant, a coding agent on a model that runs on this machine. \
You work in the project folder you were started in, with tools to read, search, edit and write files there and to run \
commands. Look before you change anything, change only what the task needs, and say plainly what you did. Paths are \
relative to the project folder.";

/// Messages kept as they are when the rest is summarized, at least.
const KEPT_ON_COMPACTION: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    /// A tool's result, answering one of the assistant's calls.
    Tool,
}

/// A tool the model asked to call, as it asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    pub id: String,
    pub name: String,
    /// JSON, as the model wrote it; it may not parse.
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calls: Vec<Call>,
    /// For a tool's result, the call it answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answers: Option<String>,
}

impl Message {
    pub fn said(role: Role, text: impl Into<String>) -> Self {
        Self { role, text: text.into(), calls: Vec::new(), answers: None }
    }

    pub fn result(call: &str, text: impl Into<String>) -> Self {
        Self { role: Role::Tool, text: text.into(), calls: Vec::new(), answers: Some(call.to_owned()) }
    }

    fn wire(&self) -> Value {
        match self.role {
            Role::Tool => json!({"role": "tool", "tool_call_id": self.answers, "content": self.text}),
            Role::User => json!({"role": "user", "content": self.text}),
            Role::Assistant if self.calls.is_empty() => json!({"role": "assistant", "content": self.text}),
            Role::Assistant => json!({
                "role": "assistant",
                "content": self.text,
                "tool_calls": self.calls.iter().map(|call| json!({
                    "id": call.id,
                    "type": "function",
                    "function": {"name": call.name, "arguments": call.arguments},
                })).collect::<Vec<_>>(),
            }),
        }
    }
}

/// A line of a session file. A session is these, in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Entry {
    Said(Message),
    /// The messages before the last `kept` were summarized as `summary`.
    Compacted {
        summary: String,
        kept: usize,
    },
    Model {
        id: String,
    },
    Mode {
        id: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Conversation {
    pub messages: Vec<Message>,
    /// What the messages compaction dropped said.
    pub summary: Option<String>,
    /// Everything said, compacted or not, as it's replayed on a load.
    pub said: Vec<Message>,
    pub model: Option<String>,
    pub mode: Option<String>,
}

impl Conversation {
    /// The conversation a session file holds; unreadable lines are skipped.
    pub fn replay(lines: &str) -> Self {
        let mut conversation = Self::default();
        for entry in lines.lines().filter_map(|line| serde_json::from_str::<Entry>(line).ok()) {
            conversation.apply(entry);
        }
        conversation
    }

    pub fn apply(&mut self, entry: Entry) {
        match entry {
            Entry::Said(message) => {
                self.said.push(message.clone());
                self.messages.push(message);
            }
            Entry::Compacted { summary, kept } => {
                let dropped = self.messages.len().saturating_sub(kept);
                self.messages.drain(..dropped);
                self.summary = Some(summary);
            }
            Entry::Model { id } => self.model = Some(id),
            Entry::Mode { id } => self.mode = Some(id),
        }
    }

    fn system(&self, tools: bool) -> String {
        let base = if tools { SYSTEM_WITH_TOOLS } else { SYSTEM };
        match &self.summary {
            Some(summary) => format!("{base}\n\nWhat was said earlier in this conversation, summarized:\n{summary}"),
            None => base.to_owned(),
        }
    }

    fn wire(&self, messages: &[Message], tools: bool) -> Vec<Value> {
        let mut wire = vec![json!({"role": "system", "content": self.system(tools)})];
        wire.extend(messages.iter().map(Message::wire));
        wire
    }

    /// The next request, streamed, with its usage at the end, offering
    /// `tools` when there are any.
    pub fn request(&self, model: &str, tools: Option<&Value>) -> Value {
        let mut request = json!({
            "model": model,
            "messages": self.wire(&self.messages, tools.is_some()),
            "stream": true,
            "stream_options": {"include_usage": true},
        });
        if let Some(tools) = tools {
            request["tools"] = tools.clone();
        }
        request
    }

    /// Where the kept messages start: at least the last few, from a prompt,
    /// so no tool's result is kept without the call it answers.
    fn kept_from(&self) -> Option<usize> {
        let mut from = self.messages.len().checked_sub(KEPT_ON_COMPACTION)?;
        while from > 0 && self.messages[from].role != Role::User {
            from -= 1;
        }
        (from > 0).then_some(from)
    }

    /// Whether the prompt took four fifths of the context or more.
    pub fn should_compact(&self, prompt_tokens: u64, context: u64) -> bool {
        self.kept_from().is_some() && context > 0 && prompt_tokens.saturating_mul(5) >= context.saturating_mul(4)
    }

    /// A request for a summary of what isn't kept.
    pub fn compaction_request(&self, model: &str) -> Option<Value> {
        let dropped = self.kept_from()?;
        let mut messages = self.wire(&self.messages[..dropped], false);
        messages.push(json!({"role": "user", "content": "Summarize the conversation so far for yourself to carry on from: \
            what was asked, what was found and decided, what was changed, and what's still open. Keep names, paths and \
            numbers exact."}));
        Some(json!({"model": model, "messages": messages, "stream": false}))
    }

    /// The entry a summary makes.
    pub fn compacted(&self, summary: String) -> Entry {
        let summary = match &self.summary {
            Some(earlier) => format!("{earlier}\n\n{summary}"),
            None => summary,
        };
        let kept = self.kept_from().map_or(self.messages.len(), |from| self.messages.len() - from);
        Entry::Compacted { summary, kept }
    }
}

/// A session id this agent makes: 32 hex digits, so it's safe as a file name.
pub fn is_session_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(role: Role, text: &str) -> Entry {
        Entry::Said(Message::said(role, text))
    }

    #[test]
    fn a_session_file_replays_to_the_conversation() {
        let lines: String = [
            said(Role::User, "hi"),
            said(Role::Assistant, "hello"),
            Entry::Model { id: "gpt-oss-20b".into() },
            Entry::Mode { id: "chat".into() },
        ]
        .iter()
        .map(|entry| serde_json::to_string(entry).expect("serializes") + "\n")
        .collect::<String>()
            + "not json\n";
        let conversation = Conversation::replay(&lines);
        assert_eq!(conversation.messages.len(), 2);
        assert_eq!(conversation.model.as_deref(), Some("gpt-oss-20b"));
        assert_eq!(conversation.mode.as_deref(), Some("chat"));
        let request = conversation.request("gpt-oss-20b", None);
        assert_eq!(request["messages"][0]["role"], "system");
        assert_eq!(request["messages"][1], json!({"role": "user", "content": "hi"}));
        assert_eq!(request["messages"][2], json!({"role": "assistant", "content": "hello"}));
        assert_eq!(request["stream"], true);
        assert!(request.get("tools").is_none());
    }

    #[test]
    fn tool_calls_and_results_go_out_as_the_api_has_them() {
        let mut conversation = Conversation::default();
        conversation.apply(said(Role::User, "read it"));
        let call = Call { id: "c1".into(), name: "read_file".into(), arguments: r#"{"path":"a"}"#.into() };
        conversation.apply(Entry::Said(Message { role: Role::Assistant, text: String::new(), calls: vec![call], answers: None }));
        conversation.apply(Entry::Said(Message::result("c1", "1: x")));
        let tools = json!([{"type": "function"}]);
        let request = conversation.request("m", Some(&tools));
        assert_eq!(request["tools"], tools);
        assert!(request["messages"][0]["content"].as_str().is_some_and(|system| system.contains("tools")));
        assert_eq!(request["messages"][2]["tool_calls"][0]["function"]["name"], "read_file");
        assert_eq!(request["messages"][3], json!({"role": "tool", "tool_call_id": "c1", "content": "1: x"}));
    }

    #[test]
    fn compaction_keeps_the_last_exchanges_whole() {
        let mut conversation = Conversation::default();
        for index in 0..6 {
            conversation.apply(said(if index % 2 == 0 { Role::User } else { Role::Assistant }, &format!("m{index}")));
        }
        assert!(conversation.should_compact(800, 1000));
        assert!(!conversation.should_compact(700, 1000));
        let request = conversation.compaction_request("m").expect("something to summarize");
        assert_eq!(request["messages"].as_array().map(Vec::len), Some(1 + 2 + 1), "the system prompt, the two dropped, the ask");
        conversation.apply(conversation.compacted("they said m0 and m1".into()));
        assert_eq!(
            conversation.messages.iter().map(|message| message.text.as_str()).collect::<Vec<_>>(),
            ["m2", "m3", "m4", "m5"]
        );
        assert_eq!(conversation.said.len(), 6, "a load still replays everything");
        let next = conversation.request("m", None);
        assert!(next["messages"][0]["content"].as_str().is_some_and(|system| system.contains("they said m0 and m1")));

        let mut tooled = Conversation::default();
        tooled.apply(said(Role::User, "u0"));
        tooled.apply(said(Role::Assistant, "a0"));
        tooled.apply(said(Role::User, "u1"));
        let call = Call { id: "c".into(), name: "list".into(), arguments: "{}".into() };
        tooled.apply(Entry::Said(Message { role: Role::Assistant, text: String::new(), calls: vec![call], answers: None }));
        tooled.apply(Entry::Said(Message::result("c", "x")));
        tooled.apply(said(Role::Assistant, "a1"));
        tooled.apply(conversation_compacted(&tooled));
        assert_eq!(tooled.messages[0].text, "u1", "kept from the prompt, with the call before its result");
    }

    fn conversation_compacted(conversation: &Conversation) -> Entry {
        conversation.compacted("earlier".into())
    }

    #[test]
    fn a_short_conversation_isnt_compacted() {
        let mut conversation = Conversation::default();
        conversation.apply(said(Role::User, "a"));
        assert!(!conversation.should_compact(1000, 1000));
        assert_eq!(conversation.compaction_request("m"), None);
    }

    #[test]
    fn session_ids_are_plain_hex() {
        assert!(is_session_id(&"a".repeat(32)));
        assert!(!is_session_id("../../etc/passwd"));
        assert!(!is_session_id(&"A".repeat(32)));
    }
}
