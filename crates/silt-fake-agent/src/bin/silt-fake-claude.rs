//! A stand-in for Claude Code in the multi-machine tests, on every
//! platform. A first run records a session: a prompt, an edit of
//! `src/main.rs` (`fn farewell() {}`), a saved tool result and an answer,
//! running the hooks `--settings` names as Claude Code does. `--resume`
//! checks the restored transcript names this directory, the conversation
//! and the tool result, and mentions every `FAKE_EXPECT` word (dashes for
//! spaces), then adds `FAKE_SAY` and the line `FAKE_LINE`. With
//! `FAKE_IN_TOOL` it touches that file during the edit and waits for
//! `<it>.go`. `FAKE_TRANSLATED` skips the tool result check, for a session
//! translated from another agent.

use std::env;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

use serde_json::{Value, json};

struct Run {
    session: String,
    settings: Option<Value>,
    cwd: PathBuf,
    dir: PathBuf,
    transcript: PathBuf,
}

/// Exits the fake with a code a test can tell apart, as the shell stand-in
/// did.
struct Failed(u8, String);

type Step = Result<(), Failed>;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failed(code, message)) => {
            if !message.is_empty() {
                eprintln!("{message}");
            }
            ExitCode::from(code)
        }
    }
}

fn run() -> Step {
    let mut args = env::args().skip(1);
    let (mut session, mut resume, mut settings) = (String::new(), false, None);
    let log_dir = env::var_os("FAKE_LOG_DIR").map(PathBuf::from);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--session-id" => session = args.next().unwrap_or_default(),
            "--resume" => {
                session = args.next().unwrap_or_default();
                resume = true;
            }
            "--settings" => settings = args.next().and_then(|text| serde_json::from_str(&text).ok()),
            "--append-system-prompt" => {
                let text = args.next().unwrap_or_default();
                if let Some(logs) = &log_dir {
                    write(&logs.join("claude-instructions"), text.as_bytes())?;
                }
            }
            "--plugin-dir" => {
                let dir = PathBuf::from(args.next().unwrap_or_default());
                let mut skills: Vec<String> = fs::read_dir(dir.join("skills"))
                    .map(|entries| entries.flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect())
                    .unwrap_or_default();
                skills.sort();
                if let Some(logs) = &log_dir {
                    write(
                        &logs.join("claude-skills"),
                        skills.iter().map(|skill| format!("{skill}\n")).collect::<String>().as_bytes(),
                    )?;
                }
            }
            _ => {}
        }
    }
    let cwd = env::current_dir().map_err(|error| Failed(1, format!("no current directory: {error}")))?;
    let slug: String = cwd.to_string_lossy().chars().map(|char| if char.is_ascii_alphanumeric() { char } else { '-' }).collect();
    let config = PathBuf::from(env::var_os("CLAUDE_CONFIG_DIR").ok_or_else(|| Failed(1, "CLAUDE_CONFIG_DIR is unset".into()))?);
    let dir = config.join("projects").join(slug);
    let transcript = dir.join(format!("{session}.jsonl"));
    let run = Run { session, settings, cwd, dir, transcript };
    if resume { resumed(&run, &config) } else { first(&run) }
}

fn first(run: &Run) -> Step {
    fs::create_dir_all(&run.dir).map_err(io)?;
    run.say(&json!({"type": "ai-title", "aiTitle": "Add a farewell", "sessionId": "x"}))?;
    run.say(&json!({
        "type": "user",
        "cwd": run.cwd,
        "sessionId": run.session,
        "message": {"role": "user", "content": "Please add a farewell line to main"},
    }))?;
    run.hook("UserPromptSubmit", "toolu_1", None)?;
    run.say(&tool_use("toolu_1"))?;
    run.hook("PreToolUse", "toolu_1", None)?;
    in_tool()?;
    append(&run.cwd.join("src").join("main.rs"), "fn farewell() {}\n")?;
    run.hook("PostToolUse", "toolu_1", None)?;
    let results = run.dir.join(&run.session).join("tool-results");
    fs::create_dir_all(&results).map_err(io)?;
    let saved = results.join("t1.txt");
    write(&saved, b"big output\n")?;
    run.say(&json!({
        "type": "user",
        "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "toolu_1", "content": format!("saved to {}", saved.display())},
        ]},
    }))?;
    run.say(&json!({
        "type": "assistant",
        "message": {"role": "assistant", "content": [
            {"type": "text", "text": "Done: farewell added. I used ghp_abcdefghijklmnopqrstuvwxyz0123 to fetch it."},
        ]},
    }))?;
    run.hook("Stop", "toolu_1", None)
}

fn resumed(run: &Run, config: &Path) -> Step {
    let text = fs::read_to_string(&run.transcript).unwrap_or_default();
    let records: Vec<Value> = text.lines().filter_map(|line| serde_json::from_str(line).ok()).collect();
    let cwd = run.cwd.to_string_lossy();
    if !records.iter().any(|record| record["cwd"].as_str() == Some(&cwd)) {
        return Err(Failed(3, format!("transcript not restored for {cwd}")));
    }
    if !text.contains("Please add a farewell line") {
        return Err(Failed(3, "conversation missing".into()));
    }
    if env::var_os("FAKE_TRANSLATED").is_none() {
        let saved = records.iter().find_map(saved_to).unwrap_or_default();
        let expected = run.dir.join(&run.session).join("tool-results").join("t1.txt");
        if Path::new(&saved) != expected {
            return Err(Failed(4, format!("tool result path not rewritten: {saved}")));
        }
        if !fs::read_to_string(&saved).is_ok_and(|result| result.contains("big output")) {
            return Err(Failed(4, "tool result missing".into()));
        }
    }
    if !records.iter().any(|record| record["sessionId"].as_str() == Some(&run.session)) {
        return Err(Failed(5, "session id not rewritten".into()));
    }
    for needle in env::var("FAKE_EXPECT").unwrap_or_default().split_whitespace() {
        let needle = needle.replace('-', " ");
        if !text.contains(&needle) {
            return Err(Failed(6, format!("transcript lacks {needle}\n{text}")));
        }
    }
    write(&config.join("last-session"), run.session.as_bytes())?;
    let said = env::var("FAKE_SAY").unwrap_or_else(|_| "Continuing at home: add a home line".into());
    run.say(&json!({"type": "user", "message": {"role": "user", "content": said}}))?;
    run.hook("UserPromptSubmit", "toolu_2", Some(&config.join("last-hook-output")))?;
    run.say(&tool_use("toolu_2"))?;
    run.hook("PreToolUse", "toolu_2", None)?;
    in_tool()?;
    let line = env::var("FAKE_LINE").unwrap_or_else(|_| "fn home() {}".into());
    append(&run.cwd.join("src").join("main.rs"), &format!("{line}\n"))?;
    run.hook("PostToolUse", "toolu_2", None)?;
    run.say(&json!({
        "type": "user",
        "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "toolu_2", "content": "edited", "is_error": false},
        ]},
    }))?;
    run.hook("Stop", "toolu_2", None)
}

impl Run {
    fn say(&self, record: &Value) -> Step {
        append(&self.transcript, &format!("{record}\n"))
    }

    /// Runs the hook the settings give for `event`, as Claude Code does: a
    /// program with `args`, or a command for the shell, with the hook's
    /// input on stdin and its output where `output` says. Without settings
    /// there's no hook at all.
    fn hook(&self, event: &str, tool_use: &str, output: Option<&Path>) -> Step {
        let Some(settings) = &self.settings else {
            return Ok(());
        };
        let hook = &settings["hooks"][event][0]["hooks"][0];
        let Some(program) = hook["command"].as_str() else {
            return Err(Failed(8, format!("no {event} hook in the settings")));
        };
        let mut command = match hook["args"].as_array() {
            Some(args) => {
                let mut command = Command::new(program);
                command.args(args.iter().filter_map(Value::as_str));
                command
            }
            None => {
                let mut command = Command::new("sh");
                command.args(["-c", program]);
                command
            }
        };
        let input = json!({
            "session_id": self.session,
            "hook_event_name": event,
            "tool_name": "Edit",
            "tool_use_id": tool_use,
            "tool_input": {"file_path": self.cwd.join("src").join("main.rs")},
        });
        if let Some(output) = output {
            command.stdout(fs::File::create(output).map_err(io)?);
        }
        let mut child = command.stdin(Stdio::piped()).spawn().map_err(|error| Failed(1, format!("{program}: {error}")))?;
        child.stdin.take().expect("piped").write_all(input.to_string().as_bytes()).map_err(io)?;
        let status = child.wait().map_err(io)?;
        match status.code() {
            Some(0) => Ok(()),
            code => Err(Failed(code.and_then(|code| u8::try_from(code).ok()).unwrap_or(1), String::new())),
        }
    }
}

fn tool_use(id: &str) -> Value {
    json!({
        "type": "assistant",
        "message": {"role": "assistant", "content": [
            {"type": "tool_use", "id": id, "name": "Edit", "input": {"file_path": "src/main.rs"}},
        ]},
    })
}

/// The path a tool result says it was saved to.
fn saved_to(record: &Value) -> Option<String> {
    record["message"]["content"]
        .as_array()?
        .iter()
        .find_map(|block| block["content"].as_str()?.strip_prefix("saved to ").map(str::to_owned))
}

/// Lets a test act while the tool runs: says so, then waits to be let go.
fn in_tool() -> Step {
    let Some(flag) = env::var_os("FAKE_IN_TOOL").map(PathBuf::from) else {
        return Ok(());
    };
    write(&flag, b"")?;
    let mut go = flag.into_os_string();
    go.push(".go");
    while !Path::new(&go).exists() {
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

fn append(path: &Path, text: &str) -> Step {
    fs::OpenOptions::new().create(true).append(true).open(path).and_then(|mut file| file.write_all(text.as_bytes())).map_err(io)
}

fn write(path: &Path, bytes: &[u8]) -> Step {
    fs::write(path, bytes).map_err(io)
}

fn io(error: std::io::Error) -> Failed {
    Failed(1, error.to_string())
}
