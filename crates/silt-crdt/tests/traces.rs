//! Replays real editing traces (fetch them with `scripts/fetch-traces.sh`).
//! Ignored by default because of their size: run with `--ignored --release`.

mod support;

use std::time::Instant;

use silt_crdt::TextReplica;
use silt_types::ActorId;
use support::trace;

fn check(name: &str) {
    let Some(trace) = trace::load(name) else {
        eprintln!("skipping {name}: run scripts/fetch-traces.sh first");
        return;
    };
    let started = Instant::now();
    let replica = trace::replay_local(&trace, usize::MAX);
    let local = started.elapsed();
    assert_eq!(replica.text(), trace.end_content);

    let started = Instant::now();
    let mut remote = TextReplica::new(ActorId(2));
    for event in replica.events() {
        remote.receive(event).expect("remote replay");
    }
    let merge = started.elapsed();
    assert_eq!(remote.text(), trace.end_content);
    eprintln!("{name}: {} txns, local {local:?}, remote {merge:?}", trace.txns.len());
}

#[test]
#[ignore = "large; run with --ignored --release"]
fn automerge_paper_trace_replays_to_its_final_text() {
    check("automerge-paper");
}

#[test]
#[ignore = "large; run with --ignored --release"]
fn seph_blog_trace_replays_to_its_final_text() {
    check("seph-blog1");
}
