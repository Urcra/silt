use crate::Entry;

const TOOL_TEXT_LIMIT: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

/// A conversation as alternating plain-text messages, for writing it in
/// another agent's format. Tool calls and results become text, since the
/// other agent has different tools, and thinking is dropped because it is
/// signed for the account that produced it.
pub fn translated_text(entries: &[Entry]) -> Vec<(Role, String)> {
    let mut messages: Vec<(Role, String)> = Vec::new();
    let mut push = |role: Role, text: String| {
        if text.trim().is_empty() {
            return;
        }
        match messages.last_mut() {
            Some((last, existing)) if *last == role => {
                existing.push_str("\n\n");
                existing.push_str(&text);
            }
            _ => messages.push((role, text)),
        }
    };
    for entry in entries {
        match entry {
            Entry::User { text } => push(Role::User, text.clone()),
            Entry::Assistant { text } => push(Role::Assistant, text.clone()),
            Entry::ToolCall { name, input, .. } => push(Role::Assistant, format!("[Called tool {name}: {}]", clip(&input.to_string()))),
            Entry::ToolResult { text, is_error, .. } => {
                let label = if *is_error { "Tool error" } else { "Tool result" };
                push(Role::Assistant, format!("[{label}: {}]", clip(text)));
            }
            Entry::Compaction { summary: Some(summary) } => {
                push(Role::User, format!("Summary of the earlier conversation:\n{summary}"));
            }
            Entry::Thinking { .. } | Entry::Compaction { summary: None } | Entry::Other { .. } => {}
        }
    }
    if messages.first().is_some_and(|(role, _)| *role == Role::Assistant) {
        messages.insert(0, (Role::User, "(This conversation was continued from another agent.)".to_owned()));
    }
    messages
}

fn clip(text: &str) -> String {
    if text.chars().count() <= TOOL_TEXT_LIMIT {
        return text.to_owned();
    }
    let kept: String = text.chars().take(TOOL_TEXT_LIMIT).collect();
    format!("{kept}… (cut; the full text is in Silt's history)")
}

/// The first prompt of a new session that continues a conversation recorded
/// with another agent: as much of the end of it as fits in `budget`
/// characters, and where to read the rest. The prompt is an agent's
/// command-line argument, which can't be longer than the system allows, so
/// even the last message is cut to the budget.
pub fn render_transcript(entries: &[Entry], thread: &str, from_agent: &str, budget: usize) -> String {
    let messages = translated_text(entries);
    let mut kept: Vec<String> = Vec::new();
    let mut used = 0;
    let mut cut = false;
    for (role, text) in messages.iter().rev() {
        let speaker = match role {
            Role::User => "User",
            Role::Assistant => "Assistant",
        };
        let block = format!("## {speaker}\n\n{}\n", text.trim());
        let length = block.chars().count();
        if used + length > budget {
            if kept.is_empty() {
                kept.push(end_of(speaker, text.trim(), budget));
            }
            cut = true;
            break;
        }
        used += length;
        kept.push(block);
    }
    kept.reverse();
    let mut prompt = format!(
        "You are continuing a conversation that was recorded with {from_agent} in Silt thread {thread}. \
         The code in this directory is where that conversation left it."
    );
    if cut {
        prompt.push_str(" Only the end of the conversation is below; read the rest with the silt_history_read tool.");
    } else {
        prompt.push_str(" The silt_history_read and silt_history_search tools can read it again later.");
    }
    prompt.push_str(" Read it, then reply briefly that you are ready to continue and wait for the next instruction.\n\n# Conversation so far\n\n");
    prompt.push_str(&kept.join("\n"));
    prompt
}

/// A message block holding about the last `budget` characters of `text`.
fn end_of(speaker: &str, text: &str, budget: usize) -> String {
    const MARK: &str = "(The start of this message is cut.) …";
    let room = budget.saturating_sub(speaker.len() + MARK.len() + 6);
    let skip = text.chars().count().saturating_sub(room);
    let end: String = text.chars().skip(skip).collect();
    format!("## {speaker}\n\n{MARK}{end}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn conversation() -> Vec<Entry> {
        vec![
            Entry::Thinking { text: "secret".into() },
            Entry::Assistant { text: "Hello".into() },
            Entry::User { text: "Fix the lexer".into() },
            Entry::Assistant { text: "Looking.".into() },
            Entry::ToolCall {
                id: "t1".into(),
                name: "Read".into(),
                input: json!({"file_path": "lexer.rs"}),
            },
            Entry::ToolResult {
                tool_use_id: "t1".into(),
                text: "fn lex() {}".into(),
                is_error: false,
            },
            Entry::Other { kind: "hook".into() },
            Entry::User { text: "Thanks".into() },
        ]
    }

    #[test]
    fn translation_alternates_plain_messages() {
        let messages = translated_text(&conversation());
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0].0, Role::User);
        assert_eq!(messages[1], (Role::Assistant, "Hello".into()));
        assert_eq!(
            messages[3],
            (
                Role::Assistant,
                "Looking.\n\n[Called tool Read: {\"file_path\":\"lexer.rs\"}]\n\n[Tool result: fn lex() {}]".into()
            )
        );
        assert!(!messages.iter().any(|(_, text)| text.contains("secret")));
    }

    #[test]
    fn rendering_keeps_the_end_within_the_budget() {
        let full = render_transcript(&conversation(), "abc", "Claude Code", 10_000);
        assert!(full.contains("## User\n\nFix the lexer") && full.contains("thread abc"));
        assert!(!full.contains("Only the end"));
        let short = render_transcript(&conversation(), "abc", "Claude Code", 20);
        assert!(short.contains("Only the end") && short.contains("## User\n\nThanks") && !short.contains("Fix the lexer"));
    }

    #[test]
    fn a_last_message_longer_than_the_budget_keeps_its_end() {
        let long = Entry::User {
            text: format!("start {} end", "é".repeat(100_000)),
        };
        let budget = 1_000;
        let prompt = render_transcript(&[long], "abc", "Claude Code", budget);
        let (_, conversation) = prompt.split_once("# Conversation so far\n\n").expect("has the conversation");
        assert!(conversation.chars().count() <= budget, "{}", conversation.chars().count());
        assert!(prompt.contains("Only the end") && prompt.contains("is cut.) …") && prompt.ends_with("é end\n"));
        assert!(!prompt.contains("start é"));
    }
}
