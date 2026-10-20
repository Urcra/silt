//! Claude Code: sessions live in `<config>/projects/<slug(cwd)>/<id>.jsonl`,
//! with subagent transcripts in `<id>/subagents/` and large tool outputs in
//! `<id>/tool-results/` (docs/research/probes.md).

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::entry::Entry;

pub const AGENT: &str = "claude-code";

/// Claude Code's configuration directory: `$CLAUDE_CONFIG_DIR` or `~/.claude`.
pub fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = env::var_os("CLAUDE_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude"))
}

/// The directory name Claude Code uses for sessions started in `cwd`.
pub fn project_slug(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPaths {
    pub transcript: PathBuf,
    pub subagents: PathBuf,
    pub tool_results: PathBuf,
}

pub fn session_paths(config_dir: &Path, cwd: &Path, session_id: &str) -> SessionPaths {
    let project = config_dir.join("projects").join(project_slug(cwd));
    let session_dir = project.join(session_id);
    SessionPaths {
        transcript: project.join(format!("{session_id}.jsonl")),
        subagents: session_dir.join("subagents"),
        tool_results: session_dir.join("tool-results"),
    }
}

/// Writes a session's transcripts so `claude --resume <session_id>` run in
/// `cwd` continues it, rewriting the original working directory `old_cwd`
/// to `cwd` in every line (docs/research/probes.md). Returns the paths
/// written, for tailing to continue after them.
pub fn restore(
    config_dir: &Path,
    cwd: &Path,
    session_id: &str,
    old_cwd: &str,
    main: &[String],
    subagents: &[(String, Vec<String>)],
) -> io::Result<SessionPaths> {
    let paths = session_paths(config_dir, cwd, session_id);
    let new_cwd = cwd.to_string_lossy();
    let rewrite = |lines: &[String]| -> String {
        lines
            .iter()
            .map(|line| {
                let line = if old_cwd.is_empty() { line.clone() } else { line.replace(old_cwd, &new_cwd) };
                line + "\n"
            })
            .collect()
    };
    fs::create_dir_all(paths.transcript.parent().expect("transcripts live in a project dir"))?;
    fs::write(&paths.transcript, rewrite(main))?;
    if !subagents.is_empty() {
        fs::create_dir_all(&paths.subagents)?;
        for (name, lines) in subagents {
            fs::write(paths.subagents.join(format!("{name}.jsonl")), rewrite(lines))?;
        }
    }
    Ok(paths)
}

/// The conversation steps recorded in one transcript line.
pub fn normalize(line: &str) -> Vec<Entry> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return vec![Entry::Other {
            kind: "unparseable".into(),
        }];
    };
    let kind = value.get("type").and_then(Value::as_str).unwrap_or("unknown");
    match kind {
        "user" if value.get("isCompactSummary").and_then(Value::as_bool) == Some(true) => {
            vec![Entry::Compaction {
                summary: Some(message_text(&value)),
            }]
        }
        "system" if value.get("subtype").and_then(Value::as_str) == Some("compact_boundary") => {
            vec![Entry::Compaction { summary: None }]
        }
        "user" | "assistant" => message_entries(kind, &value),
        other => vec![Entry::Other { kind: other.to_owned() }],
    }
}

fn message_entries(role: &str, value: &Value) -> Vec<Entry> {
    let content = value.pointer("/message/content");
    let blocks = match content {
        Some(Value::String(text)) => return vec![text_entry(role, text.clone())],
        Some(Value::Array(blocks)) => blocks,
        _ => return Vec::new(),
    };
    blocks
        .iter()
        .filter_map(|block| match block.get("type").and_then(Value::as_str)? {
            "text" => Some(text_entry(role, string_field(block, "text"))),
            "thinking" => Some(Entry::Thinking {
                text: string_field(block, "thinking"),
            })
            .filter(|entry| entry.searchable_text().is_some_and(|text| !text.is_empty())),
            "tool_use" => Some(Entry::ToolCall {
                id: string_field(block, "id"),
                name: string_field(block, "name"),
                input: block.get("input").cloned().unwrap_or(Value::Null),
            }),
            "tool_result" => Some(Entry::ToolResult {
                tool_use_id: string_field(block, "tool_use_id"),
                text: block_text(block.get("content")),
                is_error: block.get("is_error").and_then(Value::as_bool).unwrap_or(false),
            }),
            other => Some(Entry::Other {
                kind: format!("{role}_{other}"),
            }),
        })
        .collect()
}

fn text_entry(role: &str, text: String) -> Entry {
    if role == "user" {
        Entry::User { text }
    } else {
        Entry::Assistant { text }
    }
}

fn message_text(value: &Value) -> String {
    block_text(value.pointer("/message/content"))
}

fn block_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn string_field(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

/// What Claude Code sends a hook on stdin.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HookInput {
    pub session_id: String,
    pub hook_event_name: String,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub transcript_path: Option<PathBuf>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_use_id: Option<String>,
}

/// Hook events Silt listens to.
pub const HOOK_EVENTS: [&str; 5] = ["UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop", "SessionStart"];

/// Settings JSON for `claude --settings` that runs `command <event>` for each
/// hook event Silt needs, with the hook input on stdin.
pub fn hook_settings(command: &str) -> String {
    let mut hooks = serde_json::Map::new();
    for event in HOOK_EVENTS {
        hooks.insert(
            event.to_owned(),
            json!([{
                "matcher": "*",
                "hooks": [{ "type": "command", "command": format!("{command} {event}"), "timeout": 30 }]
            }]),
        );
    }
    json!({ "hooks": hooks }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_replaces_everything_but_ascii_alphanumerics() {
        assert_eq!(
            project_slug(Path::new("/home/u/my_repo.v2/src")),
            "-home-u-my-repo-v2-src"
        );
    }

    #[test]
    fn session_paths_follow_claude_layout() {
        let paths = session_paths(Path::new("/c"), Path::new("/repo"), "abc");
        assert_eq!(paths.transcript, Path::new("/c/projects/-repo/abc.jsonl"));
        assert_eq!(paths.subagents, Path::new("/c/projects/-repo/abc/subagents"));
        assert_eq!(paths.tool_results, Path::new("/c/projects/-repo/abc/tool-results"));
    }

    #[test]
    fn normalizes_messages_tools_and_compaction() {
        let user = r#"{"type":"user","message":{"role":"user","content":"Fix the lexer"}}"#;
        assert_eq!(normalize(user), vec![Entry::User { text: "Fix the lexer".into() }]);

        let assistant = r#"{"type":"assistant","message":{"role":"assistant","content":[
            {"type":"thinking","thinking":"look at lexer.rs","signature":"x"},
            {"type":"thinking","thinking":"","signature":"redacted"},
            {"type":"text","text":"On it."},
            {"type":"tool_use","id":"toolu_1","name":"Edit","input":{"file_path":"src/lexer.rs"}}]}}"#;
        assert_eq!(
            normalize(assistant),
            vec![
                Entry::Thinking { text: "look at lexer.rs".into() },
                Entry::Assistant { text: "On it.".into() },
                Entry::ToolCall {
                    id: "toolu_1".into(),
                    name: "Edit".into(),
                    input: json!({"file_path": "src/lexer.rs"})
                },
            ]
        );

        let result = r#"{"type":"user","message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"text","text":"ok"}],"is_error":false}]}}"#;
        assert_eq!(
            normalize(result),
            vec![Entry::ToolResult {
                tool_use_id: "toolu_1".into(),
                text: "ok".into(),
                is_error: false
            }]
        );

        let boundary = r#"{"type":"system","subtype":"compact_boundary","content":"Conversation compacted"}"#;
        assert_eq!(normalize(boundary), vec![Entry::Compaction { summary: None }]);
        let summary = r#"{"type":"user","isCompactSummary":true,"message":{"role":"user","content":"We were fixing the lexer."}}"#;
        assert_eq!(
            normalize(summary),
            vec![Entry::Compaction {
                summary: Some("We were fixing the lexer.".into())
            }]
        );
        assert_eq!(
            normalize(r#"{"type":"file-history-snapshot"}"#),
            vec![Entry::Other {
                kind: "file-history-snapshot".into()
            }]
        );
    }

    #[test]
    fn restore_relocates_the_session() {
        let config = tempfile::tempdir().expect("temp dir");
        let paths = restore(
            config.path(),
            Path::new("/home/b/repo"),
            "abc",
            "/home/a/repo",
            &[r#"{"cwd":"/home/a/repo","type":"user"}"#.to_owned()],
            &[("agent-1".to_owned(), vec![r#"{"cwd":"/home/a/repo/src"}"#.to_owned()])],
        )
        .expect("restores");
        assert_eq!(paths.transcript, config.path().join("projects/-home-b-repo/abc.jsonl"));
        assert_eq!(
            fs::read_to_string(&paths.transcript).expect("transcript"),
            "{\"cwd\":\"/home/b/repo\",\"type\":\"user\"}\n"
        );
        assert_eq!(
            fs::read_to_string(paths.subagents.join("agent-1.jsonl")).expect("subagent"),
            "{\"cwd\":\"/home/b/repo/src\"}\n"
        );
    }

    #[test]
    fn hook_input_parses_tool_events() {
        let input: HookInput = serde_json::from_str(
            r#"{"session_id":"s","hook_event_name":"PreToolUse","cwd":"/repo",
                "transcript_path":"/c/x.jsonl","tool_name":"Edit","tool_use_id":"toolu_9","tool_input":{}}"#,
        )
        .expect("parses");
        assert_eq!(input.tool_use_id.as_deref(), Some("toolu_9"));
        let settings: Value = serde_json::from_str(&hook_settings("silt hook")).expect("valid json");
        assert_eq!(
            settings.pointer("/hooks/PostToolUse/0/hooks/0/command"),
            Some(&json!("silt hook PostToolUse"))
        );
    }
}
