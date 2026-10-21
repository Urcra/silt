use std::collections::{HashMap, HashSet};

use silt_crdt::{Added, CausalError, CausalGraph};
use silt_log::Delta;
use silt_types::DeltaId;

/// The server's copy of one family: every delta in arrival order.
#[derive(Debug, Clone, Default)]
pub struct Room {
    generation: u64,
    graph: CausalGraph,
    deltas: Vec<Delta>,
    by_first: HashMap<DeltaId, usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PushOutcome {
    pub accepted: usize,
    pub seq: u64,
}

/// A checked batch, ready to store. Its ids are already in the room's
/// graph; dropping it without [`Prepared::commit`] takes them out again.
#[derive(Debug)]
pub struct Prepared<'room> {
    room: &'room mut Room,
    fresh: Vec<Delta>,
}

impl Prepared<'_> {
    /// The deltas the room doesn't have yet.
    pub fn fresh(&self) -> &[Delta] {
        &self.fresh
    }

    /// Stores the batch, numbered from the room's `seq() + 1`.
    pub fn commit(mut self) -> PushOutcome {
        let fresh = std::mem::take(&mut self.fresh);
        let room = &mut *self.room;
        room.graph.keep();
        let accepted = fresh.len();
        for delta in fresh {
            room.by_first.insert(delta.span.first(), room.deltas.len());
            room.deltas.push(delta);
        }
        PushOutcome {
            accepted,
            seq: room.seq(),
        }
    }
}

impl Drop for Prepared<'_> {
    fn drop(&mut self) {
        self.room.graph.roll_back();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PushError {
    #[error("delta {0} is already stored with different content")]
    Conflict(DeltaId),
    #[error("delta {0} depends on deltas the server doesn't have")]
    MissingParents(DeltaId),
    #[error("delta {id} is invalid: {reason}")]
    Invalid { id: DeltaId, reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PullError {
    /// The server doesn't have the history the cursor points into: it was
    /// restored from an older state, possibly growing a different history
    /// since.
    #[error("cursor {cursor} does not match the server's history")]
    UnknownCursor { cursor: u64 },
}

impl Room {
    /// A room for a server process identified by `generation`, a value
    /// chosen anew every time the server starts. A client that sees it change
    /// syncs everything again, which covers the server having been restored
    /// from an older state.
    pub fn new(generation: u64) -> Self {
        Self {
            generation,
            ..Self::default()
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The same room after a server restart.
    pub fn restarted(&self, generation: u64) -> Self {
        Self {
            generation,
            ..self.clone()
        }
    }

    /// The `seq` of the last delta; deltas are numbered from 1.
    pub fn seq(&self) -> u64 {
        self.deltas.len() as u64
    }

    pub fn graph(&self) -> &CausalGraph {
        &self.graph
    }

    pub fn deltas(&self) -> &[Delta] {
        &self.deltas
    }

    /// Stores new deltas, skipping known ones. The batch is checked whole
    /// first, so a rejected batch changes nothing.
    pub fn push(&mut self, batch: Vec<Delta>) -> Result<PushOutcome, PushError> {
        Ok(self.prepare(batch)?.commit())
    }

    /// Checks a batch: the new deltas, in the order [`Prepared::commit`]
    /// will number them. A rejected batch changes nothing.
    pub fn prepare(&mut self, batch: Vec<Delta>) -> Result<Prepared<'_>, PushError> {
        self.graph.begin();
        let mut prepared = Prepared {
            room: self,
            fresh: Vec::new(),
        };
        let mut fresh_ids = HashSet::new();
        for delta in batch {
            let id = delta.span.first();
            if let Some(index) = prepared.room.by_first.get(&id) {
                if prepared.room.deltas[*index] != delta {
                    return Err(PushError::Conflict(id));
                }
                continue;
            }
            if !fresh_ids.insert(id) {
                continue;
            }
            // Every replica would refuse a malformed delta, so storing it
            // would only wedge their sync (ADR 0016).
            delta.check_shape().map_err(|error| PushError::Invalid {
                id,
                reason: error.to_string(),
            })?;
            let added = prepared.room.graph.add(delta.span, &delta.parents).map_err(|error| match error {
                CausalError::UnknownParent(_) => PushError::MissingParents(id),
                CausalError::ConflictingSpan { .. } => PushError::Conflict(id),
                other => PushError::Invalid {
                    id,
                    reason: other.to_string(),
                },
            })?;
            // A delta that only partly covers ids stored under other deltas
            // would be dropped by clients, which index deltas by first id.
            if added != Added::New(delta.span) {
                return Err(PushError::Conflict(id));
            }
            prepared.fresh.push(delta);
        }
        Ok(prepared)
    }

    /// The `seq` of the delta whose first id is `id`.
    pub fn position(&self, id: DeltaId) -> Option<u64> {
        self.by_first.get(&id).map(|index| *index as u64 + 1)
    }

    /// Deltas after `cursor`, with their `seq`. `last` is the id of the
    /// delta the client saw at `cursor`, which must still be there.
    pub fn pull(&self, cursor: u64, last: Option<DeltaId>, limit: usize) -> Result<Vec<(u64, Delta)>, PullError> {
        Ok(self.after(cursor, last)?.take(limit).map(|(seq, delta)| (seq, delta.clone())).collect())
    }

    /// `pull` without copying: the deltas after `cursor`, with their `seq`.
    pub fn after(&self, cursor: u64, last: Option<DeltaId>) -> Result<impl Iterator<Item = (u64, &Delta)>, PullError> {
        let matches = match (cursor, last) {
            (0, _) => true,
            (_, None) => false,
            (cursor, Some(last)) => self.position(last) == Some(cursor),
        };
        if !matches {
            return Err(PullError::UnknownCursor { cursor });
        }
        let start = usize::try_from(cursor).map_or(self.deltas.len(), |start| start.min(self.deltas.len()));
        Ok((cursor + 1..).zip(&self.deltas[start..]))
    }
}
