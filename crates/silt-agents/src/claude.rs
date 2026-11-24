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
use crate::entry::{Entry, Task, TaskKind, TaskStatus};
use crate::{
    Agent, AgentError, Capture, CheckedSession, Found, Launch, LaunchPlan, OnDisk, Read, Reader, Restore, Rewrites, Role,
    SessionIds, Start, Stateless, Write, translated_text,
};

pub const AGENT: &str = "claude-code";

/// Claude Code's configuration directory: `$CLAUDE_CONFIG_DIR` or `~/.claude`.
pub fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = env::var_os("CLAUDE_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    silt_os::home_dir().map(|home| home.join(".claude"))
}

/// [`config_dir`] for a session in `cwd`, whose workspace may give Claude
/// Code a directory of its own (ADR 0065).
fn config_dir_in(cwd: &Path) -> Option<PathBuf> {
    crate::agent_home_in("CLAUDE_CONFIG_DIR", ".claude", cwd)
}

/// The directory name Claude Code uses for sessions started in `cwd`.
pub fn project_slug(cwd: &Path) -> String {
    cwd.to_string_lossy().chars().map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' }).collect()
}

/// Every session in the configuration directory `config`, with where and
/// when it started as its first records say. Older versions kept
/// subagents' transcripts beside the sessions, as `agent-<id>.jsonl`.
pub fn on_disk_in(config: &Path) -> Vec<OnDisk> {
    let Ok(projects) = fs::read_dir(config.join("projects")) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for project in projects.flatten() {
        let Ok(sessions) = fs::read_dir(project.path()) else {
            continue;
        };
        for entry in sessions.flatten() {
            let path = entry.path();
            let Some(id) = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()) else {
                continue;
            };
            if path.extension().is_none_or(|extension| extension != "jsonl")
                || id.starts_with("agent-")
                || !crate::is_safe_id(&id)
            {
                continue;
            }
            let (mut cwd, mut started_ms) = (None, None);
            for line in crate::first_lines(&path, 32) {
                let Ok(record) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                cwd = cwd.or_else(|| record.get("cwd").and_then(Value::as_str).map(PathBuf::from));
                started_ms = started_ms.or_else(|| crate::record_time(&line));
                if cwd.is_some() && started_ms.is_some() {
                    break;
                }
            }
            found.push(OnDisk { id, transcript: path, cwd, started_ms });
        }
    }
    found.sort_by(|a, b| a.started_ms.cmp(&b.started_ms).then_with(|| a.id.cmp(&b.id)));
    found
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

/// The transcript of Claude Code's session `session` started in `cwd`,
/// once it has one.
pub fn transcript_of(cwd: &Path, session: &str) -> Option<PathBuf> {
    transcript_in(&config_dir_in(cwd)?, cwd, session)
}

fn transcript_in(config: &Path, cwd: &Path, session: &str) -> Option<PathBuf> {
    if !crate::is_safe_id(session) {
        return None;
    }
    let own = session_paths(config, cwd, session).transcript;
    if own.exists() {
        return Some(own);
    }
    // Claude Code may spell its working directory otherwise, as Windows'
    // verbatim paths do, and so name its project's folder otherwise.
    fs::read_dir(config.join("projects"))
        .ok()?
        .flatten()
        .map(|project| project.path().join(format!("{session}.jsonl")))
        .find(|path| path.exists())
}

/// The background tasks that `lines` of a Claude Code transcript report
/// ended, by Claude's id for each, with how they ended.
pub fn tasks_ended(lines: &str) -> Vec<(String, TaskStatus)> {
    lines
        .lines()
        .filter(|line| line.contains("<task-notification>"))
        .flat_map(normalize)
        .filter_map(|entry| match entry {
            Entry::Task(task) if task.status != TaskStatus::Running => Some((task.id, task.status)),
            _ => None,
        })
        .collect()
}

/// Writes a session's files so `claude --resume <id>` run in `cwd` continues
/// it, rewriting every former session directory, working directory and
/// session id to the new ones (docs/research/probes.md).
pub fn restore_into(config_dir: &Path, root: &Path, cwd: &Path, session: &CheckedSession) -> Result<SessionPaths, AgentError> {
    let paths = session_paths(config_dir, cwd, &session.id);
    plan_restore(&paths, root, cwd, session).apply()?;
    Ok(paths)
}

/// The files restoring `session` at `paths` writes.
pub fn plan_restore(paths: &SessionPaths, root: &Path, cwd: &Path, session: &CheckedSession) -> Restore {
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
    fn model_apis(&self) -> &'static [crate::ModelApi] {
        &crate::CLAUDE_APIS
    }
    fn model_hosts(&self) -> &'static [&'static str] {
        &["api.anthropic.com", "console.anthropic.com", "claude.ai", "statsig.anthropic.com"]
    }
    fn homes(&self, home: &Path, env: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
        // Without its own folder, Claude Code keeps its settings in the home folder too.
        match env("CLAUDE_CONFIG_DIR").filter(|dir| !dir.is_empty()) {
            Some(dir) => vec![PathBuf::from(dir)],
            None => vec![home.join(".claude"), home.join(".claude.json")],
        }
    }

    fn sign_in(&self, home: &Path, env: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
        self.homes(home, env).first().map(|dir| dir.join(".credentials.json")).into_iter().collect()
    }

    fn read_only_home(&self, home: &Path, env: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
        let Some(dir) = self.homes(home, env).into_iter().find(|dir| dir.extension().is_none()) else { return Vec::new() };
        // Its memory, and the folders of what it loads and runs: plugins
        // bring hooks and MCP servers, and hooks' scripts are run by name.
        ["settings.json", "settings.local.json", "CLAUDE.md", "plugins", "hooks", "commands", "agents", "skills"]
            .iter()
            .map(|name| dir.join(name))
            .collect()
    }

    fn project_state(&self, home: &Path, env: &dyn Fn(&str) -> Option<String>, cwd: &Path) -> crate::ProjectState {
        let Some(dir) = self.homes(home, env).into_iter().find(|dir| dir.extension().is_none()) else {
            return crate::ProjectState::default();
        };
        // Sessions and the copies of files its edits replaced are kept per
        // project; the prompt history isn't.
        crate::ProjectState {
            hide: vec![dir.join("projects"), dir.join("file-history")],
            own: vec![dir.join("projects").join(project_slug(cwd))],
            fresh: vec![dir.join("history.jsonl")],
            // Its MCP servers, which it and Silt's connectors start outside.
            copied: vec![match env("CLAUDE_CONFIG_DIR").filter(|dir| !dir.is_empty()) {
                Some(_) => dir.join(".claude.json"),
                None => home.join(".claude.json"),
            }],
        }
    }

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
        Box::new(Stateless(|line| Read { entries: normalize(line), ..Read::default() }))
    }

    fn capture(&self) -> Capture {
        Capture::Hooks
    }

    fn session_ids(&self) -> SessionIds {
        SessionIds::Chosen
    }

    fn transcript(&self, cwd: &Path, id: &str) -> Option<PathBuf> {
        Some(session_paths(&config_dir_in(cwd)?, cwd, id).transcript)
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
        let project = config_dir_in(cwd)?.join("projects").join(project_slug(cwd));
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

    fn on_disk(&self, _dirs: &[PathBuf]) -> Vec<OnDisk> {
        crate::agent_homes("CLAUDE_CONFIG_DIR", ".claude").iter().flat_map(|config| on_disk_in(config)).collect()
    }

    fn restore(&self, root: &Path, cwd: &Path, session: &CheckedSession) -> Result<PathBuf, AgentError> {
        let config = config_dir_in(cwd).ok_or(AgentError::NoHome)?;
        Ok(restore_into(&config, root, cwd, session)?.transcript)
    }

    fn translate(&self, entries: &[Entry], cwd: &Path, id: &str) -> Option<Vec<String>> {
        Some(transcript_lines(&translated_text(entries), cwd, id))
    }

    fn bypass_flag(&self, args: &[String]) -> Option<String> {
        crate::flag_in(
            args,
            &["--dangerously-skip-permissions", "--allow-dangerously-skip-permissions"],
            &[("--permission-mode", &["bypassPermissions", "auto", "dontAsk"])],
        )
    }

    fn sandbox_support(&self) -> crate::SandboxSupport {
        crate::SandboxSupport::Isolate
    }

    fn launch(&self, plan: &LaunchPlan) -> Result<Launch, AgentError> {
        let mut launch = Launch::new(self.command());
        let mut prompt = None;
        match &plan.start {
            Start::New { id: Some(id) } => {
                launch.arg("--session-id").arg(id);
            }
            Start::New { id: None } => {}
            Start::Resume { id, prompt: first } => {
                launch.arg("--resume").arg(id);
                prompt.clone_from(first);
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
        let hooks = hook_settings(&plan.hook_command, cfg!(windows).then_some(plan.hook_program.as_path()));
        launch.arg("--settings").arg(locked_settings(&hooks, &plan.locks));
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
    // Only the fields read, so the large structured tool results beside
    // them aren't built; a line of another shape is read whole.
    if let Ok(fields) = serde_json::from_str::<Fields>(line) {
        return entries(fields);
    }
    match serde_json::from_str::<Value>(line) {
        Ok(value) => entries(Fields::of(&value)),
        Err(_) => vec![Entry::unparseable()],
    }
}

/// The parts of a transcript line `normalize` reads.
#[derive(Deserialize)]
struct Fields {
    #[serde(rename = "type")]
    kind: Option<Value>,
    #[serde(rename = "isCompactSummary")]
    compact_summary: Option<Value>,
    #[serde(rename = "isMeta")]
    meta: Option<Value>,
    subtype: Option<Value>,
    message: Option<Message>,
    /// What a tool's result says beside its text: the subagent or
    /// background command it started, or the subagent it resumed.
    #[serde(rename = "toolUseResult", default, deserialize_with = "started")]
    started: Option<Started>,
    attachment: Option<Attachment>,
}

/// The ids a tool's structured result gives the work it left running.
#[derive(Debug, Default, PartialEq, Eq)]
struct Started {
    agent: Option<String>,
    /// A finished subagent that `SendMessage` set running again.
    resumed: Option<String>,
    background: Option<String>,
    status: Option<String>,
}

/// What Claude Code adds to the conversation on its own; a
/// `queued_command` is something that came in while it worked, such as a
/// background task's report.
#[derive(Deserialize)]
struct Attachment {
    #[serde(rename = "type")]
    kind: Option<String>,
    prompt: Option<Value>,
}

/// Reads only `agentId`, `resumedAgentId`, `backgroundTaskId` and `status`
/// of a structured result, skipping its output, which can be long; a result
/// that isn't an object gives none.
fn started<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Started>, D::Error> {
    use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};

    struct Fields;
    impl<'de> Visitor<'de> for Fields {
        type Value = Option<Started>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a tool's structured result")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut started = Started::default();
            while let Some(key) = map.next_key::<std::borrow::Cow<'de, str>>()? {
                let field = match key.as_ref() {
                    "agentId" => &mut started.agent,
                    "resumedAgentId" => &mut started.resumed,
                    "backgroundTaskId" => &mut started.background,
                    "status" => &mut started.status,
                    _ => {
                        map.next_value::<IgnoredAny>()?;
                        continue;
                    }
                };
                *field = map.next_value::<Value>()?.as_str().map(str::to_owned);
            }
            Ok((started != Started::default()).then_some(started))
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            while seq.next_element::<IgnoredAny>()?.is_some() {}
            Ok(None)
        }

        fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
    }
    deserializer.deserialize_any(Fields)
}

#[derive(Deserialize)]
struct Message {
    content: Option<Value>,
    id: Option<String>,
    usage: Option<Value>,
}

impl Fields {
    fn of(value: &Value) -> Self {
        Self {
            kind: value.get("type").cloned(),
            compact_summary: value.get("isCompactSummary").cloned(),
            meta: value.get("isMeta").cloned(),
            subtype: value.get("subtype").cloned(),
            message: value.get("message").map(|message| Message {
                content: message.get("content").cloned(),
                id: message.get("id").and_then(Value::as_str).map(str::to_owned),
                usage: message.get("usage").cloned(),
            }),
            started: value.get("toolUseResult").and_then(|result| started(result).ok().flatten()),
            attachment: value.get("attachment").map(|attachment| Attachment {
                kind: attachment.get("type").and_then(Value::as_str).map(str::to_owned),
                prompt: attachment.get("prompt").cloned(),
            }),
        }
    }
}

/// The line's entries, its texts moved out of the parsed line rather than
/// copied: a tool's output can be long.
fn entries(fields: Fields) -> Vec<Entry> {
    let flag = |value: &Option<Value>| value.as_ref().and_then(Value::as_bool) == Some(true);
    let kind = fields.kind.as_ref().and_then(Value::as_str).unwrap_or("unknown").to_owned();
    let (content, usage) = match fields.message {
        Some(message) => (message.content, message.id.zip(message.usage)),
        None => (None, None),
    };
    match kind.as_str() {
        "user" if flag(&fields.compact_summary) => vec![Entry::Compaction { summary: Some(block_text(content)) }],
        "system" if fields.subtype.as_ref().and_then(Value::as_str) == Some("compact_boundary") => {
            vec![Entry::Compaction { summary: None }]
        }
        // What Claude Code adds as the user's turn, such as a skill's text,
        // hook feedback or a note on an image, which nobody typed.
        "user" if flag(&fields.meta) => vec![Entry::Other { kind: "meta".into() }],
        "attachment" => {
            let attachment = fields.attachment.filter(|attachment| attachment.kind.as_deref() == Some("queued_command"));
            match attachment.and_then(|attachment| attachment.prompt).as_ref().and_then(Value::as_str).and_then(task_notification)
            {
                Some(task) => vec![Entry::Task(task)],
                None => vec![Entry::Other { kind: "attachment".into() }],
            }
        }
        role @ ("user" | "assistant") => {
            let mut entries = message_entries(role, content);
            if role == "user" {
                let reported = entries.iter().find_map(|entry| match entry {
                    Entry::User { text } => task_notification(text),
                    _ => None,
                });
                entries.extend(reported.map(Entry::Task));
                entries.extend(fields.started.and_then(|started| started_task(started, &entries)).map(Entry::Task));
            }
            if let Some((message, usage)) = usage.filter(|_| role == "assistant") {
                let count = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
                let input = count("input_tokens") + count("cache_read_input_tokens") + count("cache_creation_input_tokens");
                let output = count("output_tokens");
                if input + output > 0 {
                    entries.push(Entry::Usage { message, input, output });
                }
            }
            entries
        }
        other => vec![Entry::Other { kind: other.to_owned() }],
    }
}

/// The work a tool's result says it left running, or a subagent's that
/// ran to its end, tied to the call the result answers.
fn started_task(started: Started, entries: &[Entry]) -> Option<Task> {
    let call = entries.iter().find_map(|entry| match entry {
        Entry::ToolResult { tool_use_id, .. } => Some(tool_use_id.clone()),
        _ => None,
    });
    let (id, kind) = match (started.agent.or(started.resumed), started.background) {
        (Some(agent), _) => (agent, TaskKind::Subagent),
        (None, Some(background)) => (background, TaskKind::Background),
        (None, None) => return None,
    };
    let status = match started.status.as_deref() {
        None | Some("async_launched") => TaskStatus::Running,
        Some(status) => TaskStatus::parse(status),
    };
    let side = (kind == TaskKind::Subagent).then(|| format!("agent-{id}"));
    Some(Task { id, call, kind: Some(kind), status, side, summary: None })
}

/// A background task's report, which Claude Code puts in the conversation
/// as `<task-notification>` with its id, the call that started it, its
/// status, and a summary and result.
pub fn task_notification(text: &str) -> Option<Task> {
    let body = text.trim_start().strip_prefix("<task-notification>")?;
    let field = |tag: &str| {
        let start = body.find(&format!("<{tag}>"))? + tag.len() + 2;
        let end = body[start..].find(&format!("</{tag}>")).map_or(body.len(), |end| start + end);
        Some(body[start..end].trim().to_owned()).filter(|value| !value.is_empty())
    };
    let summary = match (field("summary"), field("result")) {
        (Some(summary), Some(result)) => Some(format!("{summary}\n\n{result}")),
        (summary, result) => summary.or(result),
    };
    Some(Task {
        id: field("task-id")?,
        call: field("tool-use-id"),
        kind: None,
        status: field("status").map_or(TaskStatus::Running, |status| TaskStatus::parse(&status)),
        side: None,
        summary,
    })
}

fn message_entries(role: &str, content: Option<Value>) -> Vec<Entry> {
    let blocks = match content {
        Some(Value::String(text)) => return vec![text_entry(role, text)],
        Some(Value::Array(blocks)) => blocks,
        _ => return Vec::new(),
    };
    // A message's images follow its words, so a prompt's text comes first.
    let mut images = Vec::new();
    let mut entries: Vec<Entry> = blocks
        .into_iter()
        .filter_map(|block| {
            let Value::Object(mut block) = block else {
                return None;
            };
            let kind = block.get("type")?.as_str()?.to_owned();
            match kind.as_str() {
                "image" => {
                    images.extend(image(&block));
                    None
                }
                "text" => Some(text_entry(role, take_string(&mut block, "text"))),
                "thinking" => {
                    Some(take_string(&mut block, "thinking")).filter(|text| !text.is_empty()).map(|text| Entry::Thinking { text })
                }
                "tool_use" => Some(Entry::ToolCall {
                    id: take_string(&mut block, "id"),
                    name: take_string(&mut block, "name"),
                    input: block.remove("input").unwrap_or(Value::Null),
                    kind: None,
                }),
                "tool_result" => {
                    let content = block.remove("content");
                    if let Some(Value::Array(parts)) = &content {
                        images.extend(
                            parts
                                .iter()
                                .filter_map(Value::as_object)
                                .filter(|part| part.get("type").and_then(Value::as_str) == Some("image"))
                                .filter_map(image),
                        );
                    }
                    Some(Entry::ToolResult {
                        tool_use_id: take_string(&mut block, "tool_use_id"),
                        is_error: block.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                        text: block_text(content),
                    })
                }
                other => Some(Entry::Other { kind: format!("{role}_{other}") }),
            }
        })
        .collect();
    entries.extend(images);
    entries
}

/// An image block's picture, when it carries its bytes rather than a link.
pub(crate) fn image(block: &serde_json::Map<String, Value>) -> Option<Entry> {
    let source = block.get("source")?;
    (source.get("type")?.as_str()? == "base64").then_some(())?;
    Some(Entry::Image {
        media_type: source.get("media_type")?.as_str()?.to_owned(),
        data: source.get("data")?.as_str()?.to_owned(),
        stored: None,
    })
}

fn text_entry(role: &str, text: String) -> Entry {
    if role == "user" { Entry::User { text } } else { Entry::Assistant { text } }
}

pub(crate) fn block_text(content: Option<Value>) -> String {
    match content {
        Some(Value::String(text)) => text,
        Some(Value::Array(blocks)) => {
            blocks.iter().filter_map(|block| block.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n")
        }
        _ => String::new(),
    }
}

/// The string at `key`, empty when it's missing or not a string.
fn take_string(block: &mut serde_json::Map<String, Value>, key: &str) -> String {
    match block.remove(key) {
        Some(Value::String(text)) => text,
        _ => String::new(),
    }
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
    /// For tool hooks: the tool's arguments, as the model gave them.
    #[serde(default)]
    pub tool_input: Option<Value>,
    /// For `SessionStart`: `startup`, `resume`, `clear` or `compact`.
    #[serde(default)]
    pub source: Option<String>,
    /// For `UserPromptSubmit`: what the person asked.
    #[serde(default)]
    pub prompt: Option<String>,
    /// For `SessionEnd`: why, such as `clear`, `logout` or `prompt_input_exit`.
    #[serde(default)]
    pub reason: Option<String>,
}

/// `settings` with the workspace's locks in Claude Code's own keys, which
/// it honors from `--settings` (ADR 0065).
pub fn locked_settings(settings: &str, locks: &crate::Locks) -> String {
    let mut settings: Value = serde_json::from_str(settings).unwrap_or_else(|_| json!({}));
    if locks.no_bypass {
        settings["permissions"]["disableBypassPermissionsMode"] = json!("disable");
        settings["disableAutoMode"] = json!("disable");
    }
    if locks.sandbox {
        settings["sandbox"] = json!({ "enabled": true, "failIfUnavailable": true, "allowUnsandboxedCommands": false });
    }
    if !locks.deny.is_empty() {
        let paths: Vec<String> = locks.deny.iter().map(|path| path.to_string_lossy().replace('\\', "/")).collect();
        settings["sandbox"]["filesystem"]["denyRead"] = json!(paths);
        settings["sandbox"]["filesystem"]["denyWrite"] = json!(paths);
        let rules: Vec<String> = paths
            .iter()
            .flat_map(|path| {
                let path = path.trim_end_matches('/');
                ["Read", "Edit"].map(|tool| format!("{tool}(/{path}/**)"))
            })
            .collect();
        settings["permissions"]["deny"] = json!(rules);
    }
    settings.to_string()
}

/// Hook events Silt listens to.
pub const HOOK_EVENTS: [&str; 6] = ["UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop", "SessionStart", "SessionEnd"];

/// Settings JSON for `claude --settings` that runs `command <event>` for each
/// hook event Silt needs, with the hook input on stdin. With `program`, the
/// hooks start it directly with `hook <event>` as its arguments (Claude
/// Code's exec form), which Windows needs (ADR 0045): there Claude Code
/// runs a command through Git Bash or else PowerShell, which read a quoted
/// path differently.
pub fn hook_settings(command: &str, program: Option<&Path>) -> String {
    let mut hooks = serde_json::Map::new();
    for event in HOOK_EVENTS {
        let hook = match program {
            Some(program) => json!({ "type": "command", "command": program, "args": ["hook", event], "timeout": 30 }),
            None => json!({ "type": "command", "command": format!("{command} {event}"), "timeout": 30 }),
        };
        hooks.insert(event.to_owned(), json!([{ "matcher": "*", "hooks": [hook] }]));
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

    fn task(id: &str, call: Option<&str>, kind: Option<TaskKind>, status: TaskStatus) -> Task {
        Task { id: id.into(), call: call.map(str::to_owned), kind, status, side: None, summary: None }
    }

    #[test]
    fn a_result_names_the_background_command_or_subagent_it_started() {
        let background = r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"t1","type":"tool_result","content":"Command running in background with ID: b3r","is_error":false}]},"toolUseResult":{"stdout":"","stderr":"","interrupted":false,"backgroundTaskId":"b3r"}}"#;
        assert_eq!(
            normalize(background)[1],
            Entry::Task(task("b3r", Some("t1"), Some(TaskKind::Background), TaskStatus::Running))
        );
        let launched = r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"t2","type":"tool_result","content":[{"type":"text","text":"Async agent launched"}]}]},"toolUseResult":{"isAsync":true,"status":"async_launched","agentId":"ad7","prompt":"Research"}}"#;
        assert_eq!(
            normalize(launched)[1],
            Entry::Task(Task {
                side: Some("agent-ad7".into()),
                ..task("ad7", Some("t2"), Some(TaskKind::Subagent), TaskStatus::Running)
            })
        );
        let finished = launched.replace("async_launched", "completed");
        assert!(matches!(&normalize(&finished)[1], Entry::Task(Task { status: TaskStatus::Completed, .. })));
        let plain = r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"t3","type":"tool_result","content":"ok"}]},"toolUseResult":"Error: no"}"#;
        assert_eq!(normalize(plain).len(), 1);
    }

    #[test]
    fn a_message_that_resumes_a_finished_subagent_sets_it_running_under_the_message_s_call() {
        let resumed = r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"s1","type":"tool_result","content":[{"type":"text","text":"{\"success\":true,\"message\":\"Resuming agent ad7\"}"}]}]},"toolUseResult":{"success":true,"message":"Resuming agent ad7","resumedAgentId":"ad7","revivedAgentId":"ad7","pin":{"id":"ad7","name":"ad7","ref":"450a12"}}}"#;
        assert_eq!(
            normalize(resumed)[1],
            Entry::Task(Task {
                side: Some("agent-ad7".into()),
                ..task("ad7", Some("s1"), Some(TaskKind::Subagent), TaskStatus::Running)
            })
        );
        let queued = r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"s2","type":"tool_result","content":"queued"}]},"toolUseResult":{"success":true,"message":"Message queued for delivery to ad7 at its next tool round.","pin":{"id":"ad7"}}}"#;
        assert_eq!(normalize(queued).len(), 1, "a message to a subagent still running starts nothing");
    }

    #[test]
    fn a_task_report_says_how_it_ended_whether_it_came_idle_or_mid_turn() {
        let report = "<task-notification>\n<task-id>b3r</task-id>\n<tool-use-id>t1</tool-use-id>\n<status>completed</status>\n<summary>Background command \"probe\" completed (exit code 0)</summary>\n</task-notification>";
        let ended = Task {
            summary: Some("Background command \"probe\" completed (exit code 0)".into()),
            ..task("b3r", Some("t1"), None, TaskStatus::Completed)
        };
        let idle = json!({"type": "user", "message": {"role": "user", "content": report}}).to_string();
        assert_eq!(normalize(&idle), vec![Entry::User { text: report.into() }, Entry::Task(ended.clone())]);
        let queued = json!({"type": "attachment", "attachment": {"type": "queued_command", "prompt": report}}).to_string();
        assert_eq!(normalize(&queued), vec![Entry::Task(ended)]);
        let typed = json!({"type": "attachment", "attachment": {"type": "queued_command", "prompt": "also this"}}).to_string();
        assert_eq!(normalize(&typed), vec![Entry::Other { kind: "attachment".into() }]);
        assert_eq!(
            task_notification("<task-notification><task-id>x</task-id><status>killed</status>").map(|task| task.status),
            Some(TaskStatus::Stopped)
        );
    }

    #[test]
    fn a_transcript_tells_which_background_tasks_ended_and_how() {
        let report = |id: &str, status: &str| {
            let text = format!("<task-notification>\n<task-id>{id}</task-id>\n<status>{status}</status>\n</task-notification>");
            json!({"type": "user", "origin": {"kind": "task-notification"}, "message": {"role": "user", "content": text}})
                .to_string()
        };
        let reminded = json!({"type": "user", "message": {"role": "user", "content": [{"type": "text",
            "text": "<system-reminder>\n<task-notification>\n<task-id>quoted</task-id>\n<status>failed</status>\n</task-notification>"}]}});
        let started = r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"t1","type":"tool_result","content":"Command running in background with ID: b3r"}]},"toolUseResult":{"backgroundTaskId":"b3r"}}"#;
        let lines =
            [started.to_owned(), report("b3r", "completed"), reminded.to_string(), report("k1", "killed"), report("r1", "")];
        assert_eq!(
            tasks_ended(&lines.join("\n")),
            [("b3r".to_owned(), TaskStatus::Completed), ("k1".to_owned(), TaskStatus::Stopped)]
        );
    }

    #[test]
    fn a_transcript_is_found_by_its_session_alone() {
        let config = tempfile::tempdir().unwrap();
        let cwd = Path::new("/work/repo");
        // Claude Code named its project's folder after a spelling of the directory other than this one.
        let elsewhere = config.path().join("projects/-verbatim-work-repo");
        fs::create_dir_all(&elsewhere).unwrap();
        fs::write(elsewhere.join("s1.jsonl"), "").unwrap();
        assert_eq!(transcript_in(config.path(), cwd, "s1"), Some(elsewhere.join("s1.jsonl")));
        let own = session_paths(config.path(), cwd, "s2").transcript;
        fs::create_dir_all(own.parent().unwrap()).unwrap();
        fs::write(&own, "").unwrap();
        assert_eq!(transcript_in(config.path(), cwd, "s2"), Some(own), "its own project's first");
        assert_eq!(transcript_in(config.path(), cwd, "s3"), None);
        assert_eq!(transcript_in(config.path(), cwd, "../s1"), None);
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
                Entry::ToolCall {
                    id: "toolu_1".into(),
                    name: "Edit".into(),
                    input: json!({"file_path": "src/lexer.rs"}),
                    kind: None
                },
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

        // Lines of other shapes than Claude Code's own read as they always did.
        assert_eq!(normalize(r#"{"type":"user","message":"not an object"}"#), Vec::new());
        assert_eq!(
            normalize(r#"{"type":"user","isCompactSummary":"yes","message":{"content":"x"}}"#),
            vec![Entry::User { text: "x".into() }]
        );
        assert_eq!(normalize(r#"{"type":7}"#), vec![Entry::Other { kind: "unknown".into() }]);
        assert_eq!(normalize("not json"), vec![Entry::unparseable()]);
        let beside = r#"{"type":"user","message":{"content":"hi","usage":{"x":1}},"toolUseResult":{"file":{"content":"big"}}}"#;
        assert_eq!(normalize(beside), vec![Entry::User { text: "hi".into() }]);
    }

    #[test]
    fn restore_relocates_the_session() {
        let config = tempfile::tempdir().expect("temp dir");
        let session = crate::RecordedSession {
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
            attachments: vec![("tool-results/t1.txt".to_owned(), b"big output".to_vec())],
        }
        .checked()
        .expect("safe");
        let paths =
            restore_into(config.path(), Path::new("/home/b/repo"), Path::new("/home/b/repo"), &session).expect("restores");
        assert_eq!(paths.transcript, config.path().join("projects/-home-b-repo/abc.jsonl"));
        // The old directory becomes the new one, and on Windows the rest of
        // the path takes its separators.
        let saved = format!("saved to {}", paths.tool_results.join("t1.txt").display());
        assert_eq!(
            fs::read_to_string(&paths.transcript).expect("transcript"),
            format!(
                "{{\"cwd\":\"/home/b/repo\",\"type\":\"user\",\"sessionId\":\"abc\"}}\n{{\"cwd\":\"/home/b/repo/src\"}}\n{}\n",
                json!({ "result": saved })
            )
        );
        assert_eq!(fs::read(paths.tool_results.join("t1.txt")).expect("attachment"), b"big output");
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
        let settings: Value = serde_json::from_str(&hook_settings("silt hook", None)).expect("valid json");
        assert_eq!(settings.pointer("/hooks/PostToolUse/0/hooks/0/command"), Some(&json!("silt hook PostToolUse")));
        let program = Path::new(r"C:\Program Files\Silt\silt.exe");
        let exec: Value = serde_json::from_str(&hook_settings("ignored", Some(program))).expect("valid json");
        let hook = exec.pointer("/hooks/Stop/0/hooks/0").expect("a Stop hook");
        assert_eq!(hook["command"], json!(r"C:\Program Files\Silt\silt.exe"));
        assert_eq!(hook["args"], json!(["hook", "Stop"]), "no shell reads the path");
        assert_eq!(
            settings.pointer("/hooks/SessionEnd/0/hooks/0/command"),
            Some(&json!("silt hook SessionEnd")),
            "Silt hears when the agent goes (ADR 0090)"
        );
        let ending: HookInput =
            serde_json::from_str(r#"{"session_id":"s","hook_event_name":"SessionEnd","reason":"clear"}"#).expect("parses");
        assert_eq!(ending.reason.as_deref(), Some("clear"));
        let prompt: HookInput =
            serde_json::from_str(r#"{"session_id":"s","hook_event_name":"UserPromptSubmit","prompt":"fix it"}"#).expect("parses");
        assert_eq!(prompt.prompt.as_deref(), Some("fix it"));
    }

    #[test]
    fn sessions_on_disk_are_listed_with_where_and_when_they_started() {
        let config = tempfile::tempdir().expect("temp dir");
        let project = config.path().join("projects/-w-repo");
        fs::create_dir_all(project.join("s1/subagents")).expect("mkdir");
        fs::write(
            project.join("s1.jsonl"),
            "{\"type\":\"summary\",\"summary\":\"x\"}\n{\"type\":\"user\",\"cwd\":\"/w/repo\",\"timestamp\":\"2026-01-02T03:04:05.000Z\"}\n",
        )
        .expect("writes");
        fs::write(project.join("s1/subagents/agent-a.jsonl"), "{}\n").expect("writes");
        fs::write(project.join("agent-old.jsonl"), "{}\n").expect("writes");
        fs::write(project.join("s0.jsonl"), "not json\n").expect("writes");
        let found = on_disk_in(config.path());
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!((found[0].id.as_str(), found[0].cwd.as_deref()), ("s0", None));
        assert_eq!(found[1].cwd.as_deref(), Some(Path::new("/w/repo")));
        assert_eq!(found[1].started_ms, crate::time::parse_timestamp("2026-01-02T03:04:05.000Z"));
    }
}
