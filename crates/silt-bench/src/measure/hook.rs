//! How long an agent's hook takes, for a family of 100k deltas and a
//! checkout of 20k files:
//! - `record.hook_daemonless`: the capture step in the hook's own process,
//!   with no daemon (ADR 0018), not counting starting that process;
//! - `record.hook`: `silt hook` as agents run it, forwarded to the daemon,
//!   process start included.

use std::collections::HashMap;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use exn::{Exn, ResultExt};
use silt_daemon::{NewRun, Recorder, capture_here};
use silt_log::{Author, ConvOp, Delta, Family, Op, SessionFile, SessionRef};
use silt_store::Store;
use silt_types::ActorId;

use crate::budgets::{Measured, RECORD_HOOK, RECORD_HOOK_DAEMONLESS, quantile};
use crate::daemon::Home;
use crate::fixtures::write_repo;
use crate::{BenchError, fail};

const FILES: usize = 20_000;
const DELTAS: usize = 100_000;
const RUNS: usize = 40;
const EVENTS: [&str; 4] = ["UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop"];

/// How a hook is captured.
enum Way<'a> {
    InProcess,
    Forwarded(&'a Home),
}

/// Both budgets, on the same family: in process first, then through a
/// daemon started on it.
pub async fn measure(silt: &Path, root: &Path) -> Result<Vec<(&'static str, Measured)>, Exn<BenchError>> {
    let home = Home::new(silt, root);
    let repo = root.join("repo");
    let started = Instant::now();
    write_repo(&repo, FILES).or_raise(|| BenchError("cannot write the 20k-file repository".into()))?;
    let session = uuid::Uuid::new_v4().to_string();
    let run = NewRun { cwd: &repo, agent: "claude", session_id: Some(&session), isolated: false };
    drop(Recorder::start(&home.data, run).await.or_raise(|| BenchError("cannot start a session".into()))?);
    grow_family(&home.data, DELTAS).await?;
    eprintln!("silt-bench: {FILES} files and {DELTAS} deltas set up in {:.1?}", started.elapsed());

    let daemonless = time_hooks(&home, &repo, &session, &Way::InProcess).await?;
    let daemon = home.start()?;
    let forwarded = time_hooks(&home, &repo, &session, &Way::Forwarded(&home)).await;
    daemon.stop()?;
    Ok(vec![(RECORD_HOOK_DAEMONLESS, daemonless), (RECORD_HOOK, forwarded?)])
}

/// The p95 of the slowest hook event.
async fn time_hooks(home: &Home, repo: &Path, session: &str, way: &Way<'_>) -> Result<Measured, Exn<BenchError>> {
    let edited = repo.join("src/d0/f0.rs");
    let mut times: HashMap<&str, Vec<Duration>> = HashMap::new();
    // The first run saves the family's snapshot, as the first hook after a
    // long stretch does, and lets a daemon just started settle.
    for run in 0..=RUNS {
        for event in EVENTS {
            if event == "PostToolUse" {
                fs::write(&edited, format!("pub fn f{run}() {{}}\n")).or_raise(|| BenchError("the tool can't edit".into()))?;
            }
            let input = serde_json::json!({
                "session_id": session,
                "hook_event_name": event,
                "tool_name": "Edit",
                "tool_use_id": format!("toolu_{run}"),
                "tool_input": { "file_path": edited },
            });
            let start = Instant::now();
            match way {
                Way::InProcess => {
                    capture_here(&home.data, event, input).await.or_raise(|| BenchError(format!("cannot capture {event}")))?;
                }
                Way::Forwarded(home) => run_hook(home, repo, event, &input)?,
            }
            if run > 0 {
                times.entry(event).or_default().push(start.elapsed());
            }
        }
    }

    let mut slowest = Duration::ZERO;
    let label = if matches!(way, Way::InProcess) { "in process" } else { "forwarded" };
    for event in EVENTS {
        let taken = times.remove(event).unwrap_or_default();
        let (p50, p95, max) = (quantile(taken.clone(), 0.5), quantile(taken.clone(), 0.95), quantile(taken, 1.0));
        let (Some(p50), Some(p95), Some(max)) = (p50, p95, max) else {
            return Err(fail(format!("no {event} hooks were measured")));
        };
        slowest = slowest.max(p95);
        eprintln!("silt-bench: {label:>10} {event:>16}: p50 {p50:>8.1?}  p95 {p95:>8.1?}  max {max:>8.1?}");
    }
    Ok(Measured::Time(slowest))
}

/// Runs `silt hook <event>` as Claude Code does, with the input on stdin.
fn run_hook(home: &Home, repo: &Path, event: &str, input: &serde_json::Value) -> Result<(), Exn<BenchError>> {
    let failed = || BenchError(format!("silt hook {event} failed"));
    let mut child =
        home.command(repo).args(["hook", event]).stdin(Stdio::piped()).stdout(Stdio::null()).spawn().or_raise(failed)?;
    child.stdin.take().expect("piped").write_all(input.to_string().as_bytes()).or_raise(failed)?;
    if !child.wait().or_raise(failed)?.success() {
        return Err(fail(format!("silt hook {event} exited with an error")));
    }
    Ok(())
}

/// Fills the session's family up to `total` deltas with transcript lines,
/// most of what a long session records.
async fn grow_family(data: &Path, total: usize) -> Result<(), Exn<BenchError>> {
    let failed = || BenchError("cannot grow the family".into());
    let store = Store::open(data).await.or_raise(failed)?;
    let record = store.sessions().await.or_raise(failed)?.pop().ok_or_else(|| fail("no session was recorded"))?;
    let family_id = store.threads().await.or_raise(failed)?.pop().ok_or_else(|| fail("no thread was recorded"))?.family;
    let mut family = Family::new();
    for delta in store.deltas(family_id).await.or_raise(failed)? {
        family.apply(delta, &mut HashMap::new()).or_raise(failed)?;
    }
    let session = SessionRef { agent: record.agent, id: record.session_id };
    let author = Author::Agent { agent: session.agent.clone(), session: session.id.clone(), tool_use_id: None };
    let actor = ActorId(uuid::Uuid::new_v4().as_u128());
    let mut batch: Vec<Delta> = Vec::new();
    for index in family.log().len()..total {
        let line = transcript_line(index);
        let op = Op::Conv(ConvOp::Native { session: session.clone(), file: SessionFile::Main, line });
        let delta = family.delta(actor, record.thread, author.clone(), 0, family.thread_head(record.thread), op);
        family.apply(delta.clone(), &mut HashMap::new()).or_raise(failed)?;
        batch.push(delta);
        if batch.len() == 5_000 {
            store.append_deltas(family_id, &std::mem::take(&mut batch)).await.or_raise(failed)?;
        }
    }
    store.append_deltas(family_id, &batch).await.or_raise(failed)
}

fn transcript_line(index: usize) -> String {
    let text = format!("Step {index}: reading the module, then changing the function it names and running the tests again.");
    if index.is_multiple_of(2) {
        serde_json::json!({ "type": "user", "message": { "role": "user", "content": text } }).to_string()
    } else {
        serde_json::json!({ "type": "assistant", "message": { "role": "assistant", "content": [{ "type": "text", "text": text }] } })
            .to_string()
    }
}
