//! A family's projection kept between loads, so loading it replays only
//! what was stored since (ADR 0018). The log's deltas aren't in it, as they
//! are stored already; only their ids are, to find them among the stored.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use silt_crdt::{CausalGraph, FileTree, InodeId};
use silt_types::{DeltaId, ThreadId};

use super::{Family, FileState, ThreadState, Turns};
use crate::delta::{Delta, SessionRef};

/// The encoding of [`Family::snapshot`]; a snapshot in another one is
/// replayed instead of read.
pub const SNAPSHOT_FORMAT: i64 = 2;

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("the snapshot could not be decoded")]
    Decode(#[from] postcard::Error),
    #[error("the stored deltas are not the ones the snapshot was taken of")]
    Mismatch,
}

#[derive(Serialize)]
struct Written<'a> {
    graph: &'a CausalGraph,
    tree: &'a FileTree,
    files: &'a HashMap<InodeId, FileState>,
    threads: &'a BTreeMap<ThreadId, ThreadState>,
    turns: &'a HashMap<SessionRef, Turns>,
    log: Vec<Logged>,
}

/// [`Written`], read back: postcard keeps fields in order.
#[derive(Deserialize)]
struct Read {
    graph: CausalGraph,
    tree: FileTree,
    files: HashMap<InodeId, FileState>,
    threads: BTreeMap<ThreadId, ThreadState>,
    turns: HashMap<SessionRef, Turns>,
    log: Vec<Logged>,
}

/// A delta of the log, told apart from a different one stored under the
/// same id by a fingerprint of its encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Logged {
    id: DeltaId,
    fingerprint: u64,
}

impl Logged {
    fn of(delta: &Delta) -> Result<Self, postcard::Error> {
        Ok(Self { id: delta.span.first(), fingerprint: fnv1a(&postcard::to_allocvec(delta)?) })
    }
}

/// Stable across builds, unlike the standard library's hashers.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3))
}

impl Family {
    pub fn snapshot(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(&Written {
            graph: &self.graph,
            tree: &self.tree,
            files: &self.files,
            threads: &self.threads,
            turns: &self.turns,
            log: self.log.iter().map(Logged::of).collect::<Result<_, _>>()?,
        })
    }

    /// The family `snapshot` was taken of, given every delta stored for it.
    /// Takes the deltas it holds out of `stored`, leaving those after them,
    /// still to be applied; on an error, `stored` is left as it was.
    pub fn from_snapshot(snapshot: &[u8], stored: &mut Vec<Delta>) -> Result<Self, SnapshotError> {
        let read: Read = postcard::from_bytes(snapshot)?;
        // Only deltas stored under one id need their contents told apart;
        // the store never holds two.
        let mut ids = HashSet::with_capacity(stored.len());
        let unique = stored.iter().all(|delta| ids.insert(delta.span.first()));
        let is = |delta: &Delta, logged: &Logged| {
            delta.span.first() == logged.id && (unique || Logged::of(delta).is_ok_and(|found| found == *logged))
        };
        let mut next = 0;
        let mut held = Vec::with_capacity(read.log.len());
        for logged in &read.log {
            // Stored deltas in between were refused or already known when the
            // snapshot was taken, as a replay would find them again.
            let found = stored[next..].iter().position(|delta| is(delta, logged)).ok_or(SnapshotError::Mismatch)?;
            held.push(next + found);
            next += found + 1;
        }
        let rest = stored.split_off(next);
        let mut held = held.into_iter().peekable();
        let log = std::mem::replace(stored, rest)
            .into_iter()
            .enumerate()
            .filter_map(|(index, delta)| held.next_if_eq(&index).map(|_| delta))
            .collect();
        let family = Self {
            graph: read.graph,
            tree: read.tree,
            files: read.files,
            threads: read.threads,
            turns: read.turns,
            log,
            ..Self::default()
        };
        Ok(family)
    }
}
