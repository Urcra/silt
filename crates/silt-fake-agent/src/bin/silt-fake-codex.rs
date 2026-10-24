//! A stand-in for Codex: picks its own id and writes a rollout under
//! `$CODEX_HOME/sessions`. `-c` settings go to `$CODEX_HOME/last-config`;
//! `resume <id>` finds that rollout again.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use serde_json::{Value, json};
use silt_fake_agent::{
    Failed, Step, Utc, append, cwd, env_dir, env_or, exit, expect_words, has, io, main_rs, read, records, write,
};

fn main() -> ExitCode {
    exit(run())
}

fn run() -> Step {
    let home = env_dir("CODEX_HOME")?;
    let (mut resume, mut id) = (false, String::new());
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "resume" => resume = true,
            "-c" => append(&home.join("last-config"), &format!("{}\n", args.next().unwrap_or_default()))?,
            _ => id = arg,
        }
    }
    let cwd = cwd()?;
    let now = Utc::now();
    if resume {
        let rollout = find_rollout(&home.join("sessions"), &id).ok_or_else(|| Failed(3, format!("no rollout for {id}")))?;
        let text = read(&rollout);
        if !records(&text).iter().any(|record| has(record, "cwd", &cwd.to_string_lossy())) {
            return Err(Failed(3, format!("rollout not restored for {}", cwd.display())));
        }
        expect_words(&text, "rollout")?;
        write(&home.join("last-session"), id.as_bytes())?;
        return turn(
            &rollout,
            now,
            &cwd,
            &env_or("FAKE_SAY", "Continuing in codex"),
            "call_2",
            &env_or("FAKE_LINE", "fn codex() {}"),
        );
    }
    let id = uuid::Uuid::new_v4().to_string();
    let dir = home.join("sessions").join(now.year.to_string()).join(format!("{:02}", now.month)).join(format!("{:02}", now.day));
    fs::create_dir_all(&dir).map_err(io)?;
    let rollout = dir.join(format!("rollout-{}-{id}.jsonl", now.file_stamp()));
    line(
        &rollout,
        now,
        "session_meta",
        &json!({"id": id, "timestamp": now.rfc3339(), "cwd": cwd, "originator": "codex_cli_rs", "cli_version": "0.163.0", "source": "cli"}),
    )?;
    line(
        &rollout,
        now,
        "turn_context",
        &json!({"cwd": cwd, "approval_policy": "on-request", "sandbox_policy": {"type": "workspace-write"}, "model": "gpt-5.5"}),
    )?;
    turn(&rollout, now, &cwd, "Please add a farewell line to main", "call_1", "fn farewell() {}")
}

fn line(rollout: &Path, now: Utc, kind: &str, payload: &Value) -> Step {
    append(rollout, &format!("{}\n", json!({"timestamp": now.rfc3339(), "type": kind, "payload": payload})))
}

fn turn(rollout: &Path, now: Utc, cwd: &Path, said: &str, call: &str, added: &str) -> Step {
    let arguments = json!({"path": main_rs(cwd)}).to_string();
    line(rollout, now, "event_msg", &json!({"type": "task_started", "turn_id": "t"}))?;
    line(
        rollout,
        now,
        "response_item",
        &json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": said}]}),
    )?;
    line(rollout, now, "event_msg", &json!({"type": "user_message", "message": said}))?;
    line(
        rollout,
        now,
        "response_item",
        &json!({"type": "function_call", "name": "apply_patch", "arguments": arguments, "call_id": call}),
    )?;
    append(&main_rs(cwd), &format!("{added}\n"))?;
    std::thread::sleep(Duration::from_millis(500));
    line(rollout, now, "response_item", &json!({"type": "function_call_output", "call_id": call, "output": "patched"}))?;
    line(
        rollout,
        now,
        "response_item",
        &json!({"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Codex says done."}]}),
    )?;
    line(rollout, now, "event_msg", &json!({"type": "task_complete", "turn_id": "t", "last_agent_message": "Codex says done."}))
}

/// The rollout `rollout-*-<id>.jsonl` anywhere under `dir`.
fn find_rollout(dir: &Path, id: &str) -> Option<PathBuf> {
    let suffix = format!("-{id}.jsonl");
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_rollout(&path, id) {
                return Some(found);
            }
        } else if entry.file_name().to_str().is_some_and(|name| name.starts_with("rollout-") && name.ends_with(&suffix)) {
            return Some(path);
        }
    }
    None
}
