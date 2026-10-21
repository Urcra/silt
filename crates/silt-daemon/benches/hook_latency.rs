//! How long an agent's hook takes to capture in its own process, without
//! the daemon (ADR 0018), against its budget: p95 ≤ 200 ms for a family of
//! 100k deltas and a checkout of 20k files. It measures the capture step,
//! not starting the `silt hook` process around it.
//!
//! `cargo bench -p silt-daemon --bench hook_latency`; it fails when a hook
//! is over budget.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use silt_daemon::{NewRun, Recorder, capture_here};
use silt_log::{Author, ConvOp, Delta, Family, Op, SessionFile, SessionRef};
use silt_store::Store;
use silt_types::ActorId;

const FILES: usize = 20_000;
const DELTAS: usize = 100_000;
const RUNS: usize = 40;
const BUDGET: Duration = Duration::from_millis(200);
const EVENTS: [&str; 4] = ["UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop"];

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
    let over = runtime.block_on(measure());
    if over {
        std::process::exit(1);
    }
}

/// Measures every hook event; whether any is over budget.
async fn measure() -> bool {
    let dir = tempfile::tempdir().expect("temp dir");
    let (data, repo) = (dir.path().join("silt"), dir.path().join("repo"));
    let started = Instant::now();
    clone_of(&repo);
    let session = uuid::Uuid::new_v4().to_string();
    let run = NewRun { cwd: &repo, agent: "claude", session_id: Some(&session), isolated: false };
    drop(Recorder::start(&data, run).await.expect("starts a session"));
    grow_family(&data, DELTAS).await;
    eprintln!("set up {FILES} files and {DELTAS} deltas in {:.1?}", started.elapsed());

    let edited = repo.join("src/d0/f0.rs");
    let mut times: HashMap<&str, Vec<Duration>> = HashMap::new();
    // The first run saves the family's snapshot, as the first hook after a
    // long stretch does.
    for run in 0..=RUNS {
        for event in EVENTS {
            if event == "PostToolUse" {
                fs::write(&edited, format!("pub fn f{run}() {{}}\n")).expect("the tool edits");
            }
            let input = serde_json::json!({
                "session_id": session,
                "hook_event_name": event,
                "tool_name": "Edit",
                "tool_use_id": format!("toolu_{run}"),
                "tool_input": { "file_path": edited },
            });
            let start = Instant::now();
            capture_here(&data, event, input).await.expect("captures");
            if run > 0 {
                times.entry(event).or_default().push(start.elapsed());
            }
        }
    }

    let mut over = false;
    for event in EVENTS {
        let mut taken = times.remove(event).expect("measured");
        taken.sort();
        let at = |quantile: f64| taken[((taken.len() - 1) as f64 * quantile).round() as usize];
        let p95 = at(0.95);
        over |= p95 > BUDGET;
        println!("{event:>16}: p50 {:>8.1?}  p95 {:>8.1?}  max {:>8.1?}", at(0.5), p95, taken[taken.len() - 1]);
    }
    println!("budget: p95 ≤ {BUDGET:?}: {}", if over { "over" } else { "within" });
    over
}

/// A clone with one commit of `FILES` small files.
fn clone_of(repo: &Path) {
    for index in 0..FILES {
        let dir = repo.join(format!("src/d{}", index / 100));
        fs::create_dir_all(&dir).expect("mkdir");
        fs::write(dir.join(format!("f{index}.rs")), format!("pub fn f{index}() {{}}\n")).expect("write");
    }
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["add", "."],
        &["-c", "user.name=b", "-c", "user.email=b@example.com", "commit", "-q", "-m", "files"],
    ] {
        assert!(Command::new("git").args(args).current_dir(repo).status().expect("git").success());
    }
}

/// Fills the session's family up to `total` deltas with transcript lines,
/// most of what a long session records.
async fn grow_family(data: &Path, total: usize) {
    let store = Store::open(data).await.expect("store");
    let record = store.sessions().await.expect("sessions").pop().expect("the session");
    let family_id = store.threads().await.expect("threads").pop().expect("the thread").family;
    let mut family = Family::new();
    for delta in store.deltas(family_id).await.expect("deltas") {
        family.apply(delta, &mut HashMap::new()).expect("replays");
    }
    let session = SessionRef { agent: record.agent, id: record.session_id };
    let author = Author::Agent { agent: session.agent.clone(), session: session.id.clone(), tool_use_id: None };
    let actor = ActorId(uuid::Uuid::new_v4().as_u128());
    let mut batch: Vec<Delta> = Vec::new();
    for index in family.log().len()..total {
        let line = transcript_line(index);
        let op = Op::Conv(ConvOp::Native { session: session.clone(), file: SessionFile::Main, line });
        let delta = family.delta(actor, record.thread, author.clone(), 0, family.thread_head(record.thread), op);
        family.apply(delta.clone(), &mut HashMap::new()).expect("applies");
        batch.push(delta);
        if batch.len() == 5_000 {
            store.append_deltas(family_id, &std::mem::take(&mut batch)).await.expect("stores");
        }
    }
    store.append_deltas(family_id, &batch).await.expect("stores");
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
