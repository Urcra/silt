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
//!
//! `FAKE_SCRIPT` picks another session for the single-machine tests:
//! `short`, one turn ending in a compaction, after `FAKE_PAUSE` seconds;
//! `tools`, two turns with tools of every kind Silt treats apart and an
//! edit by hand between them; `beside`, one turn with a subagent, whose
//! transcript is its own file, and a command left in the background that
//! reports back while the turn goes on; with `FAKE_UNFINISHED`, the
//! subagent runs on in the background and the command never reports;
//! `git`, one turn with an edit of `src/main.rs`, then a command running
//! `git merge --ff-only -q upstream`, or `git FAKE_GIT`, then an edit of
//! `lib.rs`;
//! `git-by-hand`, the same with the merge run between two turns, outside
//! any tool call, as a person would.

use std::env;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

use serde_json::{Value, json};
use silt_fake_agent::{Failed, Step, append, exit, expect_words, io, records, write};

struct Run {
    session: String,
    settings: Option<Value>,
    cwd: PathBuf,
    dir: PathBuf,
    transcript: PathBuf,
}

fn main() -> ExitCode {
    exit(run())
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
            "--settings" => {
                let text = args.next().unwrap_or_default();
                if let Some(logs) = &log_dir {
                    write(&logs.join("claude-settings"), text.as_bytes())?;
                }
                settings = serde_json::from_str(&text).ok();
            }
            "--mcp-config" => {
                let config = env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_default();
                write(&config.join("mcp-config.json"), args.next().unwrap_or_default().as_bytes())?;
            }
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
    match (resume, env::var("FAKE_SCRIPT").as_deref()) {
        (true, _) => resumed(&run, &config),
        (false, Ok("short")) => short(&run),
        (false, Ok("tools")) => tools(&run),
        (false, Ok("beside")) => beside(&run),
        (false, Ok("git")) => with_git(&run),
        (false, Ok("git-by-hand")) => with_git_by_hand(&run),
        (false, _) => first(&run),
    }
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
    run.hook("UserPromptSubmit", &run.edit("UserPromptSubmit", "toolu_1"), None)?;
    run.say(&tool_use("toolu_1"))?;
    run.hook("PreToolUse", &run.edit("PreToolUse", "toolu_1"), None)?;
    in_tool()?;
    append(&run.cwd.join("src").join("main.rs"), "fn farewell() {}\n")?;
    run.hook("PostToolUse", &run.edit("PostToolUse", "toolu_1"), None)?;
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
    run.hook("Stop", &run.edit("Stop", "toolu_1"), None)
}

fn resumed(run: &Run, config: &Path) -> Step {
    let text = fs::read_to_string(&run.transcript).unwrap_or_default();
    let records = records(&text);
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
    expect_words(&text, "transcript")?;
    write(&config.join("last-session"), run.session.as_bytes())?;
    let said = env::var("FAKE_SAY").unwrap_or_else(|_| "Continuing at home: add a home line".into());
    run.say(&json!({"type": "user", "message": {"role": "user", "content": said}}))?;
    run.hook("UserPromptSubmit", &run.edit("UserPromptSubmit", "toolu_2"), Some(&config.join("last-hook-output")))?;
    run.say(&tool_use("toolu_2"))?;
    run.hook("PreToolUse", &run.edit("PreToolUse", "toolu_2"), None)?;
    in_tool()?;
    let line = env::var("FAKE_LINE").unwrap_or_else(|_| "fn home() {}".into());
    append(&run.cwd.join("src").join("main.rs"), &format!("{line}\n"))?;
    run.hook("PostToolUse", &run.edit("PostToolUse", "toolu_2"), None)?;
    run.say(&json!({
        "type": "user",
        "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "toolu_2", "content": "edited", "is_error": false},
        ]},
    }))?;
    run.hook("Stop", &run.edit("Stop", "toolu_2"), None)
}

impl Run {
    fn say(&self, record: &Value) -> Step {
        append(&self.transcript, &format!("{record}\n"))
    }

    /// A hook's input about the edit of `src/main.rs` in `tool_use`.
    fn edit(&self, event: &str, tool_use: &str) -> Value {
        json!({
            "session_id": self.session,
            "hook_event_name": event,
            "tool_name": "Edit",
            "tool_use_id": tool_use,
            "tool_input": {"file_path": self.cwd.join("src").join("main.rs")},
        })
    }

    /// Runs the hook the settings give for the event `input` names, as
    /// Claude Code does: a program with `args`, or a command for the shell,
    /// with `input` on stdin and its output where `output` says. Without
    /// settings there's no hook at all; settings without this event's hook
    /// fail.
    fn hook(&self, event: &str, input: &Value, output: Option<&Path>) -> Step {
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

    /// A hook's input about `tool` (`""` and `null` between tools).
    fn about(&self, event: &str, tool: &str, tool_use: &str, tool_input: &Value) -> Value {
        json!({"session_id": self.session, "hook_event_name": event, "tool_name": tool, "tool_use_id": tool_use, "tool_input": tool_input})
    }

    fn reply(&self, text: &str) -> Step {
        self.say(&json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": text}]}}))
    }

    fn prompt(&self, text: &str) -> Step {
        self.say(&json!({"type": "user", "message": {"role": "user", "content": text}}))?;
        self.hook("UserPromptSubmit", &self.about("UserPromptSubmit", "", "", &Value::Null), None)
    }

    fn start(&self, id: &str, tool: &str, input: &Value) -> Step {
        self.say(&json!({
            "type": "assistant",
            "message": {"role": "assistant", "content": [{"type": "tool_use", "id": id, "name": tool, "input": input}]},
        }))?;
        self.hook("PreToolUse", &self.about("PreToolUse", tool, id, input), None)
    }

    fn finish(&self, id: &str, tool: &str, input: &Value) -> Step {
        self.hook("PostToolUse", &self.about("PostToolUse", tool, id, input), None)?;
        self.say(&json!({
            "type": "user",
            "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": id, "content": "ok", "is_error": false}]},
        }))
    }
}

/// One turn that edits `src/main.rs`, then a compaction.
fn short(run: &Run) -> Step {
    fs::create_dir_all(&run.dir).map_err(io)?;
    let hook =
        |event: &str| json!({"session_id": run.session, "hook_event_name": event, "tool_name": "Edit", "tool_use_id": "toolu_1"});
    run.say(&json!({"type": "ai-title", "aiTitle": "Add a greeting", "sessionId": "x"}))?;
    run.say(&json!({"type": "user", "message": {"role": "user", "content": "Please add a farewell line to main"}}))?;
    run.hook("UserPromptSubmit", &hook("UserPromptSubmit"), None)?;
    let pause = env::var("FAKE_PAUSE").ok().and_then(|pause| pause.parse::<f64>().ok()).unwrap_or(0.0);
    std::thread::sleep(Duration::from_secs_f64(pause));
    run.say(&json!({
        "type": "assistant",
        "message": {"role": "assistant", "content": [
            {"type": "text", "text": "Adding it now."},
            {"type": "tool_use", "id": "toolu_1", "name": "Edit", "input": {"file_path": "src/main.rs"}},
        ]},
    }))?;
    run.hook("PreToolUse", &hook("PreToolUse"), None)?;
    append(&run.cwd.join("src").join("main.rs"), "fn farewell() {}\n")?;
    run.hook("PostToolUse", &hook("PostToolUse"), None)?;
    run.say(&json!({
        "type": "user",
        "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_1", "content": "edited", "is_error": false}]},
    }))?;
    run.reply("Done: farewell added.")?;
    run.reply("Decision: farewell stays in main.rs instead of its own module, to keep the binary single-file.")?;
    run.say(&json!({"type": "system", "subtype": "compact_boundary", "content": "Conversation compacted"}))?;
    run.say(&json!({"type": "user", "isCompactSummary": true, "message": {"role": "user", "content": "Summary: farewell was added to main."}}))?;
    run.hook("Stop", &hook("Stop"), None)
}

/// Two turns with an edit, a read, a command and a write, and a hand edit
/// between them.
fn tools(run: &Run) -> Step {
    fs::create_dir_all(&run.dir).map_err(io)?;
    let main = json!({"file_path": "src/main.rs"});
    let stop = || run.hook("Stop", &run.about("Stop", "", "", &Value::Null), None);
    run.prompt("Add a farewell and a helper")?;
    run.start("toolu_1", "Edit", &main)?;
    append(&run.cwd.join("src").join("main.rs"), "fn farewell() {}\n")?;
    run.finish("toolu_1", "Edit", &main)?;
    run.start("toolu_2", "Read", &main)?;
    run.finish("toolu_2", "Read", &main)?;
    let make = json!({"command": "make helper"});
    run.start("toolu_3", "Bash", &make)?;
    write(&run.cwd.join("src").join("helper.rs"), b"pub fn help() {}\n")?;
    run.finish("toolu_3", "Bash", &make)?;
    run.reply("Added both.")?;
    stop()?;
    append(&run.cwd.join("src").join("helper.rs"), "// by hand\n")?;
    run.prompt("Now a note")?;
    let notes = json!({"file_path": run.cwd.join("NOTES.md")});
    run.start("toolu_4", "Write", &notes)?;
    write(&run.cwd.join("NOTES.md"), b"# Notes\n")?;
    run.finish("toolu_4", "Write", &notes)?;
    run.reply("Noted.")?;
    stop()
}

/// An edit of the agent's own, then a merge that git makes of `upstream`.
fn with_git(run: &Run) -> Step {
    fs::create_dir_all(&run.dir).map_err(io)?;
    let main = json!({"file_path": "src/main.rs"});
    run.prompt("Add a helper, then bring in upstream")?;
    run.start("toolu_1", "Edit", &main)?;
    append(&run.cwd.join("src").join("main.rs"), "fn mine() {}\n")?;
    run.finish("toolu_1", "Edit", &main)?;
    let merge = json!({"command": format!("git {}", git_args().join(" "))});
    run.start("toolu_2", "Bash", &merge)?;
    merge_upstream(&run.cwd)?;
    run.finish("toolu_2", "Bash", &merge)?;
    let lib = json!({"file_path": "lib.rs"});
    run.start("toolu_3", "Edit", &lib)?;
    append(&run.cwd.join("lib.rs"), "fn after() {}\n")?;
    run.finish("toolu_3", "Edit", &lib)?;
    run.reply("Added mine and merged upstream.")?;
    run.hook("Stop", &run.about("Stop", "", "", &Value::Null), None)
}

/// The same edits as `with_git` in two turns, and between them a merge that
/// isn't the agent's.
fn with_git_by_hand(run: &Run) -> Step {
    fs::create_dir_all(&run.dir).map_err(io)?;
    let stop = || run.hook("Stop", &run.about("Stop", "", "", &Value::Null), None);
    let main = json!({"file_path": "src/main.rs"});
    run.prompt("Add a helper")?;
    run.start("toolu_1", "Edit", &main)?;
    append(&run.cwd.join("src").join("main.rs"), "fn mine() {}\n")?;
    run.finish("toolu_1", "Edit", &main)?;
    run.reply("Added mine.")?;
    stop()?;
    merge_upstream(&run.cwd)?;
    run.prompt("Now one after lib")?;
    let lib = json!({"file_path": "lib.rs"});
    run.start("toolu_2", "Edit", &lib)?;
    append(&run.cwd.join("lib.rs"), "fn after() {}\n")?;
    run.finish("toolu_2", "Edit", &lib)?;
    run.reply("Added after.")?;
    stop()
}

/// `FAKE_GIT`'s words, by default a merge of `upstream`.
fn git_args() -> Vec<String> {
    env::var("FAKE_GIT").unwrap_or_else(|_| "merge --ff-only -q upstream".into()).split_whitespace().map(str::to_owned).collect()
}

fn merge_upstream(cwd: &Path) -> Step {
    let args = git_args();
    let ran = Command::new("git").args(&args).current_dir(cwd).status().map_err(io)?;
    if ran.success() { Ok(()) } else { Err(Failed(4, format!("git {args:?} failed"))) }
}

/// A subagent, its transcript in `subagents/agent-a1.jsonl`, and a command
/// left running in the background, which reports back mid-turn.
fn beside(run: &Run) -> Step {
    let unfinished = env::var_os("FAKE_UNFINISHED").is_some();
    fs::create_dir_all(&run.dir).map_err(io)?;
    run.prompt("Look into the lexer and start the dev server")?;
    let look = json!({"description": "Investigate the lexer", "subagent_type": "Explore", "prompt": "Investigate the lexer"});
    run.start("toolu_a", "Agent", &look)?;
    let subagents = run.dir.join(&run.session).join("subagents");
    fs::create_dir_all(&subagents).map_err(io)?;
    let side = subagents.join("agent-a1.jsonl");
    let said = |record: Value| append(&side, &format!("{record}\n"));
    said(
        json!({"type": "user", "isSidechain": true, "agentId": "a1", "message": {"role": "user", "content": "Investigate the lexer"}}),
    )?;
    said(json!({"type": "assistant", "isSidechain": true, "agentId": "a1", "message": {"role": "assistant", "content": [
        {"type": "text", "text": "Read lexer.rs: nothing wrong with it."},
    ]}}))?;
    write(&subagents.join("agent-a1.meta.json"), br#"{"agentType":"Explore","toolUseId":"toolu_a"}"#)?;
    run.hook("PostToolUse", &run.about("PostToolUse", "Agent", "toolu_a", &look), None)?;
    run.say(&json!({
        "type": "user",
        "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_a", "content": "The lexer is fine."}]},
        "toolUseResult": {"status": if unfinished { "async_launched" } else { "completed" }, "agentId": "a1"},
    }))?;
    let serve = json!({"command": "npm run dev", "description": "Start the dev server", "run_in_background": true});
    run.start("toolu_b", "Bash", &serve)?;
    run.hook("PostToolUse", &run.about("PostToolUse", "Bash", "toolu_b", &serve), None)?;
    run.say(&json!({
        "type": "user",
        "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "toolu_b", "content": "Command running in background with ID: bg1."},
        ]},
        "toolUseResult": {"stdout": "", "stderr": "", "interrupted": false, "backgroundTaskId": "bg1"},
    }))?;
    let report = "<task-notification>\n<task-id>bg1</task-id>\n<tool-use-id>toolu_b</tool-use-id>\n<status>failed</status>\n\
                  <summary>Background command \"Start the dev server\" failed with exit code 1</summary>\n</task-notification>";
    if !unfinished {
        run.say(&json!({"type": "attachment", "attachment": {"type": "queued_command", "prompt": report}}))?;
    }
    run.reply("The lexer is fine; the dev server failed to start.")?;
    run.hook("Stop", &run.about("Stop", "", "", &Value::Null), None)
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
