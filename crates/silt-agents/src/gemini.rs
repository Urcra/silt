//! Gemini CLI: sessions are append-only JSONL in
//! `~/.gemini/tmp/<project slug>/chats/session-<time>-<id8>.jsonl`. A
//! message is written again whenever it grows, so the reader keys messages
//! by id (docs/research/agents.md).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::conventions::NativeConventions;
use crate::{
    Agent, Entry, Found, Launch, LaunchPlan, Mark, Read, Reader, RecordedSession, Rewrites, Role, SessionIds, Start,
    translated_text,
};

pub const AGENT: &str = "gemini-cli";

/// The directory holding `.gemini`: `$GEMINI_CLI_HOME` or the home dir.
pub fn home() -> Option<PathBuf> {
    std::env::var_os("GEMINI_CLI_HOME").map(PathBuf::from).or_else(crate::home)
}

fn gemini_dir() -> Option<PathBuf> {
    home().map(|home| home.join(".gemini"))
}

/// The slug Gemini CLI derives from a project's directory name.
pub fn base_slug(cwd: &Path) -> String {
    let name = cwd.file_name().map(|name| name.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut slug = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-').to_owned();
    if slug.is_empty() { "project".to_owned() } else { slug }
}

fn registry(gemini: &Path) -> Map<String, Value> {
    fs::read_to_string(gemini.join("projects.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| value.get("projects").and_then(Value::as_object).cloned())
        .unwrap_or_default()
}

/// The project's chats directory if Gemini has registered `cwd`, in its
/// registry or by the owner marker in a project directory, as Gemini
/// itself looks it up.
fn chats_dir(gemini: &Path, cwd: &Path) -> Option<PathBuf> {
    let path = cwd.to_string_lossy();
    let slug = registry(gemini).get(&*path).and_then(Value::as_str).map(str::to_owned).or_else(|| {
        fs::read_dir(gemini.join("tmp")).ok()?.flatten().find_map(|entry| {
            let owner = fs::read_to_string(entry.path().join(".project_root")).ok()?;
            (owner.trim() == path).then(|| entry.file_name().to_string_lossy().into_owned())
        })
    })?;
    Some(gemini.join("tmp").join(slug).join("chats"))
}

/// What Gemini records as a session's `projectHash`: the SHA-256 of the
/// project directory, in hex.
pub fn project_hash(cwd: &Path) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(cwd.to_string_lossy().as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Registers `cwd` the way Gemini CLI does, so a restored session is found
/// where Gemini will look for it.
fn claim_chats_dir(gemini: &Path, cwd: &Path) -> io::Result<PathBuf> {
    if let Some(dir) = chats_dir(gemini, cwd) {
        return Ok(dir);
    }
    let path = cwd.to_string_lossy().into_owned();
    let mut projects = registry(gemini);
    let taken: HashSet<String> = projects.values().filter_map(Value::as_str).map(str::to_owned).collect();
    let base = base_slug(cwd);
    let slug = (0..)
        .map(|n| if n == 0 { base.clone() } else { format!("{base}-{n}") })
        .find(|slug| match fs::read_to_string(gemini.join("tmp").join(slug).join(".project_root")) {
            Ok(owner) => owner.trim() == path,
            Err(_) => !taken.contains(slug),
        })
        .expect("some slug is free");
    for root in ["tmp", "history"] {
        let dir = gemini.join(root).join(&slug);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join(".project_root"), &path)?;
    }
    projects.insert(path, Value::String(slug.clone()));
    let text = json!({ "projects": projects }).to_string();
    let temporary = gemini.join("projects.json.silt");
    fs::write(&temporary, text)?;
    fs::rename(temporary, gemini.join("projects.json"))?;
    Ok(gemini.join("tmp").join(slug).join("chats"))
}

fn session_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|extension| extension == "jsonl")
                && path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("session-"))
        })
        .collect()
}

fn file_session_id(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut line = String::new();
    BufReader::new(file).read_line(&mut line).ok()?;
    let value: Value = serde_json::from_str(&line).ok()?;
    value.get("sessionId")?.as_str().map(str::to_owned)
}

/// The newest session of `cwd` written since `since_ms` in the `.gemini`
/// directory `gemini`, whose id isn't in `known`.
pub fn discover_in(gemini: &Path, cwd: &Path, since_ms: i64, known: &[String]) -> Option<Found> {
    let dir = chats_dir(gemini, cwd)?;
    session_files(&dir)
        .into_iter()
        .filter(|path| crate::modified_ms(path).is_some_and(|modified| modified >= since_ms))
        .filter_map(|path| Some((file_session_id(&path)?, path)))
        .filter(|(id, _)| !known.contains(id))
        .max_by_key(|(_, path)| crate::modified_ms(path))
        .map(|(id, transcript)| Found { id, transcript })
}

/// Writes `session` where the `.gemini` directory `gemini` finds it for
/// `cwd`, with its subagents' sessions.
pub fn restore_into(gemini: &Path, root: &Path, cwd: &Path, session: &RecordedSession) -> io::Result<PathBuf> {
    let dir = claim_chats_dir(gemini, cwd)?;
    let path = session_files(&dir)
        .into_iter()
        .find(|path| file_session_id(path).as_deref() == Some(&*session.id))
        .unwrap_or_else(|| {
            let stamp = crate::time::file_stamp(crate::now_ms());
            let short: String = session.id.chars().take(8).collect();
            dir.join(format!("session-{}-{short}.jsonl", &stamp[..16]))
        });
    let rewrites = Rewrites::new(session, None, &cwd.to_string_lossy(), &root.to_string_lossy());
    fs::create_dir_all(&dir)?;
    crate::replace_transcript(&path, &rewrites.lines(&session.main))?;
    if !session.subagents.is_empty() {
        let subagents = dir.join(&session.id);
        fs::create_dir_all(&subagents)?;
        for (name, lines) in &session.subagents {
            crate::replace_transcript(&subagents.join(format!("{name}.jsonl")), &rewrites.lines(lines))?;
        }
    }
    Ok(path)
}

/// Where the session in `transcript` keeps its subagents' sessions.
fn subagents_dir(transcript: &Path) -> Option<PathBuf> {
    let id = file_session_id(transcript).filter(|id| crate::is_safe_id(id))?;
    Some(transcript.parent()?.join(id))
}

#[derive(Default)]
struct Seen {
    kind: String,
    text: bool,
    thoughts: usize,
    calls: HashSet<String>,
    stopped: bool,
}

/// Folds a session file's records.
#[derive(Default)]
pub struct GeminiReader {
    messages: HashMap<String, Seen>,
    results: HashSet<String>,
}

impl GeminiReader {
    fn result(&mut self, read: &mut Read, id: &str, text: String, is_error: bool) {
        if self.results.insert(id.to_owned()) {
            read.marks.push(Mark::ToolFinished { tool_use_id: id.to_owned() });
            read.entries.push(Entry::ToolResult { tool_use_id: id.to_owned(), text, is_error });
        }
    }
}

impl Reader for GeminiReader {
    fn push(&mut self, line: &str) -> Read {
        let mut read = Read::default();
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Read::unparseable();
        };
        if value.get("id").is_some() {
            self.message(&value, &mut read);
            return read;
        }
        let kind =
            ["$set", "$patch", "$rewindTo", "sessionId"].into_iter().find(|key| value.get(key).is_some()).unwrap_or("unknown");
        read.entries.push(Entry::Other { kind: kind.trim_start_matches('$').to_owned() });
        // An update carries what a message gained, like a copy of it.
        let updates = value.pointer("/$patch/updates").and_then(Value::as_array).cloned().unwrap_or_default();
        for mut update in updates {
            let known =
                update.get("id").and_then(Value::as_str).and_then(|id| self.messages.get(id)).map(|seen| seen.kind.clone());
            if let (Some(kind), Some(fields)) = (known, update.as_object_mut()) {
                fields.entry("type").or_insert(Value::String(kind));
                self.message(&update, &mut read);
            }
        }
        read
    }
}

impl GeminiReader {
    fn message(&mut self, value: &Value, read: &mut Read) {
        let Some(id) = value.get("id").and_then(Value::as_str) else {
            return;
        };
        let kind = value.get("type").and_then(Value::as_str).unwrap_or_default();
        let content = value.get("content");
        // On resume Gemini repeats the tool results of the last turn under
        // `<message id>_response`; they were read with the turn.
        if id.ends_with("_response") {
            read.entries.push(Entry::Other { kind: "replayed".into() });
            return;
        }
        let seen = self.messages.entry(id.to_owned()).or_default();
        kind.clone_into(&mut seen.kind);
        match kind {
            "user" => {
                let responses: Vec<(String, String)> = parts(content)
                    .iter()
                    .filter_map(|part| {
                        let response = part.get("functionResponse")?;
                        Some((
                            response.get("id").and_then(Value::as_str).unwrap_or_default().to_owned(),
                            response_text(response.get("response")),
                        ))
                    })
                    .collect();
                let text = content_text(content);
                if !seen.text && !text.is_empty() {
                    seen.text = true;
                    let trimmed = text.trim_start();
                    if trimmed.starts_with("<session_context>") || trimmed.starts_with("<hook_context>") {
                        read.entries.push(Entry::Other { kind: "context".into() });
                    } else if trimmed.starts_with("<state_snapshot>") {
                        read.entries.push(Entry::Compaction { summary: Some(text) });
                    } else {
                        read.marks.push(Mark::PromptSubmitted);
                        read.entries.push(Entry::User { text });
                    }
                }
                for (call, text) in responses {
                    self.result(read, &call, text, false);
                }
            }
            "gemini" => {
                let thoughts = value.get("thoughts").and_then(Value::as_array).cloned().unwrap_or_default();
                for thought in thoughts.iter().skip(seen.thoughts) {
                    let subject = thought.get("subject").and_then(Value::as_str).unwrap_or_default();
                    let description = thought.get("description").and_then(Value::as_str).unwrap_or_default();
                    read.entries
                        .push(Entry::Thinking { text: format!("{subject}: {description}").trim_matches([' ', ':']).to_owned() });
                }
                seen.thoughts = seen.thoughts.max(thoughts.len());
                let calls = value.get("toolCalls").and_then(Value::as_array).cloned().unwrap_or_default();
                let text = content_text(content);
                if !seen.text && !text.is_empty() {
                    seen.text = true;
                    read.entries.push(Entry::Assistant { text });
                }
                // Gemini writes a reply as it streams and adds its tool calls
                // only once they ran, so a reply without calls ends the turn
                // only once its tokens, which come at the end, are counted.
                let requests_tools = !calls.is_empty() || parts(content).iter().any(|part| part.get("functionCall").is_some());
                if seen.text && !requests_tools && value.get("tokens").is_some() && !seen.stopped {
                    seen.stopped = true;
                    read.marks.push(Mark::Stopped);
                }
                let mut results = Vec::new();
                for call in &calls {
                    let call_id = call.get("id").and_then(Value::as_str).unwrap_or_default().to_owned();
                    if seen.calls.insert(call_id.clone()) {
                        let name = call.get("name").and_then(Value::as_str).unwrap_or_default().to_owned();
                        read.marks.push(Mark::ToolStarted { tool_use_id: call_id.clone(), tool: name.clone() });
                        read.entries.push(Entry::ToolCall {
                            id: call_id.clone(),
                            name,
                            input: call.get("args").cloned().unwrap_or(Value::Null),
                        });
                    }
                    if let Some(result) = call.get("result").filter(|result| !result.is_null()) {
                        let text = parts(Some(result))
                            .iter()
                            .map(|part| response_text(part.pointer("/functionResponse/response")))
                            .collect::<Vec<_>>()
                            .join("\n");
                        let failed = call.get("status").and_then(Value::as_str).is_some_and(|status| status == "error");
                        results.push((call_id, text, failed));
                    }
                }
                for (call, text, failed) in results {
                    self.result(read, &call, text, failed);
                }
            }
            "error" => {
                // The request failed, which ends the turn.
                if !seen.stopped {
                    seen.stopped = true;
                    read.marks.push(Mark::Stopped);
                }
                read.entries.push(Entry::Other { kind: "error".into() });
            }
            other => read.entries.push(Entry::Other { kind: other.to_owned() }),
        }
    }
}

fn parts(content: Option<&Value>) -> Vec<Value> {
    match content {
        Some(Value::Array(parts)) => parts.clone(),
        Some(part @ Value::Object(_)) => vec![part.clone()],
        _ => Vec::new(),
    }
}

/// Text of a message, leaving out thought parts.
fn content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        other => parts(other)
            .iter()
            .filter(|part| part.get("thought").and_then(Value::as_bool) != Some(true))
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
    }
}

fn response_text(response: Option<&Value>) -> String {
    match response {
        Some(Value::Object(fields)) => match fields.get("output").or_else(|| fields.get("error")) {
            Some(Value::String(text)) => text.clone(),
            Some(other) => other.to_string(),
            None => Value::Object(fields.clone()).to_string(),
        },
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

/// Session lines holding `messages` as plain text. Gemini skips sessions
/// without a `projectHash`.
pub fn session_lines(messages: &[(Role, String)], cwd: &Path, id: &str) -> Vec<String> {
    let timestamp = crate::time::timestamp_now();
    let mut lines = vec![
        json!({"sessionId": id, "projectHash": project_hash(cwd), "startTime": timestamp, "lastUpdated": timestamp, "kind": "main"}).to_string(),
    ];
    for (index, (role, text)) in messages.iter().enumerate() {
        let message = match role {
            Role::User => {
                json!({"id": format!("silt-{index}"), "timestamp": timestamp, "type": "user", "content": [{"text": text}]})
            }
            Role::Assistant => {
                json!({"id": format!("silt-{index}"), "timestamp": timestamp, "type": "gemini", "content": text, "thoughts": []})
            }
        };
        lines.push(message.to_string());
    }
    lines
}

/// The Gemini CLI adapter.
pub struct Gemini;

impl Agent for Gemini {
    fn id(&self) -> &'static str {
        AGENT
    }

    fn command(&self) -> &'static str {
        "gemini"
    }

    fn display_name(&self) -> &'static str {
        "Gemini CLI"
    }

    fn native(&self) -> NativeConventions {
        NativeConventions {
            rules: &["GEMINI.md"],
            skill_dirs: &[".agents/skills", ".gemini/skills"],
            personal_skills: true,
            given_skills: false,
        }
    }

    fn reader(&self) -> Box<dyn Reader> {
        Box::<GeminiReader>::default()
    }

    fn session_ids(&self) -> SessionIds {
        SessionIds::Chosen
    }

    /// Subagent sessions, in `chats/<session id>/<subagent id>.jsonl`.
    fn side_transcripts(&self, transcript: &Path) -> Vec<(String, PathBuf)> {
        let Some(dir) = subagents_dir(transcript) else {
            return Vec::new();
        };
        let Ok(entries) = fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut found: Vec<(String, PathBuf)> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "jsonl"))
            .filter_map(|path| Some((path.file_stem()?.to_string_lossy().into_owned(), path.clone())))
            .filter(|(name, _)| crate::is_safe_id(name))
            .collect();
        found.sort();
        found
    }

    fn transcript(&self, cwd: &Path, id: &str) -> Option<PathBuf> {
        let dir = chats_dir(&gemini_dir()?, cwd)?;
        session_files(&dir).into_iter().find(|path| file_session_id(path).as_deref() == Some(id))
    }

    fn discover(&self, cwd: &Path, since_ms: i64, known: &[String]) -> Option<Found> {
        discover_in(&gemini_dir()?, cwd, since_ms, known)
    }

    fn restore(&self, root: &Path, cwd: &Path, session: &RecordedSession) -> io::Result<PathBuf> {
        let gemini = gemini_dir().ok_or_else(|| io::Error::other("HOME is not set"))?;
        restore_into(&gemini, root, cwd, session)
    }

    fn translate(&self, entries: &[Entry], cwd: &Path, id: &str) -> Option<Vec<String>> {
        Some(session_lines(&translated_text(entries), cwd, id))
    }

    fn launch(&self, plan: &LaunchPlan) -> io::Result<Launch> {
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
        // Registered before the run: Gemini's own registration replaces
        // the overlay's copy, which is dropped afterwards.
        claim_chats_dir(&gemini_dir().ok_or_else(|| io::Error::other("HOME is not set"))?, &plan.cwd)?;
        let overlay = overlay_home(plan)?;
        launch.env("GEMINI_CLI_HOME", overlay);
        for arg in &plan.args {
            launch.arg(arg);
        }
        if let Some(prompt) = prompt {
            launch.arg("--prompt-interactive").arg(prompt);
        }
        Ok(launch)
    }
}

/// Gemini CLI takes no per-run settings, so it runs with a home of its own
/// that links everything to the real `~/.gemini` except the settings,
/// which add Silt's MCP server, the global context file, which adds the
/// session's instructions, and the project registry, which Gemini replaces
/// by renaming a new file over it. Sessions still land in the real home.
fn overlay_home(plan: &LaunchPlan) -> io::Result<PathBuf> {
    let real = gemini_dir().ok_or_else(|| io::Error::other("HOME is not set"))?;
    fs::create_dir_all(&real)?;
    let overlay = plan.scratch.join("gemini-home");
    if overlay.exists() {
        fs::remove_dir_all(&overlay)?;
    }
    let dir = overlay.join(".gemini");
    fs::create_dir_all(&dir)?;
    if let Some(agents) = home().map(|home| home.join(".agents")).filter(|path| path.exists()) {
        crate::symlink(&agents, &overlay.join(".agents"))?;
    }
    for entry in fs::read_dir(&real)?.flatten() {
        let name = entry.file_name();
        if name != "settings.json" && name != "GEMINI.md" && name != "projects.json" {
            crate::symlink(&entry.path(), &dir.join(&name))?;
        }
    }
    for shared in ["tmp", "history"] {
        let path = real.join(shared);
        if !dir.join(shared).exists() {
            fs::create_dir_all(&path)?;
            crate::symlink(&path, &dir.join(shared))?;
        }
    }
    if real.join("projects.json").exists() {
        fs::copy(real.join("projects.json"), dir.join("projects.json"))?;
    }
    let mut settings: Value = fs::read_to_string(real.join("settings.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let servers = settings.as_object_mut().expect("checked above").entry("mcpServers").or_insert_with(|| json!({}));
    if let Some(servers) = servers.as_object_mut() {
        servers.insert("silt".into(), json!({"command": plan.mcp.command.to_string_lossy(), "args": plan.mcp.args}));
    }
    fs::write(dir.join("settings.json"), settings.to_string())?;
    let mut context = fs::read_to_string(real.join("GEMINI.md")).unwrap_or_default();
    if let Some(instructions) = &plan.instructions {
        if !context.is_empty() {
            context.push_str("\n\n");
        }
        context.push_str(instructions);
    }
    if !context.is_empty() {
        fs::write(dir.join("GEMINI.md"), context)?;
    }
    Ok(overlay)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_follow_gemini() {
        assert_eq!(base_slug(Path::new("/w/My_Proj")), "my-proj");
        assert_eq!(base_slug(Path::new("/w/other.repo")), "other-repo");
    }

    #[test]
    fn repeated_messages_are_read_once() {
        let mut reader = GeminiReader::default();
        let lines = [
            r#"{"sessionId":"s","projectHash":"h","startTime":"t","lastUpdated":"t","kind":"main"}"#,
            r#"{"id":"c","timestamp":"t","type":"user","content":[{"text":"<session_context>\nGemini</session_context>"}]}"#,
            r#"{"id":"u","timestamp":"t","type":"user","content":[{"text":"list files in src"}]}"#,
            r#"{"id":"g","timestamp":"t","type":"gemini","content":"","thoughts":[]}"#,
            r#"{"id":"g","timestamp":"t","type":"gemini","content":"","thoughts":[],"toolCalls":[{"id":"ls_1","name":"list_directory","args":{"dir_path":"src"},"result":[{"functionResponse":{"id":"ls_1","name":"list_directory","response":{"output":"main.rs"}}}],"status":"success"}]}"#,
            r#"{"id":"r","timestamp":"t","type":"user","content":[{"functionResponse":{"id":"ls_1","name":"list_directory","response":{"output":"main.rs"}}}]}"#,
            r#"{"id":"g_response","timestamp":"t","type":"user","content":[{"functionResponse":{"id":"old_1","name":"x","response":{"output":"again"}}}]}"#,
            r#"{"$set":{"lastUpdated":"t"}}"#,
            r#"{"id":"a","timestamp":"t","type":"gemini","content":"There is one file.","thoughts":[{"subject":"Looking","description":"at src"}]}"#,
            r#"{"id":"a","timestamp":"t","type":"gemini","content":"There is one file.","thoughts":[{"subject":"Looking","description":"at src"}],"tokens":{"total":1}}"#,
        ];
        let reads: Vec<Read> = lines.iter().map(|line| reader.push(line)).collect();
        let entries: Vec<Entry> =
            reads.iter().flat_map(|read| read.entries.clone()).filter(|entry| !matches!(entry, Entry::Other { .. })).collect();
        assert_eq!(
            entries,
            vec![
                Entry::User { text: "list files in src".into() },
                Entry::ToolCall { id: "ls_1".into(), name: "list_directory".into(), input: json!({"dir_path": "src"}) },
                Entry::ToolResult { tool_use_id: "ls_1".into(), text: "main.rs".into(), is_error: false },
                Entry::Thinking { text: "Looking: at src".into() },
                Entry::Assistant { text: "There is one file.".into() },
            ]
        );
        let marks: Vec<Mark> = reads.into_iter().flat_map(|read| read.marks).collect();
        assert_eq!(marks.first(), Some(&Mark::PromptSubmitted));
        assert_eq!(marks.last(), Some(&Mark::Stopped));
    }

    #[test]
    fn a_reply_ends_the_turn_only_once_it_finished_without_tools() {
        let mut reader = GeminiReader::default();
        let marks = |reader: &mut GeminiReader, line: &str| reader.push(line).marks;
        marks(&mut reader, r#"{"id":"u","timestamp":"t","type":"user","content":[{"text":"fix it"}]}"#);
        let streaming = r#"{"id":"g","timestamp":"t","type":"gemini","content":"I'll edit the file.","thoughts":[]}"#;
        assert_eq!(marks(&mut reader, streaming), Vec::new());
        let counted = r#"{"id":"g","timestamp":"t","type":"gemini","content":[{"text":"I'll edit the file."},{"functionCall":{"name":"replace","args":{}}}],"thoughts":[],"tokens":{"total":1}}"#;
        assert_eq!(marks(&mut reader, counted), Vec::new(), "a reply that asked for tools");
        let ran = r#"{"$patch":{"updates":[{"id":"g","toolCalls":[{"id":"e1","name":"replace","args":{},"result":[{"functionResponse":{"id":"e1","response":{"output":"ok"}}}],"status":"success"}]}]}}"#;
        assert_eq!(
            marks(&mut reader, ran),
            vec![
                Mark::ToolStarted { tool_use_id: "e1".into(), tool: "replace".into() },
                Mark::ToolFinished { tool_use_id: "e1".into() }
            ],
            "a patch is read like a copy of the message"
        );
        let answer = r#"{"id":"a","timestamp":"t","type":"gemini","content":"Done.","thoughts":[]}"#;
        assert_eq!(marks(&mut reader, answer), Vec::new());
        let finished = r#"{"id":"a","timestamp":"t","type":"gemini","content":"Done.","thoughts":[],"tokens":{"total":2}}"#;
        assert_eq!(marks(&mut reader, finished), vec![Mark::Stopped]);
        assert_eq!(marks(&mut reader, finished), Vec::new(), "once");
    }

    #[test]
    fn subagent_sessions_are_listed_and_restored_with_their_parent() {
        let gemini = tempfile::tempdir().expect("temp dir");
        let session = RecordedSession {
            id: "new-session".into(),
            former_ids: vec!["old-session".into()],
            former_cwds: vec!["/old/repo".into()],
            main: vec![r#"{"sessionId":"old-session","projectHash":"h","kind":"main"}"#.into()],
            subagents: vec![(
                "helper-1".into(),
                vec![r#"{"sessionId":"helper-1","kind":"subagent","directories":["/old/repo"]}"#.into()],
            )],
            ..Default::default()
        };
        let main = restore_into(gemini.path(), Path::new("/new"), Path::new("/new/repo"), &session).expect("restores");
        let side = Gemini.side_transcripts(&main);
        assert_eq!(side.len(), 1);
        assert_eq!(side[0].0, "helper-1");
        assert_eq!(side[0].1, main.parent().expect("chats").join("new-session/helper-1.jsonl"));
        assert!(fs::read_to_string(&side[0].1).expect("reads").contains("/new/repo"));
    }

    #[test]
    fn translated_sessions_name_their_project() {
        let lines = session_lines(&[(Role::User, "hi".into())], Path::new("/w/repo"), "s");
        let meta: Value = serde_json::from_str(&lines[0]).expect("json");
        assert_eq!(meta["projectHash"].as_str().map(str::len), Some(64));
    }

    #[test]
    fn restored_sessions_are_registered_where_gemini_looks() {
        let gemini = tempfile::tempdir().expect("temp dir");
        let dir = claim_chats_dir(gemini.path(), Path::new("/w/My_Proj")).expect("claims");
        assert_eq!(dir, gemini.path().join("tmp/my-proj/chats"));
        let other = claim_chats_dir(gemini.path(), Path::new("/x/my-proj")).expect("claims");
        assert_eq!(other, gemini.path().join("tmp/my-proj-1/chats"));
        assert_eq!(chats_dir(gemini.path(), Path::new("/w/My_Proj")), Some(dir.clone()));
        fs::write(gemini.path().join("projects.json"), r#"{"projects":{}}"#).expect("write");
        assert_eq!(chats_dir(gemini.path(), Path::new("/w/My_Proj")), Some(dir), "found by its marker");
    }
}
