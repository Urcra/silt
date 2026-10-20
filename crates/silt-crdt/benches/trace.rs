//! Fetch the traces first with `scripts/fetch-traces.sh`.

#[path = "../tests/support/trace.rs"]
mod trace;

use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use silt_crdt::TextReplica;
use silt_types::ActorId;

fn replay(c: &mut Criterion) {
    for name in ["automerge-paper", "seph-blog1"] {
        let Some(trace) = trace::load(name) else {
            eprintln!("skipping {name}: run scripts/fetch-traces.sh first");
            continue;
        };
        let mut group = c.benchmark_group(name);
        group.sample_size(10);
        group.bench_function("local typing", |b| {
            b.iter(|| black_box(trace::replay_local(&trace, usize::MAX)));
        });
        let replayed = trace::replay_local(&trace, usize::MAX);
        assert_eq!(replayed.text(), trace.end_content, "{name} replays to its final text");
        let events = replayed.events().to_vec();
        group.bench_function("remote merge", |b| {
            b.iter_batched(
                || TextReplica::new(ActorId(2)),
                |mut replica| {
                    for event in &events {
                        replica.receive(event).expect("replays");
                    }
                    black_box(replica)
                },
                BatchSize::LargeInput,
            );
        });
        group.finish();
    }
}

criterion_group!(benches, replay);
criterion_main!(benches);
