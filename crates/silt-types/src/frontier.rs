use serde::{Deserialize, Serialize};
use smallvec::{SmallVec, smallvec};

use crate::{DeltaId, GitOid};

/// The set of latest ids that a version includes.
///
/// Stored sorted and deduplicated. Whether the ids form an antichain (none is
/// an ancestor of another) depends on the causal graph, which this type cannot
/// see; the causal graph is responsible for producing minimal frontiers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(from = "SmallVec<[DeltaId; 2]>")]
pub struct Frontier(SmallVec<[DeltaId; 2]>);

/// Frontiers arrive from other people too, so a received one is sorted and
/// deduplicated like any other.
impl From<SmallVec<[DeltaId; 2]>> for Frontier {
    fn from(ids: SmallVec<[DeltaId; 2]>) -> Self {
        ids.into_iter().collect()
    }
}

impl Frontier {
    /// The version before any delta.
    pub fn root() -> Self {
        Self::default()
    }

    pub fn single(id: DeltaId) -> Self {
        Self(smallvec![id])
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn ids(&self) -> &[DeltaId] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn contains(&self, id: &DeltaId) -> bool {
        self.0.binary_search(id).is_ok()
    }

    pub fn insert(&mut self, id: DeltaId) {
        if let Err(index) = self.0.binary_search(&id) {
            self.0.insert(index, id);
        }
    }

    pub fn remove(&mut self, id: &DeltaId) -> bool {
        match self.0.binary_search(id) {
            Ok(index) => {
                self.0.remove(index);
                true
            }
            Err(_) => false,
        }
    }
}

impl FromIterator<DeltaId> for Frontier {
    fn from_iter<I: IntoIterator<Item = DeltaId>>(iter: I) -> Self {
        let mut ids: SmallVec<[DeltaId; 2]> = iter.into_iter().collect();
        ids.sort_unstable();
        ids.dedup();
        Self(ids)
    }
}

impl From<DeltaId> for Frontier {
    fn from(id: DeltaId) -> Self {
        Self::single(id)
    }
}

/// The ids comma-separated, as a permalink's `at` carries a version.
impl std::fmt::Display for Frontier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (index, id) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str(",")?;
            }
            write!(f, "{id}")?;
        }
        Ok(())
    }
}

impl std::str::FromStr for Frontier {
    type Err = crate::ParseIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Ok(Self::root());
        }
        s.split(',').map(str::parse).collect()
    }
}

/// A point in a history: either a git object or a set of deltas.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Version {
    Git(GitOid),
    Delta(Frontier),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActorId, Lamport};

    fn id(actor: u128, lamport: u64) -> DeltaId {
        DeltaId::new(ActorId(actor), Lamport(lamport))
    }

    #[test]
    fn collects_sorted_and_deduplicated() {
        let frontier: Frontier = [id(2, 1), id(1, 5), id(2, 1)].into_iter().collect();
        assert_eq!(frontier.ids(), &[id(1, 5), id(2, 1)]);
    }

    #[test]
    fn insert_and_remove_keep_order() {
        let mut frontier = Frontier::single(id(3, 3));
        frontier.insert(id(1, 1));
        frontier.insert(id(1, 1));
        assert_eq!(frontier.ids(), &[id(1, 1), id(3, 3)]);
        assert!(frontier.remove(&id(1, 1)));
        assert!(!frontier.remove(&id(1, 1)));
        assert_eq!(frontier, Frontier::single(id(3, 3)));
    }

    #[test]
    fn root_is_empty() {
        assert!(Frontier::root().is_root());
        assert!(!Frontier::single(id(1, 1)).is_root());
    }

    #[test]
    fn reads_back_as_it_is_written() {
        let frontier: Frontier = [id(1, 2), id(7, 9)].into_iter().collect();
        assert_eq!(frontier.to_string().parse::<Frontier>().expect("parses"), frontier);
        assert_eq!("".parse::<Frontier>().expect("parses"), Frontier::root());
        assert!("nonsense".parse::<Frontier>().is_err());
    }

    #[test]
    fn serializes_as_a_plain_list() {
        let frontier: Frontier = [id(1, 2)].into_iter().collect();
        let json = serde_json::to_string(&frontier).expect("serializes");
        assert_eq!(serde_json::from_str::<Frontier>(&json).expect("parses"), frontier);
    }
}
