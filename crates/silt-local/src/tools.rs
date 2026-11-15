//! `local`'s tools: reading, searching, editing and writing files, and
//! running commands. They act inside the project as the mode allows, and
//! always ask before touching a path outside it (ADR 0066).

use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use tokio::sync::watch;

use crate::conversation::Call;

const MOST_READ_LINES: usize = 2000;
const MOST_TOOL_BYTES: usize = 64 * 1024;
const MOST_LISTED: usize = 1000;
const MOST_FOUND: usize = 500;
const DEFAULT_TIMEOUT: u64 = 120;
const MOST_TIMEOUT: u64 = 600;
/// Folders a search doesn't go into.
const SKIPPED: [&str; 4] = [".git", "node_modules", "target", ".silt"];

/// What a call does, as ACP names its kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Read,
    Search,
    Edit,
    Execute,
}

/// How much the mode lets `local` do without asking, least first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Leeway {
    /// Chat: no tools at all.
    None,
    /// Reads alone; edits and commands are asked for.
    Reads,
    /// Reads and edits; commands are asked for.
    Edits,
    Everything,
}

impl Leeway {
    pub fn of_mode(mode: &str) -> Self {
        match mode {
            "default" => Self::Reads,
            "acceptEdits" => Self::Edits,
            "bypassPermissions" => Self::Everything,
            _ => Self::None,
        }
    }
}

/// A call understood: what it does, what it touches, and how to say so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    pub kind: Kind,
    pub title: String,
    /// The paths it reads or writes, resolved against the project.
    pub paths: Vec<PathBuf>,
    /// Whether any of them is outside the project.
    pub outside: bool,
    pub input: Value,
}

impl Planned {
    pub fn needs_asking(&self, leeway: Leeway) -> bool {
        self.outside
            || match self.kind {
                Kind::Read | Kind::Search => false,
                Kind::Edit => leeway < Leeway::Edits,
                Kind::Execute => leeway < Leeway::Everything,
            }
    }
}

/// What the model is offered, as OpenAI's API takes it.
pub fn definitions() -> Value {
    let tool = |name: &str, description: &str, properties: Value, required: &[&str]| {
        json!({"type": "function", "function": {
            "name": name,
            "description": description,
            "parameters": {"type": "object", "properties": properties, "required": required},
        }})
    };
    let path = json!({"type": "string", "description": "Relative to the project folder"});
    json!([
        tool(
            "read_file",
            "Read a text file, with line numbers",
            json!({
                "path": path,
                "offset": {"type": "integer", "description": "The first line to read, from 1"},
                "limit": {"type": "integer", "description": "How many lines to read"},
            }),
            &["path"]
        ),
        tool("list", "List a folder's entries; folders end in /", json!({"path": path}), &[]),
        tool(
            "glob",
            "Find files whose paths match a pattern such as src/**/*.rs",
            json!({
                "pattern": {"type": "string"},
                "path": path,
            }),
            &["pattern"]
        ),
        tool(
            "grep",
            "Search files' lines for a regular expression",
            json!({
                "pattern": {"type": "string"},
                "path": path,
                "glob": {"type": "string", "description": "Only files whose paths match this, such as *.log"},
            }),
            &["pattern"]
        ),
        tool(
            "edit",
            "Replace text in a file: old_text must occur exactly once",
            json!({
                "path": path,
                "old_text": {"type": "string"},
                "new_text": {"type": "string"},
            }),
            &["path", "old_text", "new_text"]
        ),
        tool(
            "write",
            "Write a whole file, creating it and its folders",
            json!({
                "path": path,
                "content": {"type": "string"},
            }),
            &["path", "content"]
        ),
        tool(
            "run",
            "Run a shell command in the project folder",
            json!({
                "command": {"type": "string"},
                "timeout": {"type": "integer", "description": "Seconds; 120 by default, 600 at most"},
            }),
            &["command"]
        ),
    ])
}

/// `path` against `root`, with `.` and `..` worked out without the disk.
pub fn resolve(root: &Path, path: &str) -> PathBuf {
    let mut resolved = PathBuf::new();
    for component in root.join(path).components() {
        match component {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::CurDir => {}
            other => resolved.push(other),
        }
    }
    resolved
}

fn text_arg<'a>(input: &'a Value, name: &str) -> Result<&'a str, String> {
    input[name].as_str().ok_or_else(|| format!("`{name}` is missing"))
}

/// Reads the model's call: what it does and touches. Its arguments may be
/// anything, so whatever doesn't fit is said back to it.
pub fn plan(call: &Call, root: &Path) -> Result<Planned, String> {
    let input: Value = serde_json::from_str(if call.arguments.trim().is_empty() { "{}" } else { &call.arguments })
        .map_err(|error| format!("the arguments aren't JSON: {error}"))?;
    let at = |name: &str| {
        input[name].as_str().filter(|path| !path.is_empty()).map_or_else(|| root.to_owned(), |path| resolve(root, path))
    };
    let (kind, title, paths) = match call.name.as_str() {
        "read_file" => {
            let path = resolve(root, text_arg(&input, "path")?);
            (Kind::Read, format!("Read {}", shown(root, &path)), vec![path])
        }
        "list" => {
            let path = at("path");
            (Kind::Read, format!("List {}", shown(root, &path)), vec![path])
        }
        "glob" => (Kind::Search, format!("Find {}", text_arg(&input, "pattern")?), vec![at("path")]),
        "grep" => {
            let path = at("path");
            (Kind::Search, format!("Search for {} in {}", text_arg(&input, "pattern")?, shown(root, &path)), vec![path])
        }
        "edit" | "write" => {
            let path = resolve(root, text_arg(&input, "path")?);
            let verb = if call.name == "edit" { "Edit" } else { "Write" };
            (Kind::Edit, format!("{verb} {}", shown(root, &path)), vec![path])
        }
        "run" => (Kind::Execute, format!("$ {}", text_arg(&input, "command")?), Vec::new()),
        other => return Err(format!("there's no tool {other}")),
    };
    let outside = paths.iter().any(|path| !path.starts_with(root));
    Ok(Planned { kind, title, paths, outside, input })
}

fn shown(root: &Path, path: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(inside) if inside.as_os_str().is_empty() => ".".to_owned(),
        Ok(inside) => inside.display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

/// What a call came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// What the model is told.
    pub text: String,
    pub failed: bool,
    /// An edit's file, before and after.
    pub diff: Option<(PathBuf, Option<String>, String)>,
}

impl Outcome {
    fn ok(text: impl Into<String>) -> Self {
        Self { text: text.into(), failed: false, diff: None }
    }

    pub fn failed(text: impl Into<String>) -> Self {
        Self { text: text.into(), failed: true, diff: None }
    }
}

fn capped(mut text: String) -> String {
    if text.len() > MOST_TOOL_BYTES {
        let mut end = MOST_TOOL_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n… (cut off)");
    }
    text
}

/// Does what `planned` says, in `root`, unless `cancel` stops it first.
pub async fn run(planned: &Planned, name: &str, root: &Path, cancel: watch::Receiver<bool>) -> Outcome {
    let input = &planned.input;
    let first = planned.paths.first().cloned().unwrap_or_else(|| root.to_owned());
    let result = match name {
        "read_file" => read(&first, input).await,
        "list" => list(&first).await,
        "glob" => found(root, &first, input, false).await,
        "grep" => found(root, &first, input, true).await,
        "edit" => edit(&first, input).await,
        "write" => write(&first, input).await,
        "run" => return command(root, input, cancel).await,
        other => Err(format!("there's no tool {other}")),
    };
    result.unwrap_or_else(Outcome::failed)
}

async fn read(path: &Path, input: &Value) -> Result<Outcome, String> {
    let text = tokio::fs::read_to_string(path).await.map_err(|error| format!("can't read {}: {error}", path.display()))?;
    let offset = input["offset"].as_u64().and_then(|offset| usize::try_from(offset).ok()).unwrap_or(1).max(1);
    let limit =
        input["limit"].as_u64().and_then(|limit| usize::try_from(limit).ok()).unwrap_or(MOST_READ_LINES).min(MOST_READ_LINES);
    let lines: Vec<String> =
        text.lines().enumerate().skip(offset - 1).take(limit).map(|(number, line)| format!("{}: {line}", number + 1)).collect();
    let total = text.lines().count();
    let mut shown = lines.join("\n");
    if offset - 1 + lines.len() < total {
        shown.push_str(&format!("\n… ({total} lines in all; read on with offset)"));
    }
    Ok(Outcome::ok(capped(shown)))
}

async fn list(path: &Path) -> Result<Outcome, String> {
    let mut entries = tokio::fs::read_dir(path).await.map_err(|error| format!("can't list {}: {error}", path.display()))?;
    let mut names = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let mut name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().await.is_ok_and(|kind| kind.is_dir()) {
            name.push('/');
        }
        names.push(name);
    }
    names.sort();
    let more = names.len().saturating_sub(MOST_LISTED);
    names.truncate(MOST_LISTED);
    if more > 0 {
        names.push(format!("… and {more} more"));
    }
    Ok(Outcome::ok(if names.is_empty() { "(empty)".to_owned() } else { names.join("\n") }))
}

/// Files under `dir`, as paths relative to `root`, skipping what's
/// generated or Silt's own.
fn walk(root: &Path, dir: &Path) -> Vec<(PathBuf, String)> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_owned()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if !SKIPPED.contains(&entry.file_name().to_string_lossy().as_ref()) {
                    pending.push(path);
                }
            } else if kind.is_file() {
                let relative = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
                found.push((path, relative));
            }
        }
    }
    found.sort_by(|a, b| a.1.cmp(&b.1));
    found
}

async fn found(root: &Path, dir: &Path, input: &Value, grep: bool) -> Result<Outcome, String> {
    let pattern = text_arg(input, "pattern")?.to_owned();
    let only = input["glob"].as_str().map(str::to_owned);
    let (root, dir) = (root.to_owned(), dir.to_owned());
    tokio::task::spawn_blocking(move || {
        let files = walk(&root, &dir);
        if !grep {
            let matching: Vec<&str> = files
                .iter()
                .map(|(_, relative)| relative.as_str())
                .filter(|relative| glob_matches(&pattern, relative))
                .take(MOST_FOUND)
                .collect();
            return Ok(Outcome::ok(if matching.is_empty() { "nothing matches".to_owned() } else { matching.join("\n") }));
        }
        let expression = regex::Regex::new(&pattern).map_err(|error| format!("not a regular expression: {error}"))?;
        let mut lines = Vec::new();
        for (path, relative) in &files {
            if only.as_deref().is_some_and(|only| !glob_matches(only, relative) && !glob_matches(only, file_name(relative))) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            for (number, line) in text.lines().enumerate() {
                if expression.is_match(line) {
                    lines.push(format!("{relative}:{}: {line}", number + 1));
                    if lines.len() == MOST_FOUND {
                        lines.push("… (more not shown)".to_owned());
                        return Ok(Outcome::ok(capped(lines.join("\n"))));
                    }
                }
            }
        }
        Ok(Outcome::ok(if lines.is_empty() { "nothing matches".to_owned() } else { capped(lines.join("\n")) }))
    })
    .await
    .map_err(|error| error.to_string())?
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Whether `path` matches `pattern`: `*` within a name, `**` across folders,
/// `?` one character.
pub fn glob_matches(pattern: &str, path: &str) -> bool {
    fn names(pattern: &[&str], path: &[&str]) -> bool {
        match (pattern.first(), path.first()) {
            (None, None) => true,
            (Some(&"**"), _) => names(&pattern[1..], path) || (!path.is_empty() && names(pattern, &path[1..])),
            (Some(part), Some(name)) => name_matches(part.as_bytes(), name.as_bytes()) && names(&pattern[1..], &path[1..]),
            _ => false,
        }
    }
    fn name_matches(pattern: &[u8], name: &[u8]) -> bool {
        match (pattern.first(), name.first()) {
            (None, None) => true,
            (Some(b'*'), _) => name_matches(&pattern[1..], name) || (!name.is_empty() && name_matches(pattern, &name[1..])),
            (Some(b'?'), Some(_)) => name_matches(&pattern[1..], &name[1..]),
            (Some(want), Some(have)) => want == have && name_matches(&pattern[1..], &name[1..]),
            _ => false,
        }
    }
    let pattern: Vec<&str> = pattern.split('/').filter(|part| !part.is_empty()).collect();
    let path: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    names(&pattern, &path)
}

async fn edit(path: &Path, input: &Value) -> Result<Outcome, String> {
    let old_text = text_arg(input, "old_text")?;
    let new_text = text_arg(input, "new_text")?;
    let before = tokio::fs::read_to_string(path).await.map_err(|error| format!("can't read {}: {error}", path.display()))?;
    match before.matches(old_text).count() {
        0 => return Err(format!("old_text isn't in {}; read it again and copy the text exactly", path.display())),
        1 => {}
        many => return Err(format!("old_text occurs {many} times in {}; include more around it", path.display())),
    }
    let after = before.replacen(old_text, new_text, 1);
    tokio::fs::write(path, &after).await.map_err(|error| format!("can't write {}: {error}", path.display()))?;
    Ok(Outcome { text: format!("Edited {}", path.display()), failed: false, diff: Some((path.to_owned(), Some(before), after)) })
}

async fn write(path: &Path, input: &Value) -> Result<Outcome, String> {
    let content = text_arg(input, "content")?;
    let before = tokio::fs::read_to_string(path).await.ok();
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|error| format!("can't make {}: {error}", parent.display()))?;
    }
    tokio::fs::write(path, content).await.map_err(|error| format!("can't write {}: {error}", path.display()))?;
    Ok(Outcome {
        text: format!("Wrote {}", path.display()),
        failed: false,
        diff: Some((path.to_owned(), before, content.to_owned())),
    })
}

async fn command(root: &Path, input: &Value, cancel: watch::Receiver<bool>) -> Outcome {
    let Ok(line) = text_arg(input, "command") else {
        return Outcome::failed("`command` is missing");
    };
    let timeout = Duration::from_secs(input["timeout"].as_u64().unwrap_or(DEFAULT_TIMEOUT).clamp(1, MOST_TIMEOUT));
    let mut shell = if cfg!(windows) {
        let mut shell = tokio::process::Command::new("cmd");
        shell.arg("/C");
        shell
    } else {
        let mut shell = tokio::process::Command::new("sh");
        shell.arg("-c");
        shell
    };
    shell.arg(line).current_dir(root).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = match shell.spawn() {
        Ok(child) => child,
        Err(error) => return Outcome::failed(format!("couldn't run it: {error}")),
    };
    let (mut stdout, mut stderr) = (child.stdout.take().expect("piped"), child.stderr.take().expect("piped"));
    let gathered = async move {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let _ = tokio::join!(stdout.read_to_end(&mut out), stderr.read_to_end(&mut err));
        let status = child.wait().await;
        (out, err, status)
    };
    let (out, err, status) = tokio::select! {
        done = gathered => done,
        () = tokio::time::sleep(timeout) => return Outcome::failed(format!("it ran past {} seconds and was stopped", timeout.as_secs())),
        () = crate::model::cancelled(cancel) => return Outcome::failed("stopped: the turn was cancelled"),
    };
    let mut text = String::from_utf8_lossy(&out).into_owned();
    let errors = String::from_utf8_lossy(&err);
    if !errors.trim().is_empty() {
        text.push_str(&errors);
    }
    let code = status.ok().and_then(|status| status.code());
    let tail = keep_tail(text);
    match code {
        Some(0) => Outcome::ok(if tail.trim().is_empty() { "(no output)".to_owned() } else { tail }),
        Some(code) => Outcome::failed(format!("{tail}\n(exit code {code})")),
        None => Outcome::failed(format!("{tail}\n(ended by a signal)")),
    }
}

/// The end of a long output, which is where errors usually are.
fn keep_tail(text: String) -> String {
    if text.len() <= MOST_TOOL_BYTES {
        return text;
    }
    let mut start = text.len() - MOST_TOOL_BYTES;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("(… cut off)\n{}", &text[start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, arguments: Value) -> Call {
        Call { id: "c".into(), name: name.into(), arguments: arguments.to_string() }
    }

    #[test]
    fn a_path_outside_the_project_always_asks() {
        let root = Path::new("/work/app");
        let inside = plan(&call("read_file", json!({"path": "src/main.rs"})), root).expect("planned");
        assert!(!inside.outside);
        assert_eq!(inside.title, "Read src/main.rs");
        assert!(!inside.needs_asking(Leeway::Reads));
        let escaping = plan(&call("read_file", json!({"path": "../../etc/passwd"})), root).expect("planned");
        assert!(escaping.outside);
        assert!(escaping.needs_asking(Leeway::Everything));
        let absolute = plan(&call("write", json!({"path": "/tmp/x", "content": ""})), root).expect("planned");
        assert!(absolute.outside && absolute.needs_asking(Leeway::Everything));
    }

    #[test]
    fn the_mode_says_what_needs_asking() {
        let root = Path::new("/work/app");
        let edit = plan(&call("edit", json!({"path": "a", "old_text": "x", "new_text": "y"})), root).expect("planned");
        let run = plan(&call("run", json!({"command": "ls"})), root).expect("planned");
        assert!(edit.needs_asking(Leeway::Reads));
        assert!(!edit.needs_asking(Leeway::Edits));
        assert!(run.needs_asking(Leeway::Edits));
        assert!(!run.needs_asking(Leeway::Everything));
        assert_eq!(run.title, "$ ls");
        assert_eq!(Leeway::of_mode("acceptEdits"), Leeway::Edits);
        assert_eq!(Leeway::of_mode("chat"), Leeway::None);
    }

    #[test]
    fn what_the_model_gets_wrong_is_said_back() {
        let root = Path::new("/r");
        assert_eq!(plan(&call("delete_all", json!({})), root), Err("there's no tool delete_all".to_owned()));
        assert_eq!(plan(&call("read_file", json!({})), root), Err("`path` is missing".to_owned()));
        let broken = Call { id: "c".into(), name: "list".into(), arguments: "{not json".into() };
        assert!(plan(&broken, root).is_err_and(|why| why.starts_with("the arguments aren't JSON")));
        assert!(plan(&Call { id: "c".into(), name: "list".into(), arguments: String::new() }, root).is_ok());
    }

    #[test]
    fn globs_match_names_and_folders() {
        assert!(glob_matches("src/**/*.rs", "src/a/b/main.rs"));
        assert!(glob_matches("src/**/*.rs", "src/main.rs"));
        assert!(glob_matches("*.log", "app.log"));
        assert!(!glob_matches("*.log", "logs/app.log"));
        assert!(glob_matches("**/*.log", "logs/app.log"));
        assert!(glob_matches("a?c", "abc"));
        assert!(!glob_matches("src/*.rs", "src/a/main.rs"));
    }

    #[tokio::test]
    async fn files_are_read_searched_edited_and_written() {
        let dir = tempfile::tempdir().expect("temp");
        let root = dir.path();
        let (_, idle) = watch::channel(false);
        let go = |name: &str, input: Value| {
            let planned = plan(&call(name, input), root).expect("planned");
            let idle = idle.clone();
            let name = name.to_owned();
            async move { run(&planned, &name, root, idle).await }
        };
        let wrote = go("write", json!({"path": "logs/app.log", "content": "ok\nERROR disk full\nok\n"})).await;
        assert!(!wrote.failed && wrote.diff.as_ref().is_some_and(|(_, before, _)| before.is_none()));
        assert_eq!(
            go("read_file", json!({"path": "logs/app.log", "offset": 2, "limit": 1})).await.text,
            "2: ERROR disk full\n… (3 lines in all; read on with offset)"
        );
        assert_eq!(go("grep", json!({"pattern": "ERROR"})).await.text, "logs/app.log:2: ERROR disk full");
        assert_eq!(go("grep", json!({"pattern": "ERROR", "glob": "*.txt"})).await.text, "nothing matches");
        assert_eq!(go("glob", json!({"pattern": "**/*.log"})).await.text, "logs/app.log");
        assert_eq!(go("list", json!({})).await.text, "logs/");
        let edited = go("edit", json!({"path": "logs/app.log", "old_text": "disk full", "new_text": "disk freed"})).await;
        assert!(!edited.failed);
        assert!(
            go("edit", json!({"path": "logs/app.log", "old_text": "ok", "new_text": "x"})).await.text.contains("occurs 2 times")
        );
        assert!(std::fs::read_to_string(root.join("logs/app.log")).expect("read").contains("disk freed"));
    }

    #[tokio::test]
    async fn a_command_says_its_output_and_how_it_ended() {
        let dir = tempfile::tempdir().expect("temp");
        let (_, idle) = watch::channel(false);
        let ran = |line: &str| {
            let planned = plan(&call("run", json!({"command": line})), dir.path()).expect("planned");
            let idle = idle.clone();
            let root = dir.path().to_owned();
            async move { run(&planned, "run", &root, idle).await }
        };
        let fine = ran("echo hello").await;
        assert_eq!((fine.text.trim(), fine.failed), ("hello", false));
        let failed = ran("echo oops >&2; exit 3").await;
        assert!(failed.failed && failed.text.contains("oops") && failed.text.contains("(exit code 3)"), "{failed:?}");
    }
}
