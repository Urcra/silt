//! An agent CLI's own sign-in, run in a pseudo-terminal on the agent's
//! machine, and what its screen shows the person: the page to open, the
//! code to enter there, and whether it waits for a code pasted back
//! (ADR 0056).

use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

/// How much of the terminal's last output the person can unfold.
const TAIL_LINES: usize = 12;

/// A program asking where the cursor is, and the answer: the top left.
const CURSOR_ASKED: &[u8] = b"\x1b[6n";
const CURSOR_AT: &[u8] = b"\x1b[1;1R";

type Writer = Arc<Mutex<Box<dyn Write + Send>>>;

/// What a running sign-in reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginEvent {
    /// More of what it printed, as it printed it.
    Output(Vec<u8>),
    /// It ended; a zero exit is a success.
    Exited { success: bool },
}

/// A sign-in program running in its own terminal.
pub struct Login {
    // Held so the terminal stays open while the program runs.
    _master: Box<dyn MasterPty + Send>,
    writer: Writer,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

impl std::fmt::Debug for Login {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Login")
    }
}

impl Login {
    /// Starts `program` with only `env`, in `cwd`. Each event goes to
    /// `report`, from threads of its own, ending with [`LoginEvent::Exited`].
    pub fn start(
        program: &str,
        args: &[String],
        env: &[(String, String)],
        cwd: &Path,
        mut report: impl FnMut(LoginEvent) + Send + Clone + 'static,
    ) -> std::io::Result<Self> {
        // Wide, so a long link is printed on one line.
        let pty = native_pty_system()
            .openpty(PtySize { rows: 40, cols: 1000, pixel_width: 0, pixel_height: 0 })
            .map_err(std::io::Error::other)?;
        let mut command = CommandBuilder::new(program);
        command.args(args);
        command.env_clear();
        for (name, value) in env {
            command.env(name, value);
        }
        command.env("TERM", "xterm-256color");
        command.cwd(cwd);
        let mut child = pty.slave.spawn_command(command).map_err(std::io::Error::other)?;
        drop(pty.slave);
        let killer = child.clone_killer();
        let mut reader = pty.master.try_clone_reader().map_err(std::io::Error::other)?;
        let writer: Writer = Arc::new(Mutex::new(pty.master.take_writer().map_err(std::io::Error::other)?));
        let answers = writer.clone();
        let mut printed = report.clone();
        let (drained, read_all) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            let mut buffer = [0; 4096];
            // The end of the last read, should a question be split across two.
            let mut carried = Vec::new();
            while let Ok(read @ 1..) = reader.read(&mut buffer) {
                let chunk = &buffer[..read];
                carried.extend_from_slice(chunk);
                // Windows' console asks first, and shows nothing of the program until answered, as a terminal would.
                if carried.windows(CURSOR_ASKED.len()).any(|window| window == CURSOR_ASKED) {
                    carried.clear();
                    if let Ok(mut writer) = answers.lock() {
                        let _ = writer.write_all(CURSOR_AT).and_then(|()| writer.flush());
                    }
                }
                carried.drain(..carried.len().saturating_sub(CURSOR_ASKED.len() - 1));
                printed(LoginEvent::Output(chunk.to_vec()));
            }
            let _ = drained.send(());
        });
        std::thread::spawn(move || {
            let success = child.wait().is_ok_and(|status| status.success());
            // What it printed last comes before its end; a terminal that never
            // says it's done, as Windows' may while it's open, waits no longer.
            let _ = read_all.recv_timeout(std::time::Duration::from_secs(2));
            report(LoginEvent::Exited { success });
        });
        Ok(Self { _master: pty.master, writer, killer })
    }

    /// Types `line` and Enter, as the person would.
    pub fn type_line(&mut self, line: &str) -> std::io::Result<()> {
        let mut writer = self.writer.lock().map_err(|_| std::io::Error::other("the terminal's writer panicked"))?;
        writer.write_all(line.trim().as_bytes())?;
        writer.write_all(b"\r")?;
        writer.flush()
    }

    pub fn stop(&mut self) {
        let _ = self.killer.kill();
    }
}

impl Drop for Login {
    fn drop(&mut self) {
        self.stop();
    }
}

/// What a sign-in's terminal shows that the person needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoginScreen {
    /// The first web page it names, to open in any browser.
    pub link: Option<String>,
    /// A code to enter on that page, as a device flow shows one.
    pub code: Option<String>,
    /// It waits for a code pasted back, as its last line asks.
    pub asks_code: bool,
    /// Its last lines, without escapes.
    pub tail: String,
}

impl LoginScreen {
    pub fn read(output: &[u8]) -> Self {
        let screen = crate::cloud::plain(&String::from_utf8_lossy(output));
        let lines: Vec<&str> = screen.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
        let words = || lines.iter().flat_map(|line| line.split_whitespace());
        let link =
            words().find(|word| word.starts_with("https://")).map(|word| word.trim_end_matches(['.', ',', ')']).to_owned());
        let code = words()
            .filter(|word| !word.contains("://"))
            .map(|word| word.trim_matches(|ch: char| !ch.is_ascii_alphanumeric()))
            .find(|word| device_code(word))
            .map(str::to_owned);
        let asks_code = lines.last().is_some_and(|last| {
            let last = last.to_lowercase();
            last.contains("code") && (last.ends_with('>') || last.ends_with(':'))
        });
        let tail = lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n");
        Self { link, code, asks_code, tail }
    }
}

impl LoginScreen {
    /// Its last line, when that says the sign-in failed, as Claude's does
    /// for a code it didn't accept, and then waits rather than exit.
    pub fn failure(&self) -> Option<&str> {
        let last = self.tail.lines().last()?;
        let lower = last.to_lowercase();
        ["failed", "invalid", "error", "wasn't accepted", "not accepted"].iter().any(|word| lower.contains(word)).then_some(last)
    }
}

/// A code as device flows show them: two groups of four capitals or digits.
fn device_code(word: &str) -> bool {
    let groups: Vec<&str> = word.split('-').collect();
    groups.len() == 2
        && groups.iter().all(|group| group.len() == 4 && group.chars().all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit()))
        && word.chars().any(|ch| ch.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_claude_s_link_and_its_ask_for_the_code() {
        // As `claude-agent-acp --cli auth login --claudeai` 0.94.0 prints it,
        // the link shortened: a hyperlink escape around the same link, in color.
        let link = "https://claude.com/cai/oauth/authorize?code=true&client_id=x&state=y";
        let output = format!(
            "Opening browser to sign in\u{2026}\r\nIf the browser didn't open, visit: \u{1b}]8;;{link}\u{7}\u{1b}[94m{link}\u{1b}[39m\u{1b}]8;;\u{7}\r\nPaste code here if prompted > "
        );
        let screen = LoginScreen::read(output.as_bytes());
        assert_eq!(screen.link.as_deref(), Some(link));
        assert_eq!(screen.code, None);
        assert!(screen.asks_code);
    }

    #[test]
    fn reads_antigravity_s_link_and_its_ask_for_the_code() {
        // As `agy acp --login` 1.5.8 prints it, the link shortened.
        let link = "https://accounts.google.com/o/oauth2/auth?access_type=offline&client_id=x&state=y";
        let output = format!(
            "Open this link in a browser to log in with Google:\r\n\r\n  {link}\r\n\r\nThen paste the authorization code, or the full URL of the page you land on, and press Enter:\r\n"
        );
        let screen = LoginScreen::read(output.as_bytes());
        assert_eq!(screen.link.as_deref(), Some(link));
        assert_eq!(screen.code, None);
        assert!(screen.asks_code);
        let refused = format!("{output}bad\r\n\r\ncouldn't log in: token exchange failed: oauth2: \"invalid_grant\"\r\n");
        assert!(LoginScreen::read(refused.as_bytes()).failure().is_some());
    }

    #[test]
    fn reads_a_device_flow_s_page_and_code() {
        let output = "! First copy your one-time code: WDJB-MJH7\r\nOpen https://github.com/login/device in your browser.\r\nWaiting for authorization...\r\n";
        let screen = LoginScreen::read(output.as_bytes());
        assert_eq!(screen.link.as_deref(), Some("https://github.com/login/device"));
        assert_eq!(screen.code.as_deref(), Some("WDJB-MJH7"));
        assert!(!screen.asks_code, "it waits on the page, not on a code typed here");
    }

    #[test]
    fn reads_a_failure_claude_prints_and_waits_after() {
        let output = "Paste code here if prompted > bogus\r\nLogin failed: That code wasn't accepted. Sign in again to get a new code.\r\n";
        let screen = LoginScreen::read(output.as_bytes());
        assert_eq!(screen.failure(), Some("Login failed: That code wasn't accepted. Sign in again to get a new code."));
        assert_eq!(LoginScreen::read(b"Paste code here if prompted > ").failure(), None);
    }

    #[test]
    fn a_word_in_a_link_or_of_lowercase_is_no_code() {
        let screen = LoginScreen::read(b"see https://example.com/ABCD-1234 or abcd-efgh, 2024-1105");
        assert_eq!(screen.code, None);
    }

    #[cfg(unix)]
    #[test]
    fn runs_a_program_in_a_terminal_and_types_into_it() {
        let (sender, received) = std::sync::mpsc::channel();
        let env = vec![("PATH".to_owned(), std::env::var("PATH").unwrap_or_default())];
        let script = "printf 'code? > '; read code; [ \"$code\" = good ] && echo signed in";
        let mut login =
            Login::start("sh", &["-c".into(), script.into()], &env, Path::new("."), move |event| drop(sender.send(event)))
                .expect("starts");
        let mut output = Vec::new();
        let success = loop {
            match received.recv_timeout(std::time::Duration::from_secs(10)).expect("it reports") {
                LoginEvent::Output(chunk) => {
                    output.extend(chunk);
                    if LoginScreen::read(&output).asks_code && !output.ends_with(b"\n") {
                        login.type_line("good").expect("types");
                        output.push(b'\n');
                    }
                }
                LoginEvent::Exited { success } => break success,
            }
        };
        assert!(success);
    }

    #[cfg(unix)]
    #[test]
    fn a_program_that_asks_where_the_cursor_is_is_answered() {
        let (sender, received) = std::sync::mpsc::channel();
        let env = vec![("PATH".to_owned(), std::env::var("PATH").unwrap_or_default())];
        // Raw, so the answer, which ends in no newline, reaches the program as it comes.
        let script = "stty raw -echo; printf '\\033[6n'; dd bs=1 count=6 2>/dev/null | od -c | head -1; stty sane";
        let _login =
            Login::start("sh", &["-c".into(), script.into()], &env, Path::new("."), move |event| drop(sender.send(event)))
                .expect("starts");
        let mut output = Vec::new();
        loop {
            match received.recv_timeout(std::time::Duration::from_secs(10)).expect("it reports") {
                LoginEvent::Output(chunk) => output.extend(chunk),
                LoginEvent::Exited { success } => {
                    assert!(success);
                    break;
                }
            }
        }
        let output = String::from_utf8_lossy(&output);
        assert!(output.contains("033   [   1   ;   1   R"), "{output}");
    }
}
