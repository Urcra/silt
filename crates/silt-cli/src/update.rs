//! `silt update`: a dev build fast-forwards its clone's `main`, runs
//! `just update` there and restarts the daemon on what it installed, while
//! writing how far it got for the web UI to show (ADR 0100). Nightly and
//! stable builds only check for a newer release (ADR 0129).

use std::env;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use exn::{Exn, ResultExt};
use silt_daemon::paths;
use silt_daemon::protocol::{DaemonBuild, Request};
use silt_update::check::{self, Availability};
use silt_update::run::{self, RunState};
use silt_update::{BuildInfo, Channel, Commit, Outcome, Phase, Source, shell};

use crate::CliError;
use crate::commands::{self, fail};

/// How often a restart that waits for agents asks again.
const AGENTS_POLL: Duration = Duration::from_secs(3);
/// How long an idle daemon asked to restart itself gets before the update
/// restarts it: it looks every couple of seconds.
const HANDED_OVER: Duration = Duration::from_secs(10);
/// Tries at starting the daemon again, as a restart of it may be under way.
const START_TRIES: u32 = 3;
/// Writes of the state while output streams, at most this often.
const WRITE_EVERY: Duration = Duration::from_millis(250);

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
}

/// `silt update --check`.
pub fn check() -> Result<(), Exn<CliError>> {
    let build = BuildInfo::current();
    println!("silt {}", silt_update::VERSION);
    let availability = match build.channel {
        Channel::Dev => check::dev(&build, true),
        Channel::Nightly | Channel::Stable => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .or_raise(|| CliError("could not start a runtime".into()))?
            .block_on(silt_daemon::updates::released(&silt_daemon::updates::release_client(), &build)),
    };
    println!("{}", availability.explain());
    let listed = match &availability {
        Availability::Behind { commits, ahead, .. } | Availability::Nightly { commits, ahead, .. } => Some((commits, *ahead)),
        _ => None,
    };
    if let Some((commits, ahead)) = listed {
        for commit in commits.iter().take(10) {
            println!("  {} {}", commit.hash.short(), commit.subject);
        }
        if ahead as usize > commits.len().min(10) {
            println!("  and {} more", ahead as usize - commits.len().min(10));
        }
    }
    match &availability {
        Availability::Behind { .. } => println!("`silt update` installs it."),
        Availability::Nightly { url, .. } => println!("Download it from {url}"),
        _ => {}
    }
    Ok(())
}

/// `silt update`, and `silt update run`, which the daemon starts detached:
/// the same run, the first also printing what it does.
pub fn run(echo: bool) -> Result<(), Exn<CliError>> {
    let data = paths::data_dir();
    let build = BuildInfo::current();
    // Captured before the install replaces it: the restart runs the new
    // build found at the same path.
    let exe = env::current_exe().or_raise(|| CliError("cannot find the silt executable".into()))?;
    let before = run::read_state(&data);
    if before.as_ref().is_some_and(|before| before.in_flight() && before.pid != std::process::id() && run::runner_alive(before)) {
        return Err(fail("an update is already running; Settings › Updates shows how far it got"));
    }
    match build.channel {
        Channel::Dev => {}
        Channel::Nightly => {
            return Err(fail(format!("a nightly build updates from the nightly release's downloads, {}", check::NIGHTLY_URL)));
        }
        Channel::Stable => {
            return Err(fail("a stable build updates with its installer; `silt update --check` says whether one is out"));
        }
    }
    let (source, built) = check::updatable(&build)
        .map(|(source, built)| (source.clone(), built.clone()))
        .map_err(|reason| fail(format!("no update: {}", reason.explain())))?;
    let desktop = exe.with_file_name(if cfg!(windows) { "silt-desktop.exe" } else { "silt-desktop" }).is_file();
    let windows = matches!(source, Source::Wsl { .. }) || silt_os::in_wsl().is_some();
    let state = RunState::new(std::process::id(), now_ms(), Some(built), run::expected(desktop, windows), before.as_ref());
    let mut runner = Runner::start(&data, state, echo)?;
    if let Err(message) = runner.install(&build, &source) {
        runner.state.finish(Outcome::Failed { message: message.clone() }, now_ms());
        runner.save();
        return Err(fail(message));
    }
    // The restart is the new build's: it speaks to the daemon as the build
    // the daemon restarts on.
    runner.say("silt update restart");
    let status = Command::new(&exe)
        .args(["update", "restart"])
        .stdin(Stdio::null())
        .status()
        .or_raise(|| CliError(format!("could not run {}", exe.display())))?;
    if !status.success() {
        if let Some(mut state) = run::read_state(&data).filter(RunState::in_flight) {
            state.finish(Outcome::Failed { message: "the restart failed".to_owned() }, now_ms());
            let _ = run::write_state(&data, &state);
        }
        return Err(fail("the update installed, but the daemon's restart failed; `silt daemon stop` restarts it"));
    }
    if echo {
        println!("silt: updated");
    }
    Ok(())
}

/// `silt update restart`: restarts the daemon on the build just installed.
/// While agents the web UI started are mid-turn, the daemon is asked to
/// restart itself once none is (`RestartWhenIdle`), so only one restart
/// happens, and this waits for the new build to answer. Restart now stops
/// the daemon at once, and this starts the new one.
pub fn restart() -> Result<(), Exn<CliError>> {
    let data = paths::data_dir();
    let mut state = run::read_state(&data)
        .filter(RunState::in_flight)
        .unwrap_or_else(|| RunState::new(std::process::id(), now_ms(), None, vec![Phase::Restart], None));
    // This process carries the run on from here.
    state.pid = std::process::id();
    state.begin(Phase::Restart, now_ms());
    let _ = run::write_state(&data, &state);
    let mut asked = false;
    let mut idle_since: Option<Instant> = None;
    let mut tries = 0;
    let restarted = loop {
        let build = if commands::is_running() { commands::call::<DaemonBuild>(&Request::DaemonBuild).ok() } else { None };
        match build {
            // The new build answers, whoever restarted it.
            Some(build) if !build.replaced => break Ok(()),
            Some(build) if build.working > 0 => {
                // A daemon from before it could be asked refuses, and is
                // restarted here once idle instead.
                asked = asked || commands::restart_when_idle().is_ok();
                idle_since = None;
                let agents = u32::try_from(build.working).unwrap_or(u32::MAX);
                if state.outcome != (Outcome::Waiting { agents }) {
                    state.outcome = Outcome::Waiting { agents };
                    let _ = run::write_state(&data, &state);
                }
                thread::sleep(AGENTS_POLL);
                continue;
            }
            // Idle, and asked to restart itself: it does on its next tick.
            Some(_) if asked && idle_since.get_or_insert_with(Instant::now).elapsed() < HANDED_OVER => {
                thread::sleep(Duration::from_millis(250));
                continue;
            }
            _ => {}
        }
        if state.outcome != Outcome::Running {
            state.outcome = Outcome::Running;
            let _ = run::write_state(&data, &state);
        }
        match commands::ensure_daemon() {
            Ok(_) => break Ok(()),
            // Another restart of it may be under way, as the daemon's own.
            Err(_) if tries < START_TRIES => {
                tries += 1;
                thread::sleep(Duration::from_secs(1));
            }
            Err(error) => break Err(error),
        }
    };
    match restarted {
        Ok(()) => {
            state.finish(Outcome::Done, now_ms());
            let _ = run::write_state(&data, &state);
            Ok(())
        }
        Err(error) => {
            state.finish(Outcome::Failed { message: format!("the daemon did not start again: {error}") }, now_ms());
            let _ = run::write_state(&data, &state);
            Err(error)
        }
    }
}

/// A run under way: its state, written as it moves, and its output.
struct Runner {
    data: PathBuf,
    state: RunState,
    output: File,
    echo: bool,
    written: Instant,
}

impl Runner {
    fn start(data: &Path, state: RunState, echo: bool) -> Result<Self, Exn<CliError>> {
        silt_os::create_private_dir(&run::dir(data)).or_raise(|| CliError("cannot create the update's directory".into()))?;
        let output = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(run::output_path(data))
            .or_raise(|| CliError("cannot write the update's output".into()))?;
        let mut runner = Self { data: data.to_owned(), state, output, echo, written: Instant::now() };
        runner.save();
        Ok(runner)
    }

    fn save(&mut self) {
        if let Err(error) = run::write_state(&self.data, &self.state) {
            eprintln!("silt: could not write the update's state: {error}");
        }
        self.written = Instant::now();
    }

    /// Notes a line of output, and what it says of the run.
    fn take(&mut self, line: &str) {
        let line = run::plain(line);
        // An interactive shell without a terminal says so, which isn't news.
        if line.starts_with("bash: cannot set terminal process group") || line == "bash: no job control in this shell" {
            return;
        }
        let _ = writeln!(self.output, "{line}");
        if self.echo {
            println!("{line}");
        }
        let moved = self.state.line(&line, now_ms());
        if moved && (Phase::of_line(&line).is_some() || self.written.elapsed() >= WRITE_EVERY) {
            self.save();
        }
    }

    /// A line of the runner's own, in the output.
    fn say(&mut self, line: &str) {
        self.take(&format!("$ {line}"));
    }

    /// Fast-forwards `main` and runs `just update`: an error says what failed.
    fn install(&mut self, build: &BuildInfo, source: &Source) -> Result<(), String> {
        self.state.begin(Phase::Fetch, now_ms());
        self.save();
        let target = match check::dev(build, true) {
            Availability::Behind { target, .. } => target,
            Availability::UpToDate => return Err("already up to date with main".to_owned()),
            other => return Err(other.explain()),
        };
        self.say(&format!("git merge --ff-only {}", target.short()));
        let merged = self.stream(shell::git(source, &["merge", "--ff-only", target.as_str()]))?;
        if !merged.0.success() {
            return Err(format!("could not fast-forward main: {}", merged.1.unwrap_or_default()));
        }
        let head = shell::run_within(shell::git(source, &["rev-parse", "HEAD"]), Duration::from_secs(20))
            .ok()
            .and_then(|ran| Commit::parse(&ran.stdout));
        self.state.to = head.or(Some(target));
        self.save();

        let just = just_program();
        let mut tools = vec![just.clone()];
        if !overridden() {
            tools.extend(["cargo".to_owned(), "pnpm".to_owned()]);
        }
        self.preflight(source, &tools)?;
        self.say(&format!("{just} update"));
        let (status, last) = self.stream(shell::in_their_shell(source, &just, &["update"]))?;
        if !status.success() {
            let phase = self.state.current().map_or("the update", Phase::label);
            return Err(format!("{phase} failed: {}", last.unwrap_or_else(|| format!("`{just} update` exited with {status}"))));
        }
        self.state.end_step(now_ms());
        self.save();
        Ok(())
    }

    /// Says which tool the person's shell can't find, before a build that
    /// would fail on it part-way.
    fn preflight(&mut self, source: &Source, tools: &[String]) -> Result<(), String> {
        let script = "for tool in \"$@\"; do command -v \"$tool\" > /dev/null || { echo \"missing:$tool\"; exit 1; }; done";
        let mut args = vec!["-c", script, "sh"];
        args.extend(tools.iter().map(String::as_str));
        let ran = shell::run_within(shell::in_their_shell(source, "sh", &args), Duration::from_secs(60))
            .map_err(|error| format!("could not run your login shell: {error}"))?;
        if ran.status.success() {
            return Ok(());
        }
        let missing = ran.stdout.lines().find_map(|line| line.trim().strip_prefix("missing:")).map(str::to_owned);
        Err(match missing {
            Some(tool) => {
                format!("`{tool}` isn't on the PATH your login shell gives; install it, or add it to your shell's profile")
            }
            None => format!(
                "your login shell failed: {}",
                ran.stdout.lines().map(str::trim).rfind(|line| !line.is_empty()).unwrap_or("it exited")
            ),
        })
    }

    /// Runs `command`, taking in its output as it comes; says how it
    /// ended and what a failure names: its first error, else its last line.
    /// `just`'s own "Recipe failed" says less than the error before it.
    fn stream(&mut self, mut command: Command) -> Result<(ExitStatus, Option<String>), String> {
        command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let shown = format!("{:?}", command.get_program());
        let mut child = command.spawn().map_err(|error| format!("could not run {shown}: {error}"))?;
        let (lines, received) = mpsc::channel::<String>();
        let pipes: [Option<Box<dyn Read + Send>>; 2] = [
            child.stdout.take().map(|pipe| Box::new(pipe) as Box<dyn Read + Send>),
            child.stderr.take().map(|pipe| Box::new(pipe) as Box<dyn Read + Send>),
        ];
        for pipe in pipes.into_iter().flatten() {
            let lines = lines.clone();
            thread::spawn(move || {
                for line in BufReader::new(pipe).split(b'\n').map_while(Result::ok) {
                    if lines.send(String::from_utf8_lossy(&line).into_owned()).is_err() {
                        break;
                    }
                }
            });
        }
        drop(lines);
        let mut last = None;
        let mut first_error = None;
        let mut exited: Option<(ExitStatus, Instant)> = None;
        loop {
            match received.recv_timeout(Duration::from_millis(200)) {
                Ok(line) => {
                    let plain = run::plain(&line).trim().to_owned();
                    let error = plain.to_ascii_lowercase().starts_with("error") && !plain.starts_with("error: Recipe");
                    if error && first_error.is_none() {
                        first_error = Some(plain.clone());
                    }
                    if !plain.is_empty() {
                        last = Some(plain);
                    }
                    self.take(&line);
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if exited.is_none()
                && let Ok(Some(status)) = child.try_wait()
            {
                exited = Some((status, Instant::now()));
            }
            // A program it left running may hold the pipes open.
            if exited.is_some_and(|(_, at)| at.elapsed() > Duration::from_secs(2)) {
                break;
            }
            if self.written.elapsed() >= WRITE_EVERY {
                self.save();
            }
        }
        let status = match exited {
            Some((status, _)) => status,
            None => child.wait().map_err(|error| format!("could not wait for {shown}: {error}"))?,
        };
        let _ = self.output.flush();
        Ok((status, first_error.or(last)))
    }
}

/// `just`, or in a debug build what `SILT_UPDATE_JUST` names, which tests
/// stand in for it.
fn just_program() -> String {
    overridden_just().unwrap_or_else(|| "just".to_owned())
}

fn overridden() -> bool {
    overridden_just().is_some()
}

fn overridden_just() -> Option<String> {
    cfg!(debug_assertions).then(|| env::var("SILT_UPDATE_JUST").ok()).flatten().filter(|program| !program.is_empty())
}
