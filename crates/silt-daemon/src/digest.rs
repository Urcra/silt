//! What the views across families read of a family the daemon doesn't
//! hold: its threads' summaries, merge requests, graph, commits and the
//! files its threads changed (ADR 0115). It is taken from the family by
//! the views' own code just before the family is let go of, so it says
//! what they would; a family changes only once held again.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use silt_log::MergeRequest;
use silt_proto::view::{GraphRow, MrSummary};
use silt_store::{StoredDigest, ThreadRecord};
use silt_types::{CloudSessionId, FamilyId, GitOid, ThreadId};

use crate::core::{Core, CoreError, Event};
use crate::protocol::CommitView;
use crate::views::LoggedSummary;

/// The encoding of [`Digest`]; one in another is taken again. Bump it when
/// what a view reads of a family changes.
pub(crate) const DIGEST_FORMAT: i64 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Digest {
    pub(crate) log_len: usize,
    pub(crate) threads: HashMap<ThreadId, ThreadDigest>,
    pub(crate) requests: Vec<(MergeRequest, MrSummary)>,
    pub(crate) graph: Vec<GraphRow>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ThreadDigest {
    pub(crate) summary: LoggedSummary,
    pub(crate) cloud: Option<CloudSessionId>,
    pub(crate) commits: Vec<CommitView>,
    /// Each file its own deltas changed, by its path, and when last.
    pub(crate) touched: Vec<(String, i64)>,
    /// The last commit it recorded, which a checkout's HEAD is checked
    /// against.
    pub(crate) commit: Option<GitOid>,
}

impl Digest {
    fn decode(stored: &StoredDigest) -> Option<Self> {
        if stored.format != DIGEST_FORMAT {
            return None;
        }
        serde_json::from_slice(&stored.data).ok()
    }
}

impl Core {
    /// The family's digest, while it says what the family holds: while the
    /// family isn't held, or is and its log hasn't grown since. A family
    /// held for a look then needn't read its log for the views.
    pub(crate) fn digested(&self, family: FamilyId) -> Option<&Digest> {
        let digest = self.digests.get(&family)?;
        if !self.families.is_loaded(&family) {
            return Some(digest);
        }
        let held = match self.families.loaded(&family) {
            Some(held) => held.log_len(),
            None => self.families.loaded_raw(&family).map_or(0, Vec::len),
        };
        (held == digest.log_len).then_some(digest)
    }

    /// The family for a view across families, once it has no digest to read
    /// instead: as held, without counting as a use, else loaded.
    pub(crate) fn viewed(&self, family: FamilyId) -> Option<&silt_log::Family> {
        self.families.loaded(&family).or_else(|| self.families.get(&family))
    }

    /// The Claude Code cloud session the thread records (ADR 0040).
    pub(crate) fn cloud_session_of(&self, record: &ThreadRecord) -> Option<CloudSessionId> {
        if let Some(digest) = self.digested(record.family) {
            return digest.threads.get(&record.id)?.cloud.clone();
        }
        self.viewed(record.family)?.thread(record.id)?.cloud_session().cloned()
    }

    /// The digest of a family held here, as the views read it now.
    pub(crate) fn digest_of(&self, family: FamilyId) -> Digest {
        let records: Vec<&ThreadRecord> = self.threads.values().filter(|record| record.family == family).collect();
        let summaries = self.summaries_in(family, &records);
        let projected = self.families.loaded(&family);
        let threads = records
            .iter()
            .map(|record| {
                let digest = ThreadDigest {
                    summary: summaries.get(&record.id).cloned().unwrap_or_default(),
                    cloud: projected.and_then(|projected| projected.thread(record.id)?.cloud_session().cloned()),
                    commits: if projected.is_some() { self.commits(record.id) } else { Vec::new() },
                    touched: if projected.is_some() { self.files_changed_by(record.id) } else { Vec::new() },
                    commit: self.recorded_commit(family, record.id),
                };
                (record.id, digest)
            })
            .collect();
        Digest {
            log_len: self.local_log_len(family),
            threads,
            requests: if projected.is_some() { self.requests_in(family) } else { Vec::new() },
            graph: if projected.is_some() { self.graph_in(family) } else { Vec::new() },
        }
    }

    /// Takes the digest of a family held here and stores it, as of the
    /// store's position `position` in the family.
    pub(crate) async fn keep_digest(&mut self, family: FamilyId, position: i64) -> Result<(), CoreError> {
        let digest = self.digest_of(family);
        let data = serde_json::to_vec(&digest).map_err(|error| CoreError::Invalid(error.to_string()))?;
        self.store.put_digest(family, &StoredDigest { format: DIGEST_FORMAT, position, data }).await?;
        self.digests.insert(family, digest);
        Ok(())
    }

    /// Reads the stored digests of `families`, and queues taking again those
    /// that don't say what the store holds now: in another format, or older
    /// than what was stored since, as by a hook while the daemon was down.
    /// They're taken a family at a time between other events, so the first
    /// start after an update, which takes every one, answers meanwhile.
    pub(crate) async fn read_digests(&mut self, families: &HashSet<FamilyId>) -> Result<(), CoreError> {
        let stored = self.store.digests().await?;
        let mut stale = Vec::new();
        for family in families {
            let position = self.positions.get(family).copied().unwrap_or(-1);
            match stored.get(family).filter(|digest| digest.position == position).and_then(Digest::decode) {
                Some(digest) => {
                    self.digests.insert(*family, digest);
                }
                None => stale.push(*family),
            }
        }
        stale.sort();
        self.stale_digests = stale;
        if !self.stale_digests.is_empty() {
            let _ = self.events.send(Event::Digests);
        }
        Ok(())
    }

    /// Takes the next stale digest, its family loaded for it and let go of
    /// again unless in use, and queues the one after.
    pub(crate) async fn take_stale_digest(&mut self) {
        let Some(family) = self.stale_digests.pop() else {
            return;
        };
        if self.stale_digests.is_empty() {
            self.digests_taken();
        } else {
            let _ = self.events.send(Event::Digests);
        }
        if self.digested(family).is_some() {
            return;
        }
        let held = self.families.is_loaded(&family);
        if self.families.get(&family).is_none() && self.families.raw(&family).is_none() {
            return;
        }
        let position = self.store.positions().await.ok().and_then(|positions| positions.get(&family).copied()).unwrap_or(-1);
        if held || self.pinned(family) {
            if let Err(error) = self.keep_digest(family, position).await {
                tracing::warn!(%error, %family, "could not keep the family's digest");
            }
        } else {
            self.unload(family, position).await;
        }
    }
}
