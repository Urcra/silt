//! `checkout.rescan`: the capture step after a Bash call compares the whole
//! checkout with its recorded state (ADR 0018), the way into a rescan that
//! needs no daemon.

use std::path::Path;
use std::time::{Duration, Instant};

use exn::{Exn, ResultExt};
use silt_daemon::{NewRun, Recorder, capture_here};

use crate::BenchError;
use crate::budgets::{Measured, median};
use crate::fixtures::clone_repo;

const RUNS: usize = 3;

pub async fn measure(root: &Path, repo: &Path) -> Result<Measured, Exn<BenchError>> {
    let (data, checkout) = (root.join("silt"), root.join("repo"));
    clone_repo(repo, &checkout).or_raise(|| BenchError("cannot clone the 50k-file repository".into()))?;
    let session = uuid::Uuid::new_v4().to_string();
    let run = NewRun { cwd: &checkout, agent: "claude", session_id: Some(&session), isolated: false };
    drop(Recorder::start(&data, run).await.or_raise(|| BenchError("cannot start a session".into()))?);

    // An mtime under 2 s old isn't trusted, so a fresh clone's files would
    // all be read; a checkout people work in is mostly older than that.
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    let mut times = Vec::new();
    // The first compare records the clone's disk state; later ones are the
    // steady state, where nothing changed.
    for run in 0..=RUNS {
        let input = |event: &str| {
            serde_json::json!({
                "session_id": session,
                "hook_event_name": event,
                "tool_name": "Bash",
                "tool_use_id": format!("toolu_{run}"),
                "tool_input": { "command": "true" },
            })
        };
        capture_here(&data, "PreToolUse", input("PreToolUse")).await.or_raise(|| BenchError("cannot capture".into()))?;
        let start = Instant::now();
        capture_here(&data, "PostToolUse", input("PostToolUse")).await.or_raise(|| BenchError("cannot capture".into()))?;
        if run > 0 {
            times.push(start.elapsed());
        }
    }
    Ok(Measured::Time(median(times).expect("measured")))
}
