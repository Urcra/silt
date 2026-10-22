//! Claude Code cloud sessions, started and steered through the person's own
//! `claude` (ADR 0040). Silt never holds their claude.ai sign-in.

use std::io::{Read, Write as _};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde::Deserialize;
use silt_types::CloudSessionId;

use crate::Launch;

/// How long `claude --cloud` may take to name the session it created.
pub const START_TIMEOUT: Duration = Duration::from_secs(60);

/// How much of the terminal's last output a failure shows.
const TAIL_LINES: usize = 12;

#[derive(Debug, thiserror::Error)]
pub enum CloudError {
    #[error("could not run claude: {0}")]
    Run(String),
    #[error("claude --cloud named no session within {}s; it last showed:\n{screen}", .waited.as_secs())]
    NoSession { waited: Duration, screen: String },
    #[error("claude --cloud exited before naming a session; it last showed:\n{0}")]
    Exited(String),
    #[error("claude could not send to the cloud session: {0}")]
    NotSent(String),
    #[error("the task is empty, or names a session rather than work to do")]
    Task,
}

/// A cloud session `claude --cloud` created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Started {
    pub id: CloudSessionId,
    /// What claude warned about while starting it, such as a branch that
    /// isn't on GitHub.
    pub warnings: Vec<String>,
}

/// Starts a cloud session for the clone at `cwd`, with `task` as its first
/// message. `claude --cloud` only runs in a terminal, so it gets a
/// pseudo-terminal, read until it names the session. Then it's stopped: the
/// session runs on in the cloud.
pub fn start(cwd: &Path, task: &str, timeout: Duration) -> Result<Started, CloudError> {
    // `--cloud` with a session's id or link attaches to it instead.
    if task.trim().is_empty() || task.trim().parse::<CloudSessionId>().is_ok() {
        return Err(CloudError::Task);
    }
    let launch = Launch::new("claude");
    let pty = native_pty_system()
        .openpty(PtySize { rows: 50, cols: 200, pixel_width: 0, pixel_height: 0 })
        .map_err(|error| CloudError::Run(error.to_string()))?;
    let mut command = CommandBuilder::new(&launch.program);
    // Joined to its flag, a task that starts with `-` isn't another flag.
    command.arg(format!("--cloud={task}"));
    command.cwd(cwd);
    command.env("TERM", "xterm-256color");
    let mut child = pty.slave.spawn_command(command).map_err(|error| CloudError::Run(error.to_string()))?;
    drop(pty.slave);
    let mut reader = pty.master.try_clone_reader().map_err(|error| CloudError::Run(error.to_string()))?;
    let (chunks, received) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buffer = [0; 4096];
        while let Ok(read @ 1..) = reader.read(&mut buffer) {
            if chunks.send(buffer[..read].to_vec()).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + timeout;
    let mut output = Vec::new();
    let outcome = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match received.recv_timeout(left) {
            Ok(chunk) => {
                output.extend_from_slice(&chunk);
                let screen = plain(&String::from_utf8_lossy(&output));
                if let Some(id) = session_in(&screen) {
                    break Ok(Started { id, warnings: warnings_in(&screen) });
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                break Err(CloudError::NoSession { waited: timeout, screen: tail(&output) });
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break Err(CloudError::Exited(tail(&output))),
        }
    };
    let _ = child.kill();
    let _ = child.wait();
    outcome
}

/// What `claude -p --cloud <id> --output-format json` answers.
#[derive(Debug, Deserialize)]
struct SendReply {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

/// Queues `message` into the cloud session. The message goes on stdin, so
/// whatever it starts with can't be taken for an option.
pub fn send(id: &CloudSessionId, message: &str) -> Result<(), CloudError> {
    let launch = Launch::new("claude");
    let mut child = Command::new(&launch.program)
        .args(["-p", "--output-format", "json", "--cloud"])
        .arg(id.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| CloudError::Run(error.to_string()))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(message.as_bytes()).map_err(|error| CloudError::Run(error.to_string()))?;
    }
    let output = child.wait_with_output().map_err(|error| CloudError::Run(error.to_string()))?;
    sent(&output.stdout, &output.stderr)
}

/// Whether the send went through, from claude's JSON answer, or its error
/// text when it gave none.
fn sent(stdout: &[u8], stderr: &[u8]) -> Result<(), CloudError> {
    match serde_json::from_slice::<SendReply>(stdout) {
        Ok(SendReply { ok: true, .. }) => Ok(()),
        Ok(SendReply { error, .. }) => Err(CloudError::NotSent(error.unwrap_or_else(|| "no reason given".into()))),
        Err(_) => {
            let said = String::from_utf8_lossy(stderr);
            let said = said.trim();
            let said = said.strip_prefix("Error: ").unwrap_or(said);
            Err(CloudError::NotSent(if said.is_empty() { "claude gave no answer".into() } else { said.to_owned() }))
        }
    }
}

/// The first cloud session a terminal shows: an id, or a claude.ai/code link.
pub fn session_in(screen: &str) -> Option<CloudSessionId> {
    screen
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .filter(|word| word.starts_with("session_") || word.starts_with("cse_"))
        .find_map(|word| word.parse().ok())
}

fn warnings_in(screen: &str) -> Vec<String> {
    screen.lines().map(str::trim).filter(|line| line.contains("is not on GitHub")).map(str::to_owned).collect()
}

/// The terminal's last lines, as text.
fn tail(output: &[u8]) -> String {
    let screen = plain(&String::from_utf8_lossy(output));
    let lines: Vec<&str> = screen.lines().map(str::trim_end).filter(|line| !line.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n")
}

/// `output` without terminal escapes: what it would show, give or take
/// where the cursor moved. A cursor move becomes a line break, so words
/// drawn apart stay apart.
pub fn plain(output: &str) -> String {
    let mut text = String::with_capacity(output.len());
    let mut chars = output.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => match chars.next() {
                Some('[') => {
                    let mut last = None;
                    for next in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&next) {
                            last = Some(next);
                            break;
                        }
                    }
                    if matches!(last, Some('H' | 'f' | 'A' | 'B' | 'E' | 'F' | 'G' | 'd')) {
                        text.push('\n');
                    }
                }
                Some(']' | 'P' | '_' | '^') => {
                    while let Some(next) = chars.next() {
                        if next == '\u{7}' || (next == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\r' => text.push('\n'),
            ch if ch.is_control() && ch != '\n' && ch != '\t' => {}
            ch => text.push(ch),
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_session_among_escapes_and_links() {
        let output = "\u{1b}[2J\u{1b}[1;1H\u{1b}[32m✔\u{1b}[0m Cloning\r\n\u{1b}]0;title\u{7}View: https://claude.ai/code/session_01ExampleCloudSession2?from=cli\r\n";
        let id = session_in(&plain(output)).expect("a session");
        assert_eq!(id.to_string(), "session_01ExampleCloudSession2");
        assert_eq!(session_in(&plain("Session ID: cse_01ABC\n")).map(|id| id.to_string()), Some("session_01ABC".to_owned()));
    }

    #[test]
    fn names_no_session_before_one_shows() {
        for screen in ["Starting a cloud session…", "the session_ starts", "see session_-p", "my_session_01"] {
            assert_eq!(session_in(&plain(screen)), None, "{screen}");
        }
    }

    #[test]
    fn cursor_moves_keep_words_apart() {
        assert_eq!(plain("a\u{1b}[5;1Hb"), "a\nb");
        assert_eq!(plain("a\u{1b}[1mb"), "ab");
        assert_eq!(plain("x\u{1b}]8;;https://e\u{1b}\\y"), "xy");
    }

    #[test]
    fn keeps_warnings_and_the_last_lines() {
        let screen = "Branch topic is not on GitHub; starting from main\nok\n";
        assert_eq!(warnings_in(screen), vec!["Branch topic is not on GitHub; starting from main".to_owned()]);
        let many: String = (0..30).map(|line| format!("line {line}\r\n")).collect();
        assert!(tail(many.as_bytes()).starts_with(&format!("line {}", 30 - TAIL_LINES)));
    }

    #[test]
    fn refuses_a_task_that_would_attach_to_a_session() {
        for task in ["", "  ", "session_01ABC", "https://claude.ai/code/cse_01ABC"] {
            assert!(matches!(start(Path::new("/"), task, Duration::ZERO), Err(CloudError::Task)), "{task:?}");
        }
    }

    #[test]
    fn reads_what_a_send_answers() {
        assert!(sent(br#"{"ok":true,"session_id":"session_01A","url":"u"}"#, b"").is_ok());
        let refused = sent(br#"{"ok":false,"session_id":"session_01A","error":"archived"}"#, b"");
        assert!(matches!(refused, Err(CloudError::NotSent(reason)) if reason == "archived"));
        let failed = sent(b"", b"Error: Session not found: session_01A\n");
        assert!(matches!(failed, Err(CloudError::NotSent(reason)) if reason == "Session not found: session_01A"));
    }
}
