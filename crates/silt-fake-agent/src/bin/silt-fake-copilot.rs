//! A stand-in for GitHub Copilot CLI: a session directory with
//! `events.jsonl` under `$COPILOT_HOME/session-state`, and the hooks of the
//! plugin `--plugin-dir` names, run as Copilot runs them: `bash`, or on
//! Windows `powershell`.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

use serde_json::{Value, json};
use silt_fake_agent::{
    Failed, Step, Utc, append, cwd, env_dir, env_or, exit, expect_words, feed, has, io, main_rs, read, records, write,
};

struct Session {
    id: String,
    plugin: PathBuf,
    home: PathBuf,
    events: PathBuf,
    cwd: PathBuf,
    time: String,
    n: u32,
}

fn main() -> ExitCode {
    exit(run())
}

fn run() -> Step {
    let home = env_dir("COPILOT_HOME")?;
    let (mut id, mut resume, mut plugin) = (String::new(), false, PathBuf::new());
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--session-id" => id = args.next().unwrap_or_default(),
            "--resume" => {
                id = args.next().unwrap_or_default();
                resume = true;
            }
            "--plugin-dir" => plugin = PathBuf::from(args.next().unwrap_or_default()),
            "--additional-mcp-config" => write(&home.join("last-mcp"), args.next().unwrap_or_default().as_bytes())?,
            "--interactive" => write(&home.join("last-prompt"), args.next().unwrap_or_default().as_bytes())?,
            _ => {}
        }
    }
    if !plugin.join("hooks").join("hooks.json").is_file() {
        return Err(Failed(7, "no Silt plugin".into()));
    }
    let dir = home.join("session-state").join(&id);
    let cwd = cwd()?;
    let mut session =
        Session { events: dir.join("events.jsonl"), id, plugin, home, cwd: cwd.clone(), time: Utc::now().rfc3339(), n: 0 };
    if resume {
        if !session.events.is_file() {
            return Err(Failed(3, format!("no session {}", session.id)));
        }
        let text = read(&session.events);
        let here = cwd.to_string_lossy();
        if !records(&text).iter().any(|record| has(record, "cwd", &here)) {
            return Err(Failed(3, format!("events not restored for {here}")));
        }
        // Silt quotes it as YAML takes a JSON string; Copilot writes it bare.
        let workspace = read(&dir.join("workspace.yaml"));
        if !workspace.contains(&*here) && !workspace.contains(&json!(here).to_string()) {
            return Err(Failed(3, "workspace not restored".into()));
        }
        expect_words(&text, "session")?;
        session.n = 1000;
        return session.turn(&env_or("FAKE_SAY", "Continuing in copilot"), "call_2", &env_or("FAKE_LINE", "fn copilot() {}"));
    }
    fs::create_dir_all(&dir).map_err(io)?;
    write(&dir.join("workspace.yaml"), format!("id: {}\ncwd: {}\n", session.id, cwd.display()).as_bytes())?;
    session.event("session.start", &json!({"sessionId": session.id, "context": {"cwd": cwd, "gitRoot": cwd}}))?;
    session.hook("sessionStart", "", true)?;
    session.turn("Please add a farewell line to main", "call_1", "fn farewell() {}")
}

impl Session {
    fn event(&mut self, kind: &str, data: &Value) -> Step {
        self.n += 1;
        let event = json!({"type": kind, "data": data, "id": format!("{}-{}", self.id, self.n), "timestamp": self.time, "parentId": null});
        append(&self.events, &format!("{event}\n"))
    }

    /// Runs the plugin's command for `name` with its input on stdin, keeping
    /// what it prints in `$COPILOT_HOME/hook-<name>`.
    fn hook(&self, name: &str, prompt: &str, optional: bool) -> Step {
        let hooks: Value = serde_json::from_str(&read(&self.plugin.join("hooks").join("hooks.json"))).unwrap_or_default();
        let shell = if cfg!(windows) { "powershell" } else { "bash" };
        let Some(command) = hooks["hooks"][name][0][shell].as_str() else {
            return if optional { Ok(()) } else { Err(Failed(8, format!("no {name} hook in the plugin"))) };
        };
        let mut run = if cfg!(windows) {
            let mut run = Command::new("powershell");
            run.args(["-NoProfile", "-NonInteractive", "-Command", command]);
            run
        } else {
            let mut run = Command::new("sh");
            run.args(["-c", command]);
            run
        };
        let output = fs::File::create(self.home.join(format!("hook-{name}"))).map_err(io)?;
        let mut child = run.stdin(Stdio::piped()).stdout(output).spawn().map_err(io)?;
        let input = json!({"sessionId": self.id, "timestamp": self.time, "cwd": self.cwd, "prompt": prompt});
        feed(&mut child, &input.to_string())?;
        match child.wait().map_err(io)?.code() {
            Some(0) => Ok(()),
            code => Err(Failed(code.and_then(|code| u8::try_from(code).ok()).unwrap_or(1), String::new())),
        }
    }

    fn turn(&mut self, said: &str, call: &str, added: &str) -> Step {
        self.hook("userPromptSubmitted", said, false)?;
        self.event("user.message", &json!({"content": said, "transformedContent": said}))?;
        self.event("assistant.turn_start", &json!({"turnId": "0"}))?;
        let path = main_rs(&self.cwd);
        self.event(
            "assistant.message",
            &json!({"content": "", "toolRequests": [{"toolCallId": call, "name": "edit", "arguments": {"path": path}}]}),
        )?;
        self.event("assistant.turn_end", &json!({"turnId": "0"}))?;
        // Like Copilot, events reach the file a little after they happen.
        append(&path, &format!("{added}\n"))?;
        std::thread::sleep(Duration::from_millis(100));
        self.event("tool.execution_start", &json!({"toolCallId": call, "toolName": "edit"}))?;
        self.event("tool.execution_complete", &json!({"toolCallId": call, "success": true, "result": {"content": "edited"}}))?;
        self.event("assistant.turn_start", &json!({"turnId": "1"}))?;
        self.event("assistant.message", &json!({"content": "Copilot says done.", "toolRequests": []}))?;
        self.event("assistant.turn_end", &json!({"turnId": "1"}))
    }
}
