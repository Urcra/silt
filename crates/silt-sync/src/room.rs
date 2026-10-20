use std::collections::HashMap;

use silt_crdt::{CausalError, CausalGraph};
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
        let mut graph = self.graph.clone();
        let mut fresh = Vec::new();
        for delta in batch {
            let id = delta.span.first();
            if let Some(index) = self.by_first.get(&id) {
                if self.deltas[*index] != delta {
                    return Err(PushError::Conflict(id));
                }
                continue;
            }
            if fresh.iter().any(|known: &Delta| known.span.first() == id) {
                continue;
            }
            graph.add(delta.span, &delta.parents).map_err(|error| match error {
                CausalError::UnknownParent(_) => PushError::MissingParents(id),
                CausalError::ConflictingSpan { .. } => PushError::Conflict(id),
                other => PushError::Invalid {
                    id,
                    reason: other.to_string(),
                },
            })?;
            fresh.push(delta);
        }
        let accepted = fresh.len();
        self.graph = graph;
        for delta in fresh {
            self.by_first.insert(delta.span.first(), self.deltas.len());
            self.deltas.push(delta);
        }
        Ok(PushOutcome {
            accepted,
            seq: self.seq(),
        })
    }

    /// The `seq` of the delta whose first id is `id`.
    pub fn position(&self, id: DeltaId) -> Option<u64> {
        self.by_first.get(&id).map(|index| *index as u64 + 1)
    }

    /// Deltas after `cursor`, with their `seq`. `last` is the id of the
    /// delta the client saw at `cursor`, which must still be there.
    pub fn pull(&self, cursor: u64, last: Option<DeltaId>, limit: usize) -> Result<Vec<(u64, Delta)>, PullError> {
        let matches = match (cursor, last) {
            (0, _) => true,
            (_, None) => false,
            (cursor, Some(last)) => self.position(last) == Some(cursor),
        };
        if !matches {
            return Err(PullError::UnknownCursor { cursor });
        }
        let start = usize::try_from(cursor).unwrap_or(usize::MAX);
        Ok(self.deltas[start..]
            .iter()
            .take(limit)
            .zip(cursor + 1..)
            .map(|(delta, seq)| (seq, delta.clone()))
            .collect())
    }
}
