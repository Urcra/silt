//! Claude Code: sessions live in `<config>/projects/<slug(cwd)>/<id>.jsonl`,
//! with subagent transcripts in `<id>/subagents/` and large tool outputs in
//! `<id>/tool-results/` (docs/research/probes.md).

use std::env;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

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
    /// Holds the session's side files: `subagents/` and `tool-results/`.
    pub session_dir: PathBuf,
    pub subagents: PathBuf,
    pub tool_results: PathBuf,
}

pub fn session_paths(config_dir: &Path, cwd: &Path, session_id: &str) -> SessionPaths {
    let project = config_dir.join("projects").join(project_slug(cwd));
    let session_dir = project.join(session_id);
    SessionPaths {
        transcript: project.join(format!("{session_id}.jsonl")),
        session_dir: session_dir.clone(),
        subagents: session_dir.join("subagents"),
        tool_results: session_dir.join("tool-results"),
    }
}

/// A session as recorded elsewhere, possibly continued across machines
/// and forks, to be written back under one id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordedSession {
    /// The id to restore it as.
    pub id: String,
    /// Ids, working directories and session directories it was recorded
    /// under; each is rewritten to the new one.
    pub former_ids: Vec<String>,
    pub former_cwds: Vec<String>,
    /// Checkout roots, rewritten to the new checkout root after the cwds.
    pub former_roots: Vec<String>,
    pub former_dirs: Vec<String>,
    pub main: Vec<String>,
    pub subagents: Vec<(String, Vec<String>)>,
    /// Side files by path relative to the session directory.
    pub attachments: Vec<(String, Vec<u8>)>,
}

/// Writes a session's files so `claude --resume <id>` run in `cwd` continues
/// it, rewriting every former session directory, working directory and
/// session id to the new ones (docs/research/probes.md). Returns the paths
/// written, for tailing to continue after them.
pub fn restore(config_dir: &Path, root: &Path, cwd: &Path, session: &RecordedSession) -> io::Result<SessionPaths> {
    let paths = session_paths(config_dir, cwd, &session.id);
    let new_dir = paths.session_dir.to_string_lossy().into_owned();
    let new_cwd = cwd.to_string_lossy().into_owned();
    let new_root = root.to_string_lossy().into_owned();
    let mut rewrites: Vec<(&str, &str)> = Vec::new();
    for (former, new) in [
        (&session.former_dirs, new_dir.as_str()),
        (&session.former_cwds, new_cwd.as_str()),
        (&session.former_roots, new_root.as_str()),
        (&session.former_ids, session.id.as_str()),
    ] {
        rewrites.extend(former.iter().filter(|old| !old.is_empty()).map(|old| (old.as_str(), new)));
    }
    // Longest first, so a path is never rewritten through a shorter path it
    // starts with.
    rewrites.sort_by_key(|(old, _)| std::cmp::Reverse(old.len()));
    let rewrite = |lines: &[String]| -> String {
        lines.iter().map(|line| replace_once(line, &rewrites) + "\n").collect()
    };
    fs::create_dir_all(paths.transcript.parent().expect("transcripts live in a project dir"))?;
    fs::write(&paths.transcript, rewrite(&session.main))?;
    if !session.subagents.is_empty() {
        fs::create_dir_all(&paths.subagents)?;
        for (name, lines) in &session.subagents {
            fs::write(paths.subagents.join(format!("{name}.jsonl")), rewrite(lines))?;
        }
    }
    for (name, bytes) in &session.attachments {
        let relative = Path::new(name);
        if !relative.components().all(|component| matches!(component, Component::Normal(_))) {
            continue;
        }
        let path = paths.session_dir.join(relative);
        fs::create_dir_all(path.parent().expect("joined onto the session dir"))?;
        fs::write(path, bytes)?;
    }
    Ok(paths)
}

/// Replaces every occurrence of each `old` with its `new` in one pass, so
/// replacements are never themselves rewritten. Earlier pairs win.
fn replace_once(text: &str, rewrites: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(ch) = rest.chars().next() {
        match rewrites.iter().find(|(old, _)| rest.starts_with(old)) {
            Some((old, new)) => {
                out.push_str(new);
                rest = &rest[old.len()..];
            }
            None => {
                out.push(ch);
                rest = &rest[ch.len_utf8()..];
            }
        }
    }
    out
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
        let session = RecordedSession {
            id: "abc".into(),
            former_ids: vec!["old".into()],
            former_cwds: vec!["/home/a/repo".into(), "/home/a/repo/.silt/checkouts/t".into()],
            former_roots: vec!["/home/a/repo".into()],
            former_dirs: vec!["/home/a/.claude/projects/-home-a-repo/abc".into()],
            main: vec![
                r#"{"cwd":"/home/a/repo","type":"user","sessionId":"old"}"#.to_owned(),
                r#"{"cwd":"/home/a/repo/.silt/checkouts/t/src"}"#.to_owned(),
                r#"{"result":"saved to /home/a/.claude/projects/-home-a-repo/abc/tool-results/t1.txt"}"#.to_owned(),
            ],
            subagents: vec![("agent-1".to_owned(), vec![r#"{"cwd":"/home/a/repo/src"}"#.to_owned()])],
            attachments: vec![
                ("tool-results/t1.txt".to_owned(), b"big output".to_vec()),
                ("../escape.txt".to_owned(), b"no".to_vec()),
            ],
        };
        let paths = restore(config.path(), Path::new("/home/b/repo"), Path::new("/home/b/repo"), &session).expect("restores");
        assert_eq!(paths.transcript, config.path().join("projects/-home-b-repo/abc.jsonl"));
        assert_eq!(
            fs::read_to_string(&paths.transcript).expect("transcript"),
            format!(
                "{{\"cwd\":\"/home/b/repo\",\"type\":\"user\",\"sessionId\":\"abc\"}}\n{{\"cwd\":\"/home/b/repo/src\"}}\n{{\"result\":\"saved to {}\"}}\n",
                paths.tool_results.join("t1.txt").display()
            )
        );
        assert_eq!(fs::read(paths.tool_results.join("t1.txt")).expect("attachment"), b"big output");
        assert!(!config.path().join("projects/-home-b-repo/escape.txt").exists());
        assert_eq!(
            fs::read_to_string(paths.subagents.join("agent-1.jsonl")).expect("subagent"),
            "{\"cwd\":\"/home/b/repo/src\"}\n"
        );
    }

    #[test]
    fn rewrites_never_apply_to_their_own_output() {
        let rewrites = [("/a/repo/old", "/a/repo/new"), ("/a/repo", "/a/repo/new")];
        assert_eq!(
            replace_once("cd /a/repo/old && cd /a/repo", &rewrites),
            "cd /a/repo/new && cd /a/repo/new"
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
