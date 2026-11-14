//! A conversation with a local model: what is sent each turn, the session
//! file it's kept in, and compaction as it nears the model's context.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const SYSTEM: &str = "You are Silt's local assistant, answering on a model that runs on this machine. \
Answer plainly and to the point, in Markdown where it helps.";

/// Messages kept as they are when the rest is summarized.
const KEPT_ON_COMPACTION: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub text: String,
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

    fn system(&self) -> String {
        match &self.summary {
            Some(summary) => format!("{SYSTEM}\n\nWhat was said earlier in this conversation, summarized:\n{summary}"),
            None => SYSTEM.to_owned(),
        }
    }

    fn wire(&self, messages: &[Message]) -> Vec<Value> {
        let mut wire = vec![json!({"role": "system", "content": self.system()})];
        wire.extend(messages.iter().map(|message| json!({"role": message.role, "content": message.text})));
        wire
    }

    /// The next turn's request, streamed, with its usage at the end.
    pub fn request(&self, model: &str) -> Value {
        json!({
            "model": model,
            "messages": self.wire(&self.messages),
            "stream": true,
            "stream_options": {"include_usage": true},
        })
    }

    /// Whether the prompt took four fifths of the context or more.
    pub fn should_compact(&self, prompt_tokens: u64, context: u64) -> bool {
        self.messages.len() > KEPT_ON_COMPACTION && context > 0 && prompt_tokens.saturating_mul(5) >= context.saturating_mul(4)
    }

    /// A request for a summary of all but the last few messages.
    pub fn compaction_request(&self, model: &str) -> Option<Value> {
        let dropped = self.messages.len().checked_sub(KEPT_ON_COMPACTION).filter(|dropped| *dropped > 0)?;
        let mut messages = self.wire(&self.messages[..dropped]);
        messages.push(json!({"role": "user", "content": "Summarize the conversation so far for yourself to carry on from: \
            what was asked, what was found and decided, and what's still open. Keep names, paths and numbers exact."}));
        Some(json!({"model": model, "messages": messages, "stream": false}))
    }

    /// The entry a summary makes.
    pub fn compacted(&self, summary: String) -> Entry {
        let summary = match &self.summary {
            Some(earlier) => format!("{earlier}\n\n{summary}"),
            None => summary,
        };
        Entry::Compacted { summary, kept: KEPT_ON_COMPACTION.min(self.messages.len()) }
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
        Entry::Said(Message { role, text: text.to_owned() })
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
        let request = conversation.request("gpt-oss-20b");
        assert_eq!(request["messages"][0]["role"], "system");
        assert_eq!(request["messages"][1], json!({"role": "user", "content": "hi"}));
        assert_eq!(request["messages"][2], json!({"role": "assistant", "content": "hello"}));
        assert_eq!(request["stream"], true);
    }

    #[test]
    fn compaction_keeps_the_last_messages_and_summarizes_the_rest() {
        let mut conversation = Conversation::default();
        for index in 0..6 {
            conversation.apply(said(if index % 2 == 0 { Role::User } else { Role::Assistant }, &format!("m{index}")));
        }
        assert!(conversation.should_compact(800, 1000));
        assert!(!conversation.should_compact(700, 1000));
        let request = conversation.compaction_request("m").expect("something to summarize");
        let messages = request["messages"].as_array().expect("messages");
        assert_eq!(messages.len(), 1 + 2 + 1, "the system prompt, the two dropped, the ask");
        conversation.apply(conversation.compacted("they said m0 and m1".into()));
        assert_eq!(
            conversation.messages.iter().map(|message| message.text.as_str()).collect::<Vec<_>>(),
            ["m2", "m3", "m4", "m5"]
        );
        assert_eq!(conversation.said.len(), 6, "a load still replays everything");
        let next = conversation.request("m");
        assert!(next["messages"][0]["content"].as_str().is_some_and(|system| system.contains("they said m0 and m1")));
        conversation.apply(conversation.compacted("then more".into()));
        assert_eq!(conversation.summary.as_deref(), Some("they said m0 and m1\n\nthen more"));
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
