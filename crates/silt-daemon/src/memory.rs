//! What the daemon holds of the store, kept to what is in use (ADR 0115).

use std::collections::HashSet;
use std::time::{Duration, Instant, SystemTime};

use silt_log::{Family, snapshot_files};
use silt_types::{BlobHash, FamilyId};
use tracing::{debug, warn};

use crate::core::Core;

/// How long a family's file may go unused before it's let go of.
pub(crate) const UNUSED: Duration = Duration::from_secs(5 * 60);

/// How often files' states no snapshot names are looked for.
const SWEEP_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// How old a file's state no snapshot names must be before it's removed: a
/// family loaded from an older snapshot, here or in a hook's process, may
/// still read it.
const SWEEP_GRACE: Duration = Duration::from_secs(60 * 60);

/// When the daemon last let go of what was unused, and last swept.
#[derive(Debug)]
pub(crate) struct Kept {
    released: Instant,
    swept: Option<Instant>,
}

impl Default for Kept {
    fn default() -> Self {
        Self { released: Instant::now(), swept: None }
    }
}

impl Core {
    /// Lets go of the files unused for a while, replays the families whose
    /// stored files can't be read, and sweeps the files' states now and then.
    pub(crate) async fn keep_memory(&mut self) {
        let unreadable: Vec<FamilyId> =
            self.families.iter().filter(|(_, family)| family.files_unreadable()).map(|(id, _)| *id).collect();
        for family in unreadable {
            warn!(%family, "replaying a family whose stored files can't be read");
            if let Err(error) = self.reload_family(family).await {
                warn!(%error, %family, "could not replay the family");
            }
        }
        if self.kept.released.elapsed() >= UNUSED {
            self.kept.released = Instant::now();
            let released: usize = self.families.values_mut().map(Family::release_files).sum();
            if released > 0 {
                debug!(released, "let go of files unused for a while");
            }
        }
        if self.resident && self.kept.swept.is_none_or(|swept| swept.elapsed() >= SWEEP_EVERY) {
            self.kept.swept = Some(Instant::now());
            self.sweep_file_states().await;
        }
    }

    /// Removes the files' states that no snapshot and no family here names.
    async fn sweep_file_states(&self) {
        let mut named: HashSet<BlobHash> = self.families.values().flat_map(Family::stored_files).collect();
        let families: HashSet<FamilyId> = self.threads.values().map(|record| record.family).collect();
        for family in families {
            match self.store.snapshot(family).await {
                Ok(Some(snapshot)) => {
                    if let Ok(hashes) = snapshot_files(&snapshot.data, snapshot.format) {
                        named.extend(hashes);
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    warn!(%error, %family, "not sweeping files' states: a snapshot couldn't be read");
                    return;
                }
            }
        }
        let stored = match self.store.files().list().await {
            Ok(stored) => stored,
            Err(error) => {
                warn!(%error, "could not list files' states");
                return;
            }
        };
        let now = SystemTime::now();
        let mut removed = 0;
        for (hash, written) in stored {
            if !named.contains(&hash)
                && now.duration_since(written).is_ok_and(|age| age > SWEEP_GRACE)
                && self.store.files().remove(hash).await.is_ok()
            {
                removed += 1;
            }
        }
        if removed > 0 {
            debug!(removed, "removed files' states no snapshot names");
        }
    }
}
