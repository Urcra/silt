//! The budgets a real daemon is measured on, one data dir built up in turn:
//! importing the big transcript (`record.import`, then `store.size`), then
//! importing it as a session that ran before Silt into a data dir of its
//! own (`import.session`), then a family of ten forks and a 50,000-file
//! checkout, then starting the daemon on all of it (`daemon.startup`) and
//! leaving it idle (`daemon.idle`).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use exn::{Exn, ResultExt};
use serde_json::Value;
use silt_agents::claude;
use silt_daemon::protocol::{MrSummary, Request, Resumed};
use silt_store::Store;

use crate::budgets::{DAEMON_IDLE, DAEMON_READY, DAEMON_STARTUP, IMPORT_SESSION, Measured, RECORD_IMPORT, STORE_SIZE, median};
use crate::daemon::{Daemon, Home};
use crate::fixtures::{Fixtures, SESSION_ID, clone_repo, small_transcript, write_repo};
use crate::{BenchError, fail};

const FORKS: usize = 10;
const IMPORTED_ID: &str = "5e1f0000-0000-4000-8000-0000000000aa";
const STARTS: usize = 3;
const IMPORT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const SETTLE_TIMEOUT: Duration = Duration::from_secs(60);
const IDLE_WINDOW: Duration = Duration::from_secs(10);

/// Measures the budgets `wanted` asks for, and those set up on the way.
pub async fn measure(
    silt: &Path,
    root: &Path,
    fixtures: &Fixtures,
    wanted: &[&str],
) -> Result<Vec<(&'static str, Measured)>, Exn<BenchError>> {
    let home = Home::new(silt, root);
    let transcript = fixtures.transcript().or_raise(|| BenchError("cannot write the transcript".into()))?;
    let bytes = fs::metadata(&transcript).or_raise(|| BenchError("cannot read the transcript".into()))?.len();
    let mut measured = Vec::new();

    let repo = repo(root, "small", 10)?;
    place(&transcript, &session_transcript(&home, &repo, SESSION_ID))?;
    let daemon = home.start()?;
    let start = Instant::now();
    home.silt(&repo, &["track"])?;
    recorded(&home, SESSION_ID, bytes).await?;
    measured.push((RECORD_IMPORT, Measured::Time(start.elapsed())));
    daemon.stop()?;
    #[expect(clippy::cast_precision_loss, reason = "a ratio of sizes")]
    let ratio = stored_bytes(&home.data)? as f64 / bytes as f64;
    measured.push((STORE_SIZE, Measured::Ratio(ratio)));

    // The same transcript as a session that ran before Silt, in a data dir
    // of its own, so the daemon's later budgets hold what they always did.
    let before = Home::new(silt, &root.join("before-silt"));
    let earlier = self::repo(root, "earlier", 10)?;
    place(&transcript, &session_transcript(&before, &earlier, IMPORTED_ID))?;
    let daemon = before.start()?;
    let start = Instant::now();
    before.silt(&earlier, &["import", "--agent", "claude"])?;
    measured.push((IMPORT_SESSION, Measured::Time(start.elapsed())));
    daemon.stop()?;
    if !wanted.iter().any(|name| [DAEMON_STARTUP, DAEMON_READY, DAEMON_IDLE].contains(name)) {
        return Ok(measured);
    }

    let daemon = home.start()?;
    fork_family(&home, root).await?;
    big_checkout(&home, root, fixtures).await?;
    daemon.stop()?;

    let (mut answered, mut listed) = (Vec::new(), Vec::new());
    let mut last = None;
    for _ in 0..STARTS {
        if let Some(daemon) = last.take() {
            Daemon::stop(daemon)?;
        }
        let start = Instant::now();
        let mut daemon = home.spawn()?;
        daemon.ready()?;
        answered.push(start.elapsed());
        home.request(&Request::Threads)?;
        listed.push(start.elapsed());
        last = Some(daemon);
    }
    measured.push((DAEMON_STARTUP, Measured::Time(median(answered).expect("measured"))));
    measured.push((DAEMON_READY, Measured::Time(median(listed).expect("measured"))));
    let daemon = last.expect("started");
    measured.push((DAEMON_IDLE, Measured::CoreShare(idle(&daemon)?)));
    daemon.stop()?;
    Ok(measured)
}

fn repo(root: &Path, name: &str, files: usize) -> Result<PathBuf, Exn<BenchError>> {
    let repo = root.join(name);
    write_repo(&repo, files).or_raise(|| BenchError(format!("cannot write the {name} repository")))?;
    silt_os::canonicalize(&repo).or_raise(|| BenchError("cannot resolve the repository".into()))
}

pub(super) fn session_transcript(home: &Home, cwd: &Path, session: &str) -> PathBuf {
    claude::session_paths(&home.claude, cwd, session).transcript
}

/// Puts the cached transcript where Claude Code would have written it,
/// linked rather than copied when it can be.
pub(super) fn place(transcript: &Path, at: &Path) -> Result<(), Exn<BenchError>> {
    let failed = || BenchError(format!("cannot place the transcript at {}", at.display()));
    fs::create_dir_all(at.parent().expect("in a project dir")).or_raise(failed)?;
    if fs::hard_link(transcript, at).is_err() {
        fs::copy(transcript, at).or_raise(failed)?;
    }
    Ok(())
}

/// Waits until the daemon has recorded the session's transcript up to
/// `bytes`.
pub(super) async fn recorded(home: &Home, session: &str, bytes: u64) -> Result<(), Exn<BenchError>> {
    let store = Store::open(&home.data).await.or_raise(|| BenchError("cannot open the store".into()))?;
    let deadline = Instant::now() + IMPORT_TIMEOUT;
    while Instant::now() < deadline {
        let offsets =
            store.session_offsets(claude::AGENT, session).await.or_raise(|| BenchError("cannot read offsets".into()))?;
        if offsets.iter().any(|offset| offset.file == "main" && offset.offset >= bytes) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    Err(fail(format!("session {session} wasn't recorded within {IMPORT_TIMEOUT:?}")))
}

/// What the data dir holds on disk: the database, its WAL and the blobs.
fn stored_bytes(dir: &Path) -> Result<u64, Exn<BenchError>> {
    let failed = || BenchError(format!("cannot measure {}", dir.display()));
    let mut total = 0;
    for entry in fs::read_dir(dir).or_raise(failed)? {
        let entry = entry.or_raise(failed)?;
        let kind = entry.file_type().or_raise(failed)?;
        if kind.is_dir() {
            total += stored_bytes(&entry.path())?;
        } else if kind.is_file() && entry.path().extension().is_none_or(|extension| extension != "log") {
            total += entry.metadata().or_raise(failed)?.len();
        }
    }
    Ok(total)
}

/// A thread with ten forks, each with a change of its own: every other
/// one merged back through an approved merge request, the rest pulling
/// the merged work in.
async fn fork_family(home: &Home, root: &Path) -> Result<(), Exn<BenchError>> {
    const SESSION: &str = "5e1f0000-0000-4000-8000-000000000002";
    let repo = repo(root, "forks", 20)?;
    let transcript = session_transcript(home, &repo, SESSION);
    small_transcript(&transcript, &repo, 20).or_raise(|| BenchError("cannot write the forks' transcript".into()))?;
    let length = fs::metadata(&transcript).or_raise(|| BenchError("cannot read the transcript".into()))?.len();
    let tracked = home.silt(&repo, &["track"])?;
    recorded(home, SESSION, length).await?;
    let thread = tracked.split_whitespace().last().ok_or_else(|| fail(format!("no thread in {tracked:?}")))?.to_owned();

    let mut pulling = Vec::new();
    for index in 0..FORKS {
        let fork =
            Request::Fork { thread: thread.clone(), cwd: repo.clone(), here: false, agent: None, private: false, turn: None };
        let forked: Resumed = decode(home.request(&fork)?)?;
        let file = format!("fork{index}.rs");
        fs::write(forked.checkout.join("src").join(&file), format!("pub fn fork{index}() {{}}\n"))
            .or_raise(|| BenchError("cannot change a fork".into()))?;
        changed(home, &forked.thread, &file).await?;
        if index % 2 == 0 {
            let opened = Request::MrCreate {
                thread: Some(forked.thread.clone()),
                cwd: forked.checkout.clone(),
                title: Some(format!("Fork {index}")),
                summary: Some(format!("Adds fork{index}")),
            };
            let request: MrSummary = decode(home.request(&opened)?)?;
            home.request(&Request::MrApprove { mr: request.id })?;
        } else {
            pulling.push(forked);
        }
    }
    for forked in pulling {
        home.request(&Request::Pull { thread: Some(forked.thread), cwd: forked.checkout })?;
    }
    Ok(())
}

/// Waits until the daemon has recorded `file` in the thread.
async fn changed(home: &Home, thread: &str, file: &str) -> Result<(), Exn<BenchError>> {
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        let diff = home.request(&Request::Diff {
            thread: thread.to_owned(),
            turn: None,
            to_turn: None,
            base: None,
            files: None,
            path: None,
            hide_git: false,
        })?;
        if diff.as_str().is_some_and(|diff| diff.contains(file)) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(fail(format!("{file} was never recorded in thread {thread}")))
}

/// A thread on a clone of the 50,000-file repository.
async fn big_checkout(home: &Home, root: &Path, fixtures: &Fixtures) -> Result<(), Exn<BenchError>> {
    const SESSION: &str = "5e1f0000-0000-4000-8000-000000000003";
    let repo = fixtures.big_repo().or_raise(|| BenchError("cannot write the 50k-file repository".into()))?;
    let checkout = root.join("big");
    clone_repo(&repo, &checkout).or_raise(|| BenchError("cannot clone the 50k-file repository".into()))?;
    let checkout = silt_os::canonicalize(&checkout).or_raise(|| BenchError("cannot resolve the checkout".into()))?;
    let transcript = session_transcript(home, &checkout, SESSION);
    small_transcript(&transcript, &checkout, 5).or_raise(|| BenchError("cannot write the transcript".into()))?;
    let length = fs::metadata(&transcript).or_raise(|| BenchError("cannot read the transcript".into()))?.len();
    home.silt(&checkout, &["track"])?;
    recorded(home, SESSION, length).await
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, Exn<BenchError>> {
    serde_json::from_value(value).or_raise(|| BenchError("the daemon answered something unexpected".into()))
}

/// The share of a core the daemon uses once it has settled after starting.
fn idle(daemon: &Daemon) -> Result<f64, Exn<BenchError>> {
    let deadline = Instant::now() + SETTLE_TIMEOUT;
    let mut quiet = 0;
    while quiet < 2 && Instant::now() < deadline {
        quiet = if cpu_share(daemon.pid(), Duration::from_secs(1))? < 0.05 { quiet + 1 } else { 0 };
    }
    cpu_share(daemon.pid(), IDLE_WINDOW)
}

fn cpu_share(pid: u32, over: Duration) -> Result<f64, Exn<BenchError>> {
    let before = cpu_ns(pid)?;
    std::thread::sleep(over);
    let used = cpu_ns(pid)?.saturating_sub(before);
    #[expect(clippy::cast_precision_loss, reason = "nanoseconds over a few seconds")]
    let seconds = used as f64 / 1e9;
    Ok(seconds / over.as_secs_f64())
}

/// CPU time the process's threads have run, in nanoseconds. Clock ticks of
/// `/proc/<pid>/stat` are 10 ms, a tenth of the whole budget over its
/// window, so each thread's `schedstat` is summed instead.
fn cpu_ns(pid: u32) -> Result<u64, Exn<BenchError>> {
    let failed = || BenchError("cannot read the daemon's CPU time".into());
    let mut total = 0;
    for task in fs::read_dir(format!("/proc/{pid}/task")).or_raise(failed)? {
        // A thread that ended meanwhile has nothing to add.
        let Ok(schedstat) = fs::read_to_string(task.or_raise(failed)?.path().join("schedstat")) else {
            continue;
        };
        let ran = schedstat.split_whitespace().next().ok_or_else(|| fail("the daemon's schedstat is unreadable"))?;
        total += ran.parse::<u64>().or_raise(|| BenchError("the daemon's schedstat is unreadable".into()))?;
    }
    Ok(total)
}
