//! A stand-in for Antigravity CLI: reads its files from `--gemini_dir`,
//! refuses to start without Silt's MCP server there, lists each
//! conversation in `antigravity-cli/conversation_summaries.db` as it starts
//! and logs its steps to the conversation's `transcript_full.jsonl`.
//! `--conversation` resumes only a conversation it keeps a store for, as
//! agy does; `--prompt-interactive` goes to `$FAKE_LOG_DIR/agy-prompt`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{Value, json};
use silt_fake_agent::{Failed, Step, Utc, append, cwd, env_dir, env_or, exit, expect_words, io, main_rs, read, write};

fn main() -> ExitCode {
    exit(run())
}

fn run() -> Step {
    let (mut gemini, mut resume, mut prompt) = (None, None, None);
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--gemini_dir" => gemini = args.next().map(PathBuf::from),
            "--conversation" => resume = args.next(),
            "--prompt-interactive" => prompt = args.next(),
            _ => {}
        }
    }
    let gemini = gemini.ok_or_else(|| Failed(2, "no --gemini_dir".into()))?;
    if !read(&gemini.join("config/mcp_config.json")).contains("\"silt\"") {
        return Err(Failed(7, "no silt MCP server in mcp_config.json".into()));
    }
    let app = gemini.join("antigravity-cli");
    let cwd = cwd()?;
    if let Some(id) = resume {
        if !app.join("conversations").join(format!("{id}.db")).is_file() {
            return Err(Failed(3, format!("no conversation {id} kept here")));
        }
        let transcript = transcript(&app, &id);
        expect_words(&read(&transcript), "conversation")?;
        return turn(&transcript, &cwd, &env_or("FAKE_SAY", "Continuing in agy"), &env_or("FAKE_LINE", "fn agy() {}"));
    }
    let id = uuid::Uuid::new_v4().to_string();
    // Started with a prompt in a folder it doesn't trust yet, as Silt's
    // checkouts are at first, agy lists the conversation without a workspace.
    list(&app, &id, prompt.is_none().then_some(cwd.as_path()))?;
    let transcript = transcript(&app, &id);
    match prompt {
        Some(prompt) => {
            write(&env_dir("FAKE_LOG_DIR")?.join("agy-prompt"), prompt.as_bytes())?;
            expect_words(&prompt, "prompt")?;
            turn(&transcript, &cwd, &prompt, &env_or("FAKE_LINE", "fn agy() {}"))
        }
        None => turn(&transcript, &cwd, "Please add a farewell line to main", "fn farewell() {}"),
    }
}

fn transcript(app: &Path, id: &str) -> PathBuf {
    app.join("brain").join(id).join(".system_generated/logs/transcript_full.jsonl")
}

/// Starts the conversation's store and lists it in the index, as agy does
/// before the first step.
fn list(app: &Path, id: &str, cwd: Option<&Path>) -> Step {
    fs::create_dir_all(app.join("conversations")).map_err(io)?;
    write(&app.join("conversations").join(format!("{id}.db")), b"")?;
    let failed = |error: rusqlite::Error| Failed(1, error.to_string());
    let index = rusqlite::Connection::open(app.join("conversation_summaries.db")).map_err(failed)?;
    index
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS conversation_summaries (conversation_id text PRIMARY KEY, \
             workspace_uris text NOT NULL, parent_conversation_id text NOT NULL DEFAULT '')",
        )
        .map_err(failed)?;
    let uris = cwd.map_or_else(String::new, |cwd| json!([format!("file://{}", uri_path(cwd))]).to_string());
    index
        .execute("INSERT INTO conversation_summaries (conversation_id, workspace_uris) VALUES (?1, ?2)", (id, uris))
        .map_err(failed)?;
    Ok(())
}

/// A path as a `file://` URI holds it: `/`-separated and percent-encoded,
/// with a Windows drive after a `/`.
fn uri_path(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let text = if text.starts_with('/') { text } else { format!("/{text}") };
    text.bytes()
        .map(|byte| match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' | b':' => char::from(byte).to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

fn step(transcript: &Path, value: &Value) -> Step {
    append(transcript, &format!("{value}\n"))
}

/// A prompt that edits `src/main.rs` and answers, its steps numbered on
/// from those the transcript has.
fn turn(transcript: &Path, cwd: &Path, said: &str, added: &str) -> Step {
    let first = read(transcript).lines().count();
    let time = Utc::now().rfc3339();
    let request = format!(
        "<USER_REQUEST>\n{said}\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nThe current local time is: {time}.\n</ADDITIONAL_METADATA>"
    );
    step(
        transcript,
        &json!({"step_index": first, "source": "USER_EXPLICIT", "type": "USER_INPUT", "status": "DONE", "created_at": time, "content": request}),
    )?;
    // The model thinks before it calls the tool, so the conversation is
    // found first.
    std::thread::sleep(std::time::Duration::from_millis(1200));
    let main = main_rs(cwd);
    step(
        transcript,
        &json!({"step_index": first + 1, "source": "MODEL", "type": "PLANNER_RESPONSE", "status": "DONE", "created_at": time,
            "input_tokens": 10, "output_tokens": 5,
            "tool_calls": [{"name": "edit_file", "args": {"Path": main, "Target": "", "Replacement": added}}]}),
    )?;
    append(&main, &format!("{added}\n"))?;
    step(
        transcript,
        &json!({"step_index": first + 2, "source": "MODEL", "type": "GENERIC", "status": "DONE", "created_at": time,
            "content": format!("Created At: {time}\nCompleted At: {time}\nMade 1 replacement in {}", main.display())}),
    )?;
    step(
        transcript,
        &json!({"step_index": first + 3, "source": "MODEL", "type": "PLANNER_RESPONSE", "status": "DONE", "created_at": time,
            "input_tokens": 10, "output_tokens": 5, "content": "Antigravity says done."}),
    )
}
