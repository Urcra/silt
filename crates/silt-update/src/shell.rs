//! Running git and `just` in a build's clone, here or in WSL.

use std::io::{self, Read};
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::info::Source;

/// Runs `"$0" "$@"` in the person's own shell, as a login and interactive
/// one, so it finds what their terminal does: `just`, cargo, and nvm's node
/// and pnpm. A daemon the desktop app started has only the session's bare
/// `PATH`. Shells that don't take `$0` and `$@` so get `sh -l`. Its
/// errors go to its output, so the two come in the order they were written.
const IN_THEIR_SHELL: &str = r#"case "${SHELL##*/}" in bash|zsh|ksh|dash|sh) shell=$SHELL ;; *) shell=/bin/sh ;; esac
exec "$shell" -lic 'exec "$0" "$@" 2>&1' "$0" "$@" 2>&1"#;

/// `git <args>` in the clone.
pub fn git(source: &Source, args: &[&str]) -> Command {
    let mut command = match source {
        Source::Local(path) => {
            let mut command = Command::new("git");
            command.arg("-C").arg(path).env("GIT_TERMINAL_PROMPT", "0");
            command
        }
        Source::Wsl { distribution, path } => {
            let mut command = Command::new("wsl.exe");
            command.args(["-d", distribution, "--cd", path, "--exec", "env", "GIT_TERMINAL_PROMPT=0", "git"]);
            command
        }
    };
    command.args(["-c", "credential.interactive=never"]).args(args);
    silt_os::own_group_std(&mut command);
    command
}

/// `program <args>` in the clone, through the person's login shell, with
/// its errors in its output.
pub fn in_their_shell(source: &Source, program: &str, args: &[&str]) -> Command {
    let mut command = match source {
        Source::Local(path) => {
            let mut command = Command::new("sh");
            command.current_dir(path).env("PATH", with_cargo_bin(std::env::var_os("PATH")));
            command
        }
        Source::Wsl { distribution, path } => {
            let mut command = Command::new("wsl.exe");
            command.args(["-d", distribution, "--cd", path, "--exec"]);
            command.arg("sh");
            command
        }
    };
    command.args(["-c", IN_THEIR_SHELL, program]).args(args);
    silt_os::own_session_std(&mut command);
    command
}

/// `PATH` with cargo's `bin` first, where `just install` puts `silt` and
/// rustup puts cargo, as a login shell's profile usually does.
fn with_cargo_bin(path: Option<std::ffi::OsString>) -> std::ffi::OsString {
    let cargo_home =
        std::env::var_os("CARGO_HOME").map(PathBuf::from).or_else(|| silt_os::home_dir().map(|home| home.join(".cargo")));
    let mut dirs: Vec<PathBuf> = cargo_home.map(|home| home.join("bin")).into_iter().collect();
    dirs.extend(path.iter().flat_map(std::env::split_paths));
    std::env::join_paths(dirs).unwrap_or_default()
}

/// What a command printed, and how it ended.
#[derive(Debug)]
pub struct Ran {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

/// Runs `command` to its end, or stops it, and what it started, after
/// `wait`: a fetch that hangs on a dead network mustn't hold a check
/// forever.
pub fn run_within(mut command: Command, wait: Duration) -> io::Result<Ran> {
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            String::from_utf8_lossy(&bytes).into_owned()
        })
    };
    let stdout = read(child.stdout.take().map(|pipe| Box::new(pipe) as Box<dyn Read + Send>));
    let stderr = read(child.stderr.take().map(|pipe| Box::new(pipe) as Box<dyn Read + Send>));
    let deadline = Instant::now() + wait;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            silt_os::kill_tree(child.id());
            let _ = child.wait();
            return Err(io::Error::new(io::ErrorKind::TimedOut, format!("gave up after {}s", wait.as_secs())));
        }
        thread::sleep(Duration::from_millis(20));
    };
    Ok(Ran { status, stdout: stdout.join().unwrap_or_default(), stderr: stderr.join().unwrap_or_default() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_bin_comes_first() {
        let path = with_cargo_bin(Some("/usr/bin".into()));
        let dirs: Vec<_> = std::env::split_paths(&path).collect();
        assert_eq!(dirs.last(), Some(&PathBuf::from("/usr/bin")));
        assert!(dirs.first().is_some_and(|dir| dir.ends_with("bin")));
    }

    /// A daemon started from a terminal keeps it, and an interactive shell
    /// in a background group of its session stops on taking it.
    #[cfg(target_os = "linux")]
    #[test]
    fn their_shell_leads_a_session_of_its_own() {
        let home = tempfile::tempdir().expect("home");
        let mut command =
            in_their_shell(&Source::Local(home.path().to_owned()), "sh", &["-c", "echo $$; cut -d' ' -f6 /proc/$$/stat"]);
        command.env("HOME", home.path()).env("SHELL", "/bin/sh");
        let ran = run_within(command, Duration::from_secs(20)).expect("ran");
        assert!(ran.status.success(), "{ran:?}");
        let lines: Vec<&str> = ran.stdout.lines().map(str::trim).filter(|line| line.parse::<u32>().is_ok()).collect();
        assert_eq!(lines.len(), 2, "{ran:?}");
        assert_eq!(lines[0], lines[1], "the shell's pid is its session's");
    }

    #[cfg(unix)]
    #[test]
    fn a_command_that_runs_too_long_is_stopped() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 5"]);
        silt_os::own_group_std(&mut command);
        let started = Instant::now();
        let error = run_within(command, Duration::from_millis(200)).expect_err("too long");
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
