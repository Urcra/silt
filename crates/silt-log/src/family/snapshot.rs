//! A family's projection kept between loads, so loading it replays only
//! what was stored since (ADR 0018). The log's deltas aren't in it, as they
//! are stored already; only their ids are, to find them among the stored.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use silt_crdt::{CausalGraph, FileTree, InodeId};
use silt_types::{BlobHash, DeltaId, ThreadId};

use super::{Family, FileState, Files, Log, NewFile, ReadCovered, ReadFile, ThreadState, Turns};
use crate::delta::{Delta, SessionRef};

/// The encoding of [`Family::snapshot`]: the tree, an index of the files'
/// states, stored apart, and the rest (ADR 0115).
pub const SNAPSHOT_FORMAT: i64 = 6;

/// The encoding before files were stored apart: the tree, the files, the
/// rest. Read, and taken again in [`SNAPSHOT_FORMAT`].
const FILES_WITHIN: i64 = 5;

/// Whether a snapshot in `format` can be read; one that can't is replayed.
pub fn is_readable_format(format: i64) -> bool {
    format == SNAPSHOT_FORMAT || format == FILES_WITHIN
}

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("the snapshot could not be decoded")]
    Decode(#[from] postcard::Error),
    #[error("the stored deltas are not the ones the snapshot was taken of")]
    Mismatch,
    #[error("snapshots in format {0} can't be read")]
    Format(i64),
}

/// A snapshot as [`Family::snapshot`] takes it: its encoding, and the
/// files' states it names that aren't stored yet, to store before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TakenSnapshot {
    pub data: Vec<u8>,
    pub files: Vec<NewFile>,
}

/// The hashes of the files' states a snapshot names, so a reader can check
/// they're all there before reading it.
pub fn snapshot_files(snapshot: &[u8], format: i64) -> Result<Vec<BlobHash>, SnapshotError> {
    match format {
        SNAPSHOT_FORMAT => {
            let (_, after) = framed(snapshot)?;
            let (index, _) = framed(after)?;
            let index: Vec<(InodeId, BlobHash)> = postcard::from_bytes(index)?;
            Ok(index.into_iter().map(|(_, hash)| hash).collect())
        }
        FILES_WITHIN => Ok(Vec::new()),
        other => Err(SnapshotError::Format(other)),
    }
}

/// What a snapshot holds besides the file tree and the files, which are
/// encoded apart so they decode at once on threads of their own (ADR 0034).
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
    files: Files,
    rest: Rest,
}

impl Read {
    /// `snapshot`'s three parts, each after its length: the tree, the
    /// files (or in [`SNAPSHOT_FORMAT`] their index) and the rest.
    fn decode(snapshot: &[u8], format: i64, read: Option<ReadFile>) -> Result<Self, SnapshotError> {
        let (tree, after) = framed(snapshot)?;
        let (files, rest) = framed(after)?;
        let files = match (format, read) {
            (SNAPSHOT_FORMAT, Some(read)) => Part::Index(files, read),
            (SNAPSHOT_FORMAT, None) => return Err(SnapshotError::Format(format)),
            (FILES_WITHIN, read) => Part::Within(files, read),
            (other, _) => return Err(SnapshotError::Format(other)),
        };
        let (tree, files, rest) = decode_apart(tree, files, rest);
        Ok(Self { tree: tree?, files: files?, rest: rest? })
    }

    fn encode(tree: &FileTree, index: &[(InodeId, BlobHash)], rest: &Written<'_>) -> Result<Vec<u8>, postcard::Error> {
        let mut encoded = Vec::new();
        for part in [postcard::to_allocvec(tree)?, postcard::to_allocvec(index)?] {
            encoded.extend_from_slice(&(part.len() as u64).to_le_bytes());
            encoded.extend_from_slice(&part);
        }
        postcard::to_extend(rest, encoded)
    }
}

/// A snapshot's files: their index, or in [`FILES_WITHIN`] the files.
enum Part<'a> {
    Index(&'a [u8], ReadFile),
    Within(&'a [u8], Option<ReadFile>),
}

impl Part<'_> {
    fn decode(self) -> Decoded<Files> {
        Ok(match self {
            Self::Index(index, read) => Files::stored(postcard::from_bytes(index)?, read),
            Self::Within(files, read) => Files::decoded(postcard::from_bytes::<HashMap<InodeId, FileState>>(files)?, read),
        })
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
fn decode_apart(tree: &[u8], files: Part<'_>, rest: &[u8]) -> (Decoded<FileTree>, Decoded<Files>, Decoded<Rest>) {
    std::thread::scope(|scope| {
        let tree = scope.spawn(|| postcard::from_bytes(tree));
        let files = scope.spawn(|| files.decode());
        let rest = postcard::from_bytes(rest);
        (tree.join().expect("decoding doesn't panic"), files.join().expect("decoding doesn't panic"), rest)
    })
}

/// A browser's replica has no threads to decode on.
#[cfg(target_arch = "wasm32")]
fn decode_apart(tree: &[u8], files: Part<'_>, rest: &[u8]) -> (Decoded<FileTree>, Decoded<Files>, Decoded<Rest>) {
    (postcard::from_bytes(tree), files.decode(), postcard::from_bytes(rest))
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
    /// Takes a snapshot. Its new files are to be stored before it, and
    /// then told to [`Self::snapshot_stored`].
    pub fn snapshot(&mut self) -> Result<TakenSnapshot, postcard::Error> {
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
        let files = self.files.unstored()?;
        let data = Read::encode(&self.tree, &self.files.index(&files), &rest)?;
        Ok(TakenSnapshot { data, files })
    }

    /// Notes that `snapshot`'s new files were stored, read back with `read`,
    /// so they can be let go of when unused. Nothing may change the family
    /// between taking the snapshot and this.
    pub fn snapshot_stored(&mut self, snapshot: &TakenSnapshot, read: ReadFile) {
        self.files.mark_stored(&snapshot.files, read);
    }

    /// Lets go of the files unused since the last call that are as stored,
    /// to read them again when used; returns how many.
    pub fn release_files(&mut self) -> usize {
        self.files.release()
    }

    /// The hashes of the files' states this family reads from the store.
    pub fn stored_files(&self) -> impl Iterator<Item = silt_types::BlobHash> + '_ {
        self.files.stored_hashes()
    }

    /// How many files are decoded.
    pub fn decoded_files(&self) -> usize {
        self.files.decoded_len()
    }

    /// Whether a file's stored state couldn't be read, so that the family
    /// is to be replayed rather than read from its snapshot.
    pub fn files_unreadable(&self) -> bool {
        self.files.unreadable()
    }

    /// The family `snapshot` was taken of, given every delta stored for it.
    /// Takes the deltas it holds out of `stored`, leaving those after them,
    /// still to be applied; on an error, `stored` is left as it was.
    /// Its files are read with `read` when used.
    pub fn from_snapshot(
        snapshot: &[u8],
        format: i64,
        stored: &mut Vec<Delta>,
        read: Option<ReadFile>,
    ) -> Result<Self, SnapshotError> {
        let read = Read::decode(snapshot, format, read)?;
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
    pub fn from_snapshot_unread(
        snapshot: &[u8],
        format: i64,
        stored: usize,
        last: DeltaId,
        read: ReadCovered,
        files: Option<ReadFile>,
    ) -> Result<Self, SnapshotError> {
        let snapshot = Read::decode(snapshot, format, files)?;
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

    use silt_crdt::{Parent, TextOp};
    use silt_types::ActorId;

    use super::*;
    use crate::FileView;
    use crate::delta::{Author, Content, ConvOp, Op, SessionFile};
    use crate::family::MemoryFiles;

    const THREAD: ThreadId = ThreadId(1);

    fn write(family: &mut Family, op: Op) -> Delta {
        let delta = family.delta(ActorId(1), THREAD, Author::User, 0, family.thread_head(THREAD), op);
        family.apply(delta.clone(), &mut HashMap::new()).expect("applies");
        delta
    }

    fn say(family: &mut Family, text: &str) -> Delta {
        let session = SessionRef { agent: "claude-code".into(), id: "s".into() };
        write(family, Op::Conv(ConvOp::Native { session, file: SessionFile::Main, line: text.into() }))
    }

    /// A file named `name` holding `text`, typed in after it was created.
    fn file(family: &mut Family, name: &str, text: &str) -> (InodeId, Vec<Delta>) {
        let create = Op::Create {
            kind: silt_crdt::InodeKind::File,
            parent: Parent::Root,
            name: name.into(),
            content: Content::Empty,
            executable: false,
        };
        let created = write(family, create);
        let inode = InodeId(created.span.first());
        let typed = write(family, Op::Text { inode, op: TextOp::Insert { pos: 0, content: text.into() } });
        (inode, vec![created, typed])
    }

    /// Takes a snapshot, keeping its files in `files`.
    fn take(family: &mut Family, files: &MemoryFiles) -> Vec<u8> {
        let taken = family.snapshot().expect("encodes");
        files.store(&taken);
        family.snapshot_stored(&taken, files.reader());
        taken.data
    }

    fn logged(snapshot: &[u8]) -> Vec<Logged> {
        Read::decode(snapshot, SNAPSHOT_FORMAT, Some(MemoryFiles::default().reader())).expect("decodes").rest.log
    }

    fn text(family: &Family, inode: InodeId) -> Option<FileView> {
        family.file(inode)
    }

    #[test]
    fn fingerprints_kept_between_snapshots_are_the_ones_taken_fresh() {
        let files = MemoryFiles::default();
        let mut family = Family::new();
        let mut stored: Vec<Delta> = ["one", "two"].iter().map(|text| say(&mut family, text)).collect();
        let first = take(&mut family, &files);
        stored.push(say(&mut family, "three"));
        let fresh: Vec<Logged> = family.log().iter().map(|delta| Logged::of(delta).expect("encodes")).collect();
        assert_eq!(logged(&take(&mut family, &files)), fresh);

        let mut rest = stored;
        let mut restored = Family::from_snapshot(&first, SNAPSHOT_FORMAT, &mut rest, Some(files.reader())).expect("matches");
        for delta in rest {
            restored.apply(delta, &mut HashMap::new()).expect("applies");
        }
        assert_eq!(logged(&take(&mut restored, &files)), fresh, "fingerprints read with a snapshot are kept");
    }

    #[test]
    fn a_family_read_from_its_snapshot_unread_is_the_one_replayed_once_its_log_is_read() {
        let files = MemoryFiles::default();
        let mut family = Family::new();
        let mut stored: Vec<Delta> = ["one", "two"].iter().map(|text| say(&mut family, text)).collect();
        let snapshot = take(&mut family, &files);
        let session = SessionRef { agent: "claude-code".into(), id: "s".into() };
        let started =
            Op::Conv(ConvOp::SessionStarted { session: session.clone(), cwd: "/repo/sub".into(), checkout_root: "/repo".into() });
        let after = write(&mut family, started);
        let covered = stored.clone();
        stored.push(after.clone());

        let last = covered[1].span.first();
        let unread = |stored, last, covered: Vec<Delta>| {
            Family::from_snapshot_unread(
                &snapshot,
                SNAPSHOT_FORMAT,
                stored,
                last,
                Box::new(move || covered),
                Some(files.reader()),
            )
        };
        let mut family_unread = unread(2, last, covered.clone()).expect("the snapshot covers them");
        family_unread.apply(after, &mut HashMap::new()).expect("applies");
        assert_eq!(family_unread.log_len(), 3);
        assert_eq!(family_unread.session_cwd(&session), Some("/repo/sub"), "where the session started is in the projection");
        assert_eq!(family_unread.log(), stored);
        assert_eq!(family_unread.log(), family.log());

        let other = say(&mut Family::new(), "elsewhere").span.first();
        assert!(unread(2, other, Vec::new()).is_err(), "another last delta");
        assert!(unread(3, last, Vec::new()).is_err(), "more stored than covered");
    }

    #[test]
    fn files_are_read_when_used_and_let_go_of_when_unused_and_as_stored() {
        let files = MemoryFiles::default();
        let mut family = Family::new();
        let (a, mut stored) = file(&mut family, "a", "alpha\n");
        let (b, more) = file(&mut family, "b", "beta\n");
        stored.extend(more);
        let snapshot = take(&mut family, &files);
        assert_eq!(files.len(), 2);

        let mut rest = stored.clone();
        let mut restored = Family::from_snapshot(&snapshot, SNAPSHOT_FORMAT, &mut rest, Some(files.reader())).expect("matches");
        assert!(rest.is_empty());
        assert_eq!(restored.decoded_files(), 0, "nothing is decoded until read");
        assert_eq!(text(&restored, a), Some(FileView::Text("alpha\n".into())));
        assert_eq!(restored.decoded_files(), 1);

        let typed = restored.delta(
            ActorId(2),
            THREAD,
            Author::User,
            0,
            restored.thread_head(THREAD),
            Op::Text { inode: b, op: TextOp::Insert { pos: 0, content: "and ".into() } },
        );
        restored.apply(typed, &mut HashMap::new()).expect("applies");
        assert_eq!(restored.release_files(), 0, "both were used since loading");
        assert_eq!(restored.release_files(), 1, "a is as stored; b changed and isn't stored yet");
        assert_eq!(restored.decoded_files(), 1);
        assert_eq!(text(&restored, a), Some(FileView::Text("alpha\n".into())), "a is read again");
        assert_eq!(text(&restored, b), Some(FileView::Text("and beta\n".into())));

        take(&mut restored, &files);
        assert_eq!(files.len(), 3, "only b's new state is stored");
        restored.release_files();
        restored.release_files();
        assert_eq!(restored.decoded_files(), 0, "once stored, b is let go of too");
        assert_eq!(text(&restored, b), Some(FileView::Text("and beta\n".into())));
        assert!(!restored.files_unreadable());
    }

    #[test]
    fn a_snapshot_with_files_within_is_read_and_taken_again_with_them_apart() {
        let mut family = Family::new();
        let (a, mut stored) = file(&mut family, "a", "alpha\n");
        stored.push(say(&mut family, "said"));
        let rest = Written {
            graph: &family.graph,
            threads: &family.threads,
            turns: &family.turns,
            session_cwds: &family.session_cwds,
            log: family.log().iter().map(|delta| Logged::of(delta).expect("encodes")).collect(),
        };
        let mut old = Vec::new();
        for part in [
            postcard::to_allocvec(&family.tree).expect("encodes"),
            postcard::to_allocvec(&family.files.states()).expect("encodes"),
        ] {
            old.extend_from_slice(&(part.len() as u64).to_le_bytes());
            old.extend_from_slice(&part);
        }
        let old = postcard::to_extend(&rest, old).expect("encodes");

        assert!(is_readable_format(FILES_WITHIN));
        let mut deltas = stored.clone();
        let mut restored = Family::from_snapshot(&old, FILES_WITHIN, &mut deltas, None).expect("reads the older format");
        assert_eq!(text(&restored, a), Some(FileView::Text("alpha\n".into())));
        let files = MemoryFiles::default();
        let snapshot = take(&mut restored, &files);
        assert_eq!(snapshot_files(&snapshot, SNAPSHOT_FORMAT).expect("reads its index").len(), 1);
        let mut deltas = stored;
        let again = Family::from_snapshot(&snapshot, SNAPSHOT_FORMAT, &mut deltas, Some(files.reader())).expect("matches");
        assert_eq!(text(&again, a), Some(FileView::Text("alpha\n".into())));
    }

    #[test]
    fn a_file_whose_state_is_missing_says_the_family_is_to_be_replayed() {
        let files = MemoryFiles::default();
        let mut family = Family::new();
        let (a, mut stored) = file(&mut family, "a", "alpha\n");
        let snapshot = take(&mut family, &files);
        let restored = Family::from_snapshot(&snapshot, SNAPSHOT_FORMAT, &mut stored, Some(MemoryFiles::default().reader()))
            .expect("matches");
        assert_eq!(restored.file(a), None);
        assert!(restored.files_unreadable());
        assert!(Family::from_snapshot(&snapshot, SNAPSHOT_FORMAT, &mut Vec::new(), None).is_err(), "no reader for its files");
    }
}
