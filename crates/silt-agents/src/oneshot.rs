//! One question to an agent CLI's print mode, for a page's `sample`
//! capability (ADR 0072): no tools, no session, the answer as it streams.

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::Value;

use crate::Launch;

/// The most a page's question may be, in bytes.
pub const QUESTION_BYTES: usize = 256 * 1024;
/// The most of an answer kept, in bytes; the rest is cut.
pub const ANSWER_BYTES: usize = 256 * 1024;
/// How much of a run's stderr is kept, for saying why it failed.
const STDERR_KEPT: usize = 16 * 1024;

/// What print mode is started with: no built-in tools, no MCP servers but
/// the asker's, none of the person's own settings or hooks, and nothing run
/// without being allowed, so a page's question can't reach what the
/// person's Claude Code otherwise could (ADR 0072).
pub fn print_args(tools: Option<&OfferedTools>, model: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = ["-p", "--output-format", "stream-json", "--verbose", "--tools", "", "--strict-mcp-config"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    args.extend(["--setting-sources".into(), "project".into(), "--permission-mode".into(), "dontAsk".into()]);
    match tools {
        Some(tools) => {
            args.extend(["--mcp-config".into(), tools.mcp_config.clone(), "--allowedTools".into(), tools.allowed.join(",")])
        }
        None => args.extend(["--mcp-config".into(), r#"{"mcpServers":{}}"#.into(), "--allowedTools".into(), String::new()]),
    }
    if let Some(model) = model {
        args.extend(["--model".into(), model.to_owned()]);
    }
    args
}

/// How long one question may take, under the hosted app's two minutes.
const RUN_LIMIT: std::time::Duration = std::time::Duration::from_secs(110);

#[derive(Debug, thiserror::Error)]
pub enum OneShotError {
    #[error("the question is larger than {QUESTION_BYTES} bytes")]
    TooLarge,
    #[error("couldn't run {0}")]
    Run(String),
    #[error("the agent answered with an error: {0}")]
    Failed(String),
}

/// An answer, whole, and whether it was cut at [`ANSWER_BYTES`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub text: String,
    pub truncated: bool,
}

/// An image shown with a question: its media type and its base64.
pub struct Image<'a> {
    pub media_type: &'a str,
    pub data: &'a str,
}

/// Tools the asker offers, through an MCP server of its own: the config
/// that starts it, and the tools' names as the agent calls them.
pub struct OfferedTools {
    pub mcp_config: String,
    pub allowed: Vec<String>,
}

/// Asks Claude Code's print mode `question` from `cwd`, a directory of its
/// own so no project's instructions apply, showing it `images`, offering it
/// `tools` and no others, on `model` when given, and telling `heard` the answer so far each
/// time more of it arrives.
pub fn ask_claude(
    question: &str,
    images: &[Image<'_>],
    tools: Option<&OfferedTools>,
    model: Option<&str>,
    cwd: &Path,
    mut heard: impl FnMut(&str),
) -> Result<Answer, OneShotError> {
    if question.len() > QUESTION_BYTES {
        return Err(OneShotError::TooLarge);
    }
    let launch = Launch::new("claude");
    // On Windows an npm install is `claude.cmd`, which a bare name misses.
    let path = std::env::var_os("PATH").unwrap_or_default();
    let program =
        silt_os::find_program(&launch.program.to_string_lossy(), &path).unwrap_or_else(|| launch.program.clone().into());
    let mut command = Command::new(&program);
    command.args(print_args(tools, model));
    // Images go in a message of the stream, as plain text can't carry them.
    let message = (!images.is_empty()).then(|| {
        let mut content: Vec<Value> = images
            .iter()
            .map(|image| {
                serde_json::json!({"type": "image", "source": {"type": "base64", "media_type": image.media_type, "data": image.data}})
            })
            .collect();
        content.push(serde_json::json!({"type": "text", "text": question}));
        command.args(["--input-format", "stream-json"]);
        serde_json::json!({"type": "user", "message": {"role": "user", "content": content}}).to_string() + "\n"
    });
    silt_os::own_group_std(&mut command);
    let mut child = command
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| OneShotError::Run(format!("{}: {error}", program.to_string_lossy())))?;
    // A run that hangs is ended, with what it started, so a page's question
    // doesn't hold a thread and the person's wait forever. The clock starts
    // before anything is written, as a run that never reads would block it.
    let pid = child.id();
    let (finished, finishing) = std::sync::mpsc::channel::<()>();
    let watchdog = std::thread::spawn(move || {
        let late = finishing.recv_timeout(RUN_LIMIT).is_err();
        if late {
            silt_os::kill_tree(pid);
        }
        late
    });
    // On stdin, so a question starting with a dash isn't taken for an option;
    // from a thread of its own, so a run that doesn't read can't hold this one.
    let writing = child.stdin.take().map(|mut stdin| {
        let said = message.unwrap_or_else(|| question.to_owned());
        std::thread::spawn(move || {
            let _ = stdin.write_all(said.as_bytes());
        })
    });
    // stderr is read as it comes, keeping its last part, so a chatty run
    // can't fill the pipe and stall.
    let reading = child.stderr.take().map(|mut stderr| {
        std::thread::spawn(move || {
            let mut kept = Vec::new();
            let mut chunk = [0u8; 4096];
            while let Ok(read) = std::io::Read::read(&mut stderr, &mut chunk) {
                if read == 0 {
                    break;
                }
                kept.extend_from_slice(&chunk[..read]);
                if kept.len() > STDERR_KEPT {
                    kept.drain(..kept.len() - STDERR_KEPT);
                }
            }
            String::from_utf8_lossy(&kept).trim().to_owned()
        })
    });
    let stdout = child.stdout.take().ok_or_else(|| OneShotError::Run("no output".into()))?;
    let mut answer = Streamed::default();
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        if answer.hear(&line) {
            heard(&answer.text);
        }
    }
    let status = child.wait().map_err(|error| OneShotError::Run(error.to_string()))?;
    let _ = finished.send(());
    let late = watchdog.join().unwrap_or(false);
    if let Some(writing) = writing {
        let _ = writing.join();
    }
    let said = reading.and_then(|reading| reading.join().ok()).unwrap_or_default();
    if late {
        return Err(OneShotError::Failed(format!("the agent took longer than {} seconds", RUN_LIMIT.as_secs())));
    }
    if let Some(error) = answer.error {
        return Err(OneShotError::Failed(error));
    }
    if !status.success() && answer.text.is_empty() {
        return Err(OneShotError::Failed(if said.is_empty() { status.to_string() } else { said }));
    }
    Ok(Answer { text: answer.text, truncated: answer.truncated })
}

/// What print mode's stream said so far.
#[derive(Debug, Default)]
struct Streamed {
    text: String,
    truncated: bool,
    error: Option<String>,
}

impl Streamed {
    /// Takes in one line of the stream; true when the answer grew.
    fn hear(&mut self, line: &str) -> bool {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            return false;
        };
        match record.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                let content = record.pointer("/message/content").and_then(Value::as_array);
                let said: String = content
                    .into_iter()
                    .flatten()
                    .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect();
                self.add(&said)
            }
            Some("result") => {
                if record.get("is_error").and_then(Value::as_bool) == Some(true) {
                    self.error = Some(record.get("result").and_then(Value::as_str).unwrap_or("no reason given").to_owned());
                    return false;
                }
                // The result is the whole answer; it stands where the chunks
                // didn't add up to it.
                match record.get("result").and_then(Value::as_str) {
                    Some(whole) if whole != self.text && !self.truncated => {
                        self.text.clear();
                        self.add(whole)
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    fn add(&mut self, said: &str) -> bool {
        if said.is_empty() || self.truncated {
            return false;
        }
        let room = ANSWER_BYTES - self.text.len();
        if said.len() > room {
            let mut cut = room;
            while !said.is_char_boundary(cut) {
                cut -= 1;
            }
            self.text.push_str(&said[..cut]);
            self.truncated = true;
        } else {
            self.text.push_str(said);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stream_adds_up_its_chunks_and_the_result_stands_where_they_dont() {
        let mut heard = Streamed::default();
        assert!(!heard.hear(r#"{"type":"system","subtype":"init"}"#));
        assert!(heard.hear(r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello"}]}}"#));
        assert!(
            heard.hear(r#"{"type":"assistant","message":{"content":[{"type":"tool_use"},{"type":"text","text":" there"}]}}"#)
        );
        assert!(!heard.hear(r#"{"type":"result","is_error":false,"result":"Hello there"}"#), "already whole");
        assert_eq!(heard.text, "Hello there");
        assert!(heard.hear(r#"{"type":"result","is_error":false,"result":"Different"}"#));
        assert_eq!(heard.text, "Different");
        heard.hear(r#"{"type":"result","is_error":true,"result":"overloaded"}"#);
        assert_eq!(heard.error.as_deref(), Some("overloaded"));
        assert!(!heard.hear("not json"));
    }

    #[test]
    fn a_long_answer_is_cut_on_a_character() {
        let mut heard = Streamed::default();
        heard.add(&"é".repeat(ANSWER_BYTES));
        assert!(heard.truncated);
        assert!(heard.text.len() <= ANSWER_BYTES && heard.text.is_char_boundary(heard.text.len()));
    }
}
