//! A family's files, each decoded from the state its snapshot keeps apart
//! only when something reads it. A long family's files were most of its
//! memory, though a session touches few of them (ADR 0115).

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use silt_crdt::InodeId;
use silt_types::BlobHash;

use super::FileState;
use super::snapshot::TakenSnapshot;

/// Reads a file's state, kept apart from its family's snapshot, by its hash.
pub type ReadFile = Arc<dyn Fn(BlobHash) -> Option<Vec<u8>> + Send + Sync>;

/// A file's state not stored yet, to store before the snapshot naming it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewFile {
    pub inode: InodeId,
    pub hash: BlobHash,
    pub data: Vec<u8>,
}

/// The hash a file's encoded state is stored under.
pub fn file_hash(data: &[u8]) -> BlobHash {
    BlobHash(*blake3::hash(data).as_bytes())
}

/// Files' states kept in memory, for families whose snapshots are too: a
/// browser's replica, tests.
#[derive(Clone, Default)]
pub struct MemoryFiles(Arc<Mutex<HashMap<BlobHash, Vec<u8>>>>);

impl MemoryFiles {
    /// Keeps `snapshot`'s new files.
    pub fn store(&self, snapshot: &TakenSnapshot) {
        let mut held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        for file in &snapshot.files {
            held.insert(file.hash, file.data.clone());
        }
    }

    pub fn reader(&self) -> ReadFile {
        let held = Arc::clone(&self.0);
        Arc::new(move |hash| held.lock().unwrap_or_else(PoisonError::into_inner).get(&hash).cloned())
    }

    pub fn len(&self) -> usize {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Default)]
pub(super) struct Files {
    slots: HashMap<InodeId, Slot>,
    read: Option<ReadFile>,
    /// Advanced by each release; a slot read since the last one is kept.
    generation: u64,
    unreadable: AtomicBool,
}

struct Slot {
    /// Where the state is stored, while it is as stored.
    stored: Option<BlobHash>,
    state: OnceLock<FileState>,
    used: AtomicU64,
}

impl Slot {
    fn decoded(state: FileState, generation: u64) -> Self {
        Self { stored: None, state: OnceLock::from(state), used: AtomicU64::new(generation) }
    }
}

impl Files {
    /// Files whose states are stored, read with `read` when first used.
    pub(super) fn stored(index: Vec<(InodeId, BlobHash)>, read: ReadFile) -> Self {
        let slots = index
            .into_iter()
            .map(|(inode, hash)| (inode, Slot { stored: Some(hash), state: OnceLock::new(), used: AtomicU64::new(0) }))
            .collect();
        Self { slots, read: Some(read), ..Self::default() }
    }

    /// Files decoded already, none stored apart yet.
    pub(super) fn decoded(states: HashMap<InodeId, FileState>, read: Option<ReadFile>) -> Self {
        let slots = states.into_iter().map(|(inode, state)| (inode, Slot::decoded(state, 0))).collect();
        Self { slots, read, ..Self::default() }
    }

    pub(super) fn get(&self, inode: &InodeId) -> Option<&FileState> {
        let slot = self.slots.get(inode)?;
        slot.used.store(self.generation, Ordering::Relaxed);
        if let Some(state) = slot.state.get() {
            return Some(state);
        }
        match self.read_state(slot) {
            Some(state) => Some(slot.state.get_or_init(|| state)),
            None => {
                self.unreadable.store(true, Ordering::Relaxed);
                None
            }
        }
    }

    pub(super) fn get_mut(&mut self, inode: &InodeId) -> Option<&mut FileState> {
        self.get(inode)?;
        let slot = self.slots.get_mut(inode).expect("read above");
        slot.stored = None;
        slot.state.get_mut()
    }

    pub(super) fn insert(&mut self, inode: InodeId, state: FileState) {
        self.slots.insert(inode, Slot::decoded(state, self.generation));
    }

    pub(super) fn contains_key(&self, inode: &InodeId) -> bool {
        self.slots.contains_key(inode)
    }

    fn read_state(&self, slot: &Slot) -> Option<FileState> {
        let bytes = (self.read.as_ref()?)(slot.stored?)?;
        postcard::from_bytes(&bytes).ok()
    }

    /// Whether a stored state couldn't be read: the family's snapshot is
    /// no longer to be trusted, and it should be replayed instead.
    pub(super) fn unreadable(&self) -> bool {
        self.unreadable.load(Ordering::Relaxed)
    }

    /// Every file's hash, those of states not stored yet among them.
    pub(super) fn index(&self, new: &[NewFile]) -> Vec<(InodeId, BlobHash)> {
        let new: HashMap<InodeId, BlobHash> = new.iter().map(|file| (file.inode, file.hash)).collect();
        let mut index: Vec<(InodeId, BlobHash)> =
            self.slots.iter().map(|(inode, slot)| (*inode, slot.stored.unwrap_or_else(|| new[inode]))).collect();
        index.sort_unstable_by_key(|(inode, _)| *inode);
        index
    }

    /// The states changed since they were stored, encoded.
    pub(super) fn unstored(&self) -> Result<Vec<NewFile>, postcard::Error> {
        let mut new = Vec::new();
        for (inode, slot) in self.slots.iter().filter(|(_, slot)| slot.stored.is_none()) {
            let state = slot.state.get().expect("a file not stored is decoded");
            let data = postcard::to_allocvec(state)?;
            new.push(NewFile { inode: *inode, hash: file_hash(&data), data });
        }
        Ok(new)
    }

    /// Notes that `stored`, as [`Self::unstored`] gave them with nothing
    /// changed since, were stored, and that the family now reads its files
    /// with `read`.
    pub(super) fn mark_stored(&mut self, stored: &[NewFile], read: ReadFile) {
        for file in stored {
            if let Some(slot) = self.slots.get_mut(&file.inode) {
                slot.stored = Some(file.hash);
            }
        }
        self.read = Some(read);
    }

    /// Lets go of the states unused since the last release that are as
    /// stored, and returns how many.
    pub(super) fn release(&mut self) -> usize {
        let mut released = 0;
        if self.read.is_some() {
            for slot in self.slots.values_mut() {
                if slot.stored.is_some() && *slot.used.get_mut() < self.generation && slot.state.take().is_some() {
                    released += 1;
                }
            }
        }
        self.generation += 1;
        released
    }

    #[cfg(test)]
    pub(super) fn states(&self) -> HashMap<InodeId, FileState> {
        self.slots.keys().map(|inode| (*inode, self.get(inode).expect("readable").clone())).collect()
    }

    /// The hashes of the states stored for these files.
    pub(super) fn stored_hashes(&self) -> impl Iterator<Item = BlobHash> + '_ {
        self.slots.values().filter_map(|slot| slot.stored)
    }

    /// How many files' states are decoded.
    pub(super) fn decoded_len(&self) -> usize {
        self.slots.values().filter(|slot| slot.state.get().is_some()).count()
    }
}

impl Clone for Files {
    fn clone(&self) -> Self {
        let slots = self
            .slots
            .iter()
            .map(|(inode, slot)| {
                let state = slot.state.get().cloned().map_or_else(OnceLock::new, OnceLock::from);
                (*inode, Slot { stored: slot.stored, state, used: AtomicU64::new(slot.used.load(Ordering::Relaxed)) })
            })
            .collect();
        Self { slots, read: self.read.clone(), generation: self.generation, unreadable: AtomicBool::new(self.unreadable()) }
    }
}

impl fmt::Debug for Files {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Files").field("files", &self.slots.len()).field("decoded", &self.decoded_len()).finish()
    }
}
