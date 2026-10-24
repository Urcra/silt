//! A stand-in for Gemini CLI: takes `--session-id`, keeps chats under the
//! project's slug in `$GEMINI_CLI_HOME/.gemini/tmp`, and refuses to start
//! without Silt's MCP server in its settings. `--prompt-interactive` goes
//! to `$FAKE_LOG_DIR/gemini-prompt`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{Value, json};
use silt_fake_agent::{
    Failed, Step, Utc, append, cwd, env_dir, env_or, exit, expect_words, has, io, main_rs, mentions, read, records, write,
};

fn main() -> ExitCode {
    exit(run())
}

fn run() -> Step {
    let (mut session, mut resume) = (String::new(), false);
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--session-id" => session = args.next().unwrap_or_default(),
            "--resume" => {
                session = args.next().unwrap_or_default();
                resume = true;
            }
            "--prompt-interactive" => {
                write(&env_dir("FAKE_LOG_DIR")?.join("gemini-prompt"), args.next().unwrap_or_default().as_bytes())?
            }
            _ => {}
        }
    }
    let gemini = env_dir("GEMINI_CLI_HOME")?.join(".gemini");
    if !read(&gemini.join("settings.json")).contains("\"silt\"") {
        return Err(Failed(7, "no silt MCP server in settings".into()));
    }
    let cwd = cwd()?;
    let slug = slug(&cwd);
    let chats = gemini.join("tmp").join(&slug).join("chats");
    let now = Utc::now();
    if resume {
        let file = sessions(&chats)
            .into_iter()
            .find(|file| records(&read(file)).iter().any(|record| has(record, "sessionId", &session)))
            .ok_or_else(|| Failed(3, format!("no session {session} in {}", chats.display())))?;
        let text = read(&file);
        if !records(&text).iter().any(|record| mentions(record, &main_rs(&cwd).to_string_lossy())) {
            return Err(Failed(3, format!("session not restored for {}", cwd.display())));
        }
        expect_words(&text, "session")?;
        return turn(&file, now, &cwd, &env_or("FAKE_SAY", "Continuing in gemini"), "2", &env_or("FAKE_LINE", "fn gemini() {}"));
    }
    fs::create_dir_all(&chats).map_err(io)?;
    write(&gemini.join("tmp").join(&slug).join(".project_root"), cwd.to_string_lossy().as_bytes())?;
    let projects = json!({"projects": {cwd.to_string_lossy(): slug}});
    write(&gemini.join("projects.json"), projects.to_string().as_bytes())?;
    let short: String = session.chars().take(8).collect();
    let stamp = now.file_stamp();
    let file = chats.join(format!("session-{}-{short}.jsonl", &stamp[..stamp.len() - 3]));
    let time = now.rfc3339();
    say(&file, &json!({"sessionId": session, "projectHash": "x", "startTime": time, "lastUpdated": time, "kind": "main"}))?;
    say(
        &file,
        &json!({"id": "ctx", "timestamp": time, "type": "user", "content": [{"text": "<session_context>Gemini CLI</session_context>"}]}),
    )?;
    turn(&file, now, &cwd, "Please add a farewell line to main", "1", "fn farewell() {}")
}

/// The project's directory name in lower case, other runs of characters as
/// one dash, as Gemini CLI names its project directories.
fn slug(cwd: &Path) -> String {
    let name = cwd.file_name().map(|name| name.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut slug = String::new();
    for char in name.chars() {
        if char.is_ascii_lowercase() || char.is_ascii_digit() {
            slug.push(char);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug
}

fn sessions(chats: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(chats)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("session-") && name.ends_with(".jsonl"))
                })
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}

fn say(file: &Path, record: &Value) -> Step {
    append(file, &format!("{record}\n"))
}

fn turn(file: &Path, now: Utc, cwd: &Path, said: &str, n: &str, added: &str) -> Step {
    let time = now.rfc3339();
    say(file, &json!({"id": format!("u-{n}"), "timestamp": time, "type": "user", "content": [{"text": said}]}))?;
    say(file, &json!({"id": format!("g-{n}"), "timestamp": time, "type": "gemini", "content": "", "thoughts": []}))?;
    // The model thinks before it calls the tool, so the session is found
    // first; and like Gemini, the tool call is only written once the tool
    // finished.
    std::thread::sleep(std::time::Duration::from_millis(1200));
    append(&main_rs(cwd), &format!("{added}\n"))?;
    say(
        file,
        &json!({
            "id": format!("g-{n}"), "timestamp": time, "type": "gemini", "content": "", "thoughts": [],
            "toolCalls": [{
                "id": format!("edit-{n}"), "name": "replace", "args": {"file_path": main_rs(cwd)},
                "result": [{"functionResponse": {"id": format!("edit-{n}"), "name": "replace", "response": {"output": "replaced"}}}],
                "status": "success",
            }],
        }),
    )?;
    say(
        file,
        &json!({"id": format!("a-{n}"), "timestamp": time, "type": "gemini", "content": "Gemini says done.", "thoughts": []}),
    )?;
    // Written again with its tokens once the reply finished streaming.
    say(
        file,
        &json!({"id": format!("a-{n}"), "timestamp": time, "type": "gemini", "content": "Gemini says done.", "thoughts": [], "tokens": {"total": 1}}),
    )?;
    say(file, &json!({"$set": {"lastUpdated": "now"}}))
}
