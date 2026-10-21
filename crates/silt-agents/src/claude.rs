//! Claude Code: sessions live in `<config>/projects/<slug(cwd)>/<id>.jsonl`,
//! with subagent transcripts in `<id>/subagents/` and large tool outputs in
//! `<id>/tool-results/` (docs/research/probes.md).

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::conventions::NativeConventions;
use crate::entry::Entry;
use crate::{
    Agent, AgentError, Capture, Found, Launch, LaunchPlan, Read, Reader, RecordedSession, Restore, Rewrites, Role, SessionIds,
    Start, Stateless, Write, translated_text,
};

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
    cwd.to_string_lossy().chars().map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' }).collect()
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

/// Writes a session's files so `claude --resume <id>` run in `cwd` continues
/// it, rewriting every former session directory, working directory and
/// session id to the new ones (docs/research/probes.md).
pub fn restore_into(config_dir: &Path, root: &Path, cwd: &Path, session: &RecordedSession) -> Result<SessionPaths, AgentError> {
    let paths = session_paths(config_dir, cwd, &session.id);
    plan_restore(&paths, root, cwd, session).apply()?;
    Ok(paths)
}

/// The files restoring `session` at `paths` writes.
pub fn plan_restore(paths: &SessionPaths, root: &Path, cwd: &Path, session: &RecordedSession) -> Restore {
    let rewrites =
        Rewrites::new(session, Some(&paths.session_dir.to_string_lossy()), &cwd.to_string_lossy(), &root.to_string_lossy());
    let mut restore = Restore {
        transcript: paths.transcript.clone(),
        writes: vec![Write::Transcript { path: paths.transcript.clone(), lines: rewrites.lines(&session.main) }],
    };
    for (name, lines) in &session.subagents {
        let path = paths.subagents.join(format!("{name}.jsonl"));
        restore.writes.push(Write::Transcript { path, lines: rewrites.lines(lines) });
    }
    restore.side_files(&paths.session_dir, session);
    restore
}

/// Claude Code transcript lines holding `messages` as plain text, chained
/// by `parentUuid` the way Claude writes them.
pub fn transcript_lines(messages: &[(Role, String)], cwd: &Path, id: &str) -> Vec<String> {
    let mut parent: Option<String> = None;
    let timestamp = crate::time::timestamp_now();
    messages
        .iter()
        .enumerate()
        .map(|(index, (role, text))| {
            let uuid = crate::derived_uuid(id, index);
            let message = match role {
                Role::User => json!({"role": "user", "content": text}),
                Role::Assistant => json!({
                    "id": format!("msg_silt_{index}"),
                    "type": "message",
                    "role": "assistant",
                    "model": "<synthetic>",
                    "content": [{"type": "text", "text": text}],
                    "stop_reason": "end_turn",
                    "stop_sequence": null,
                    "usage": {"input_tokens": 0, "output_tokens": 0}
                }),
            };
            let line = json!({
                "parentUuid": parent,
                "isSidechain": false,
                "userType": "external",
                "cwd": cwd.to_string_lossy(),
                "sessionId": id,
                "version": "2.1.0",
                "type": if *role == Role::User { "user" } else { "assistant" },
                "message": message,
                "uuid": uuid,
                "timestamp": timestamp,
            });
            parent = Some(uuid);
            line.to_string()
        })
        .collect()
}

/// The Claude Code adapter.
pub struct ClaudeCode;

impl Agent for ClaudeCode {
    fn id(&self) -> &'static str {
        AGENT
    }

    fn command(&self) -> &'static str {
        "claude"
    }

    fn display_name(&self) -> &'static str {
        "Claude Code"
    }

    fn native(&self) -> NativeConventions {
        NativeConventions {
            rules: &["CLAUDE.md", "CLAUDE.local.md"],
            skill_dirs: &[".claude/skills"],
            personal_skills: false,
            given_skills: true,
        }
    }

    fn reader(&self) -> Box<dyn Reader> {
        Box::new(Stateless(|line| Read { entries: normalize(line), marks: Vec::new() }))
    }

    fn capture(&self) -> Capture {
        Capture::Hooks
    }

    fn session_ids(&self) -> SessionIds {
        SessionIds::Chosen
    }

    fn transcript(&self, cwd: &Path, id: &str) -> Option<PathBuf> {
        Some(session_paths(&config_dir()?, cwd, id).transcript)
    }

    fn side_transcripts(&self, transcript: &Path) -> Vec<(String, PathBuf)> {
        let Ok(entries) = fs::read_dir(transcript.with_extension("").join("subagents")) else {
            return Vec::new();
        };
        let mut found: Vec<(String, PathBuf)> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "jsonl"))
            .filter_map(|path| Some((path.file_stem()?.to_string_lossy().into_owned(), path.clone())))
            .collect();
        found.sort();
        found
    }

    fn session_dir(&self, transcript: &Path) -> Option<PathBuf> {
        Some(transcript.with_extension(""))
    }

    fn attachments(&self, transcript: &Path) -> Vec<(String, PathBuf)> {
        let session_dir = transcript.with_extension("");
        let mut found = Vec::new();
        for (dir, keep) in [
            ("tool-results", (|_: &Path| true) as fn(&Path) -> bool),
            ("subagents", |path: &Path| path.extension().is_none_or(|extension| extension != "jsonl")),
        ] {
            let Ok(entries) = fs::read_dir(session_dir.join(dir)) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if entry.file_type().is_ok_and(|kind| kind.is_file()) && keep(&path) {
                    found.push((format!("{dir}/{}", entry.file_name().to_string_lossy()), path));
                }
            }
        }
        found.sort();
        found
    }

    fn discover(&self, cwd: &Path, since_ms: i64, known: &[String]) -> Option<Found> {
        let project = config_dir()?.join("projects").join(project_slug(cwd));
        fs::read_dir(project)
            .ok()?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "jsonl"))
            .filter_map(|path| Some((crate::modified_ms(&path)?, path)))
            .filter(|(modified, path)| {
                *modified >= since_ms
                    && path.file_stem().is_some_and(|stem| !known.iter().any(|known| known.as_str() == stem.to_string_lossy()))
            })
            .max_by_key(|(modified, _)| *modified)
            .map(|(_, path)| Found {
                id: path.file_stem().expect("filtered on extension").to_string_lossy().into_owned(),
                transcript: path,
            })
    }

    fn restore(&self, root: &Path, cwd: &Path, session: &RecordedSession) -> Result<PathBuf, AgentError> {
        let config = config_dir().ok_or(AgentError::NoHome)?;
        Ok(restore_into(&config, root, cwd, session)?.transcript)
    }

    fn translate(&self, entries: &[Entry], cwd: &Path, id: &str) -> Option<Vec<String>> {
        Some(transcript_lines(&translated_text(entries), cwd, id))
    }

    fn launch(&self, plan: &LaunchPlan) -> Result<Launch, AgentError> {
        let mut launch = Launch::new(self.command());
        let mut prompt = None;
        match &plan.start {
            Start::New { id: Some(id) } => {
                launch.arg("--session-id").arg(id);
            }
            Start::New { id: None } => {}
            Start::Resume { id } => {
                launch.arg("--resume").arg(id);
            }
            Start::Primed { id, prompt: text } => {
                if let Some(id) = id {
                    launch.arg("--session-id").arg(id);
                }
                prompt = Some(text.clone());
            }
        }
        let mcp = json!({
            "mcpServers": { "silt": { "command": plan.mcp.command.to_string_lossy(), "args": plan.mcp.args } }
        });
        launch.arg("--mcp-config").arg(mcp.to_string());
        launch.arg("--settings").arg(hook_settings(&plan.hook_command));
        if let Some(instructions) = &plan.instructions {
            launch.arg("--append-system-prompt").arg(instructions);
        }
        if !plan.skills.is_empty() {
            launch.arg("--plugin-dir").arg(skills_plugin(&plan.scratch, &plan.skills)?);
        }
        for arg in &plan.args {
            launch.arg(arg);
        }
        if let Some(prompt) = prompt {
            launch.arg(prompt);
        }
        Ok(launch)
    }
}

/// A plugin directory holding the given skills, so Claude Code loads them
/// as its own skills and slash commands.
fn skills_plugin(scratch: &Path, skills: &[PathBuf]) -> io::Result<PathBuf> {
    let dir = scratch.join("claude-plugin");
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    fs::create_dir_all(dir.join(".claude-plugin"))?;
    fs::write(
        dir.join(".claude-plugin/plugin.json"),
        json!({"name": "silt", "description": "Skills Silt found in this repository and your ~/.agents/skills"}).to_string(),
    )?;
    crate::link_skills(&dir.join("skills"), skills)?;
    Ok(dir)
}
/// The conversation steps recorded in one transcript line.
pub fn normalize(line: &str) -> Vec<Entry> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return vec![Entry::unparseable()];
    };
    let kind = value.get("type").and_then(Value::as_str).unwrap_or("unknown");
    match kind {
        "user" if value.get("isCompactSummary").and_then(Value::as_bool) == Some(true) => {
            vec![Entry::Compaction { summary: Some(message_text(&value)) }]
        }
        "system" if value.get("subtype").and_then(Value::as_str) == Some("compact_boundary") => {
            vec![Entry::Compaction { summary: None }]
        }
        // What Claude Code adds as the user's turn, such as a skill's text,
        // hook feedback or a note on an image, which nobody typed.
        "user" if value.get("isMeta").and_then(Value::as_bool) == Some(true) => {
            vec![Entry::Other { kind: "meta".into() }]
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
            "thinking" => Some(Entry::Thinking { text: string_field(block, "thinking") })
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
            other => Some(Entry::Other { kind: format!("{role}_{other}") }),
        })
        .collect()
}

fn text_entry(role: &str, text: String) -> Entry {
    if role == "user" { Entry::User { text } } else { Entry::Assistant { text } }
}

fn message_text(value: &Value) -> String {
    block_text(value.pointer("/message/content"))
}

fn block_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => {
            blocks.iter().filter_map(|block| block.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n")
        }
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
    /// For `SessionStart`: `startup`, `resume`, `clear` or `compact`.
    #[serde(default)]
    pub source: Option<String>,
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
        assert_eq!(project_slug(Path::new("/home/u/my_repo.v2/src")), "-home-u-my-repo-v2-src");
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
        let skill = r#"{"type":"user","isMeta":true,"message":{"role":"user","content":[{"type":"text","text":"Base directory for this skill: /s"}]}}"#;
        assert_eq!(normalize(skill), vec![Entry::Other { kind: "meta".into() }]);

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
                Entry::ToolCall { id: "toolu_1".into(), name: "Edit".into(), input: json!({"file_path": "src/lexer.rs"}) },
            ]
        );

        let result = r#"{"type":"user","message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"text","text":"ok"}],"is_error":false}]}}"#;
        assert_eq!(
            normalize(result),
            vec![Entry::ToolResult { tool_use_id: "toolu_1".into(), text: "ok".into(), is_error: false }]
        );

        let boundary = r#"{"type":"system","subtype":"compact_boundary","content":"Conversation compacted"}"#;
        assert_eq!(normalize(boundary), vec![Entry::Compaction { summary: None }]);
        let summary =
            r#"{"type":"user","isCompactSummary":true,"message":{"role":"user","content":"We were fixing the lexer."}}"#;
        assert_eq!(normalize(summary), vec![Entry::Compaction { summary: Some("We were fixing the lexer.".into()) }]);
        assert_eq!(normalize(r#"{"type":"file-history-snapshot"}"#), vec![Entry::Other { kind: "file-history-snapshot".into() }]);
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
        let paths =
            restore_into(config.path(), Path::new("/home/b/repo"), Path::new("/home/b/repo"), &session).expect("restores");
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
    fn translated_lines_chain_and_name_the_session() {
        let lines = transcript_lines(&[(Role::User, "hi".into()), (Role::Assistant, "hello".into())], Path::new("/repo"), "s1");
        let first: Value = serde_json::from_str(&lines[0]).expect("json");
        let second: Value = serde_json::from_str(&lines[1]).expect("json");
        assert_eq!(first["parentUuid"], Value::Null);
        assert_eq!(second["parentUuid"], first["uuid"]);
        assert_eq!(second["sessionId"], "s1");
        assert_eq!(normalize(&lines[1]), vec![Entry::Assistant { text: "hello".into() }]);
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
        assert_eq!(settings.pointer("/hooks/PostToolUse/0/hooks/0/command"), Some(&json!("silt hook PostToolUse")));
    }
}
