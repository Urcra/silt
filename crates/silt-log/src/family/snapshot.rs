//! A family's projection kept between loads, so loading it replays only
//! what was stored since (ADR 0018). The log's deltas aren't in it, as they
//! are stored already; only their ids are, to find them among the stored.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use silt_crdt::{CausalGraph, FileTree, InodeId};
use silt_types::{DeltaId, ThreadId};

use super::{Family, FileState, Log, ReadCovered, ThreadState, Turns};
use crate::delta::{Delta, SessionRef};

/// The encoding of [`Family::snapshot`]; a snapshot in another one is
/// replayed instead of read.
pub const SNAPSHOT_FORMAT: i64 = 5;

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("the snapshot could not be decoded")]
    Decode(#[from] postcard::Error),
    #[error("the stored deltas are not the ones the snapshot was taken of")]
    Mismatch,
}

/// What a snapshot holds besides the file tree and the files, which are
/// encoded apart so they decode at once on threads of their own: they are
/// most of a long family's snapshot (ADR 0034).
#[derive(Serialize)]
struct Written<'a> {
    graph: &'a CausalGraph,
    threads: &'a BTreeMap<ThreadId, ThreadState>,
    turns: &'a HashMap<SessionRef, Turns>,
    session_cwds: &'a HashMap<SessionRef, String>,
    log: Vec<Logged>,
}

/// [`Written`], read back: postcard keeps fields in order.
#[derive(Deserialize)]
struct Rest {
    graph: CausalGraph,
    threads: BTreeMap<ThreadId, ThreadState>,
    turns: HashMap<SessionRef, Turns>,
    session_cwds: HashMap<SessionRef, String>,
    log: Vec<Logged>,
}

/// A snapshot, decoded.
struct Read {
    tree: FileTree,
    files: HashMap<InodeId, FileState>,
    rest: Rest,
}

impl Read {
    /// `snapshot`'s three parts: the tree, the files and the rest, each
    /// after its length.
    fn decode(snapshot: &[u8]) -> Result<Self, SnapshotError> {
        let (tree, after) = framed(snapshot)?;
        let (files, rest) = framed(after)?;
        let (tree, files, rest) = decode_apart(tree, files, rest);
        Ok(Self { tree: tree?, files: files?, rest: rest? })
    }

    fn encode(tree: &FileTree, files: &HashMap<InodeId, FileState>, rest: &Written<'_>) -> Result<Vec<u8>, postcard::Error> {
        let mut encoded = Vec::new();
        for part in [postcard::to_allocvec(tree)?, postcard::to_allocvec(files)?] {
            encoded.extend_from_slice(&(part.len() as u64).to_le_bytes());
            encoded.extend_from_slice(&part);
        }
        postcard::to_extend(rest, encoded)
    }
}

type Decoded<T> = Result<T, postcard::Error>;

/// The part at the start of `bytes`, after its length, and what follows it.
fn framed(bytes: &[u8]) -> Result<(&[u8], &[u8]), postcard::Error> {
    let (length, rest) = bytes.split_first_chunk::<8>().ok_or(postcard::Error::DeserializeUnexpectedEnd)?;
    let length = usize::try_from(u64::from_le_bytes(*length)).map_err(|_| postcard::Error::DeserializeBadEncoding)?;
    if rest.len() < length {
        return Err(postcard::Error::DeserializeUnexpectedEnd);
    }
    Ok(rest.split_at(length))
}

#[cfg(not(target_arch = "wasm32"))]
fn decode_apart(
    tree: &[u8],
    files: &[u8],
    rest: &[u8],
) -> (Decoded<FileTree>, Decoded<HashMap<InodeId, FileState>>, Decoded<Rest>) {
    std::thread::scope(|scope| {
        let tree = scope.spawn(|| postcard::from_bytes(tree));
        let files = scope.spawn(|| postcard::from_bytes(files));
        let rest = postcard::from_bytes(rest);
        (tree.join().expect("decoding doesn't panic"), files.join().expect("decoding doesn't panic"), rest)
    })
}

/// A browser's replica has no threads to decode on.
#[cfg(target_arch = "wasm32")]
fn decode_apart(
    tree: &[u8],
    files: &[u8],
    rest: &[u8],
) -> (Decoded<FileTree>, Decoded<HashMap<InodeId, FileState>>, Decoded<Rest>) {
    (postcard::from_bytes(tree), postcard::from_bytes(files), postcard::from_bytes(rest))
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
        Ok(Self { id: delta.span.first(), fingerprint: fingerprint(&postcard::to_allocvec(delta)?) })
    }
}

/// FNV-1a over eight bytes at a time: stable across builds, unlike the
/// standard library's hashers, and fast enough for a long session's log.
fn fingerprint(bytes: &[u8]) -> u64 {
    const PRIME: u64 = 0x0100_0000_01b3;
    let (words, rest) = bytes.as_chunks::<8>();
    let hash = words.iter().fold(0xcbf2_9ce4_8422_2325, |hash, word| (hash ^ u64::from_le_bytes(*word)).wrapping_mul(PRIME));
    let hash = rest.iter().fold(hash, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(PRIME));
    (hash ^ bytes.len() as u64).wrapping_mul(PRIME)
}

impl Family {
    pub fn snapshot(&mut self) -> Result<Vec<u8>, postcard::Error> {
        let mut encoded = Vec::new();
        for delta in &self.log.all()[self.fingerprints.len()..] {
            encoded.clear();
            encoded = postcard::to_extend(delta, encoded)?;
            self.fingerprints.push(fingerprint(&encoded));
        }
        let log = self
            .log
            .all()
            .iter()
            .zip(&self.fingerprints)
            .map(|(delta, &fingerprint)| Logged { id: delta.span.first(), fingerprint });
        let rest = Written {
            graph: &self.graph,
            threads: &self.threads,
            turns: &self.turns,
            session_cwds: &self.session_cwds,
            log: log.collect(),
        };
        Read::encode(&self.tree, &self.files, &rest)
    }

    /// The family `snapshot` was taken of, given every delta stored for it.
    /// Takes the deltas it holds out of `stored`, leaving those after them,
    /// still to be applied; on an error, `stored` is left as it was.
    pub fn from_snapshot(snapshot: &[u8], stored: &mut Vec<Delta>) -> Result<Self, SnapshotError> {
        let read = Read::decode(snapshot)?;
        // Only deltas stored under one id need their contents told apart;
        // the store never holds two.
        let mut ids = HashSet::with_capacity(stored.len());
        let unique = stored.iter().all(|delta| ids.insert(delta.span.first()));
        let is = |delta: &Delta, logged: &Logged| {
            delta.span.first() == logged.id && (unique || Logged::of(delta).is_ok_and(|found| found == *logged))
        };
        let mut next = 0;
        let mut held = Vec::with_capacity(read.rest.log.len());
        for logged in &read.rest.log {
            // Stored deltas in between were refused or already known when the
            // snapshot was taken, as a replay would find them again.
            let found = stored[next..].iter().position(|delta| is(delta, logged)).ok_or(SnapshotError::Mismatch)?;
            held.push(next + found);
            next += found + 1;
        }
        let rest = stored.split_off(next);
        let mut held = held.into_iter().peekable();
        let log: Vec<Delta> = std::mem::replace(stored, rest)
            .into_iter()
            .enumerate()
            .filter_map(|(index, delta)| held.next_if_eq(&index).map(|_| delta))
            .collect();
        Ok(Self::restored(read, Log::from(log)))
    }

    /// The family `snapshot` was taken of, when the first `stored` deltas
    /// stored for it are those it covers, every one of them, ending with
    /// `last`. They're read with `read` only when something reads the log,
    /// and those stored after them are still to be applied.
    pub fn from_snapshot_unread(snapshot: &[u8], stored: usize, last: DeltaId, read: ReadCovered) -> Result<Self, SnapshotError> {
        let snapshot = Read::decode(snapshot)?;
        let log = &snapshot.rest.log;
        if log.len() != stored || log.last().map(|logged| logged.id) != Some(last) {
            return Err(SnapshotError::Mismatch);
        }
        let covered = log.len();
        Ok(Self::restored(snapshot, Log::unread(covered, read)))
    }

    fn restored(read: Read, log: Log) -> Self {
        let Read { tree, files, rest } = read;
        Self {
            graph: rest.graph,
            tree,
            files,
            threads: rest.threads,
            turns: rest.turns,
            session_cwds: rest.session_cwds,
            log,
            fingerprints: rest.log.iter().map(|logged| logged.fingerprint).collect(),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use silt_types::ActorId;

    use super::*;
    use crate::delta::{Author, ConvOp, Op, SessionFile};

    fn say(family: &mut Family, text: &str) -> Delta {
        let thread = ThreadId(1);
        let session = SessionRef { agent: "claude-code".into(), id: "s".into() };
        let op = Op::Conv(ConvOp::Native { session, file: SessionFile::Main, line: text.into() });
        let delta = family.delta(ActorId(1), thread, Author::User, 0, family.thread_head(thread), op);
        family.apply(delta.clone(), &mut HashMap::new()).expect("applies");
        delta
    }

    fn logged(snapshot: &[u8]) -> Vec<Logged> {
        Read::decode(snapshot).expect("decodes").rest.log
    }

    #[test]
    fn fingerprints_kept_between_snapshots_are_the_ones_taken_fresh() {
        let mut family = Family::new();
        let mut stored: Vec<Delta> = ["one", "two"].iter().map(|text| say(&mut family, text)).collect();
        let first = family.snapshot().expect("encodes");
        stored.push(say(&mut family, "three"));
        let fresh: Vec<Logged> = family.log().iter().map(|delta| Logged::of(delta).expect("encodes")).collect();
        assert_eq!(logged(&family.snapshot().expect("encodes")), fresh);

        let mut rest = stored;
        let mut restored = Family::from_snapshot(&first, &mut rest).expect("matches");
        for delta in rest {
            restored.apply(delta, &mut HashMap::new()).expect("applies");
        }
        assert_eq!(logged(&restored.snapshot().expect("encodes")), fresh, "fingerprints read with a snapshot are kept");
    }

    #[test]
    fn a_family_read_from_its_snapshot_unread_is_the_one_replayed_once_its_log_is_read() {
        let mut family = Family::new();
        let mut stored: Vec<Delta> = ["one", "two"].iter().map(|text| say(&mut family, text)).collect();
        let snapshot = family.snapshot().expect("encodes");
        let session = SessionRef { agent: "claude-code".into(), id: "s".into() };
        let started =
            Op::Conv(ConvOp::SessionStarted { session: session.clone(), cwd: "/repo/sub".into(), checkout_root: "/repo".into() });
        let after = family.delta(ActorId(1), ThreadId(1), Author::User, 0, family.thread_head(ThreadId(1)), started);
        family.apply(after.clone(), &mut HashMap::new()).expect("applies");
        let covered = stored.clone();
        stored.push(after.clone());

        let last = covered[1].span.first();
        let mut unread =
            Family::from_snapshot_unread(&snapshot, 2, last, Box::new(move || covered)).expect("the snapshot covers them");
        unread.apply(after, &mut HashMap::new()).expect("applies");
        assert_eq!(unread.log_len(), 3);
        assert_eq!(unread.session_cwd(&session), Some("/repo/sub"), "where the session started is in the projection");
        assert_eq!(unread.log(), stored);
        assert_eq!(unread.log(), family.log());

        let other = say(&mut Family::new(), "elsewhere").span.first();
        assert!(Family::from_snapshot_unread(&snapshot, 2, other, Box::new(Vec::new)).is_err(), "another last delta");
        assert!(Family::from_snapshot_unread(&snapshot, 3, last, Box::new(Vec::new)).is_err(), "more stored than covered");
    }
}
