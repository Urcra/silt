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

/// Asks Claude Code's print mode `question` from `cwd`, a directory of its
/// own so no project's instructions apply, telling `heard` the answer so far
/// each time more of it arrives.
pub fn ask_claude(question: &str, cwd: &Path, mut heard: impl FnMut(&str)) -> Result<Answer, OneShotError> {
    if question.len() > QUESTION_BYTES {
        return Err(OneShotError::TooLarge);
    }
    let launch = Launch::new("claude");
    let mut child = Command::new(&launch.program)
        .args(["-p", "--output-format", "stream-json", "--verbose", "--allowedTools", ""])
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| OneShotError::Run(format!("{}: {error}", launch.program.to_string_lossy())))?;
    // On stdin, so a question starting with a dash isn't taken for an option.
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(question.as_bytes()).map_err(|error| OneShotError::Run(error.to_string()))?;
    }
    let stdout = child.stdout.take().ok_or_else(|| OneShotError::Run("no output".into()))?;
    let mut answer = Streamed::default();
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        if answer.hear(&line) {
            heard(&answer.text);
        }
    }
    let output = child.wait_with_output().map_err(|error| OneShotError::Run(error.to_string()))?;
    if let Some(error) = answer.error {
        return Err(OneShotError::Failed(error));
    }
    if !output.status.success() && answer.text.is_empty() {
        let said = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(OneShotError::Failed(if said.is_empty() { output.status.to_string() } else { said }));
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
