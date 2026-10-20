use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use serde::Deserialize;
use silt_crdt::TextReplica;
use silt_types::ActorId;

#[derive(Debug, Deserialize)]
pub struct Trace {
    #[serde(rename = "startContent")]
    pub start_content: String,
    #[serde(rename = "endContent")]
    pub end_content: String,
    pub txns: Vec<Txn>,
}

#[derive(Debug, Deserialize)]
pub struct Txn {
    pub patches: Vec<(usize, usize, String)>,
}

/// Loads `target/editing-traces/<name>.json.gz`, fetched by
/// `scripts/fetch-traces.sh`. Returns `None` when it hasn't been fetched.
pub fn load(name: &str) -> Option<Trace> {
    let path = traces_dir().join(format!("{name}.json.gz"));
    let file = File::open(&path).ok()?;
    let trace = serde_json::from_reader(GzDecoder::new(BufReader::new(file)))
        .unwrap_or_else(|error| panic!("{} is not a valid trace: {error}", path.display()));
    Some(trace)
}

fn traces_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/editing-traces")
}

/// Replays the trace as one user typing, returning the replica.
pub fn replay_local(trace: &Trace, limit: usize) -> TextReplica {
    let mut replica = TextReplica::new(ActorId(1));
    if !trace.start_content.is_empty() {
        replica.insert(0, &trace.start_content).expect("start content");
    }
    for txn in trace.txns.iter().take(limit) {
        for (pos, deleted, inserted) in &txn.patches {
            if *deleted > 0 {
                replica.delete(*pos, *deleted).expect("trace delete");
            }
            if !inserted.is_empty() {
                replica.insert(*pos, inserted).expect("trace insert");
            }
        }
    }
    replica
}
