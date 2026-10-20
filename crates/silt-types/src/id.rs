use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// A replica that produces deltas: one checkout, daemon or server writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ActorId(pub u128);

impl ActorId {
    /// A distinct actor for one part of this replica's work, such as one
    /// thread. The same inputs always give the same actor.
    pub fn derive(self, salt: u128) -> Self {
        let mut state = self.0 ^ salt.rotate_left(64).wrapping_mul(0x9e37_79b9_7f4a_7c15_f39c_c060_5ced_c835);
        state ^= state >> 67;
        state = state.wrapping_mul(0xd6e8_feb8_6659_fd93_9e37_79b9_7f4a_7c15);
        state ^= state >> 61;
        Self(state)
    }
}

impl fmt::Display for ActorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid actor id {0:?}: expected 32 hex digits")]
pub struct ParseActorIdError(String);

impl FromStr for ActorId {
    type Err = ParseActorIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 32 {
            return Err(ParseActorIdError(s.to_owned()));
        }
        u128::from_str_radix(s, 16)
            .map(Self)
            .map_err(|_| ParseActorIdError(s.to_owned()))
    }
}

/// Position on a history's Lamport clock.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct Lamport(pub u64);

/// The highest lamport a delta may reach. Stores keep lamports as signed
/// 64-bit integers, and deltas come from other people, so the clock is
/// bounded rather than allowed to wrap.
pub const MAX_LAMPORT: u64 = i64::MAX as u64;

impl Lamport {
    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }

    pub fn offset(self, by: u64) -> Self {
        Self(self.0.saturating_add(by))
    }
}

impl fmt::Display for Lamport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Identity of a single unit of change.
///
/// Ordering compares the actor first, which is the tie-break used when
/// concurrent insertions land at the same place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DeltaId {
    pub actor: ActorId,
    pub lamport: Lamport,
}

impl DeltaId {
    pub fn new(actor: ActorId, lamport: Lamport) -> Self {
        Self { actor, lamport }
    }
}

impl fmt::Display for DeltaId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.actor, self.lamport)
    }
}

impl FromStr for DeltaId {
    type Err = ParseActorIdError;

    /// Parses the `actor.lamport` form produced by `Display`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let invalid = || ParseActorIdError(s.to_owned());
        let (actor, lamport) = s.split_once('.').ok_or_else(invalid)?;
        Ok(Self::new(actor.parse()?, Lamport(lamport.parse().map_err(|_| invalid())?)))
    }
}

/// A contiguous run of ids from one actor: `[start, start + len)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeltaSpan {
    pub actor: ActorId,
    pub start: Lamport,
    pub len: u64,
}

impl DeltaSpan {
    pub fn new(actor: ActorId, start: Lamport, len: u64) -> Self {
        Self { actor, start, len }
    }

    pub fn first(&self) -> DeltaId {
        DeltaId::new(self.actor, self.start)
    }

    /// The last id in the span. Spans are never empty when this is called.
    pub fn last(&self) -> DeltaId {
        debug_assert!(self.len > 0, "empty span has no last id");
        DeltaId::new(self.actor, self.start.offset(self.len.saturating_sub(1)))
    }

    /// Whether the span is non-empty and ends within the lamport clock.
    pub fn is_in_range(&self) -> bool {
        self.len > 0 && self.start.0.checked_add(self.len).is_some_and(|end| end <= MAX_LAMPORT)
    }

    /// One past the last lamport in the span.
    pub fn end(&self) -> Lamport {
        self.start.offset(self.len)
    }

    pub fn id_at(&self, offset: u64) -> DeltaId {
        debug_assert!(offset < self.len, "offset {offset} outside span of {}", self.len);
        DeltaId::new(self.actor, self.start.offset(offset))
    }

    pub fn contains(&self, id: DeltaId) -> bool {
        id.actor == self.actor && id.lamport >= self.start && id.lamport < self.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_id_round_trips_as_padded_hex() {
        let actor = ActorId(0xab);
        assert_eq!(actor.to_string(), "000000000000000000000000000000ab");
        assert_eq!(actor.to_string().parse::<ActorId>(), Ok(actor));
    }

    #[test]
    fn actor_id_rejects_wrong_length() {
        assert!("abc".parse::<ActorId>().is_err());
    }

    #[test]
    fn actor_hex_order_matches_numeric_order() {
        let low = ActorId(9);
        let high = ActorId(10);
        assert!(low < high);
        assert!(low.to_string() < high.to_string());
    }

    #[test]
    fn delta_ids_order_by_actor_before_lamport() {
        let a = DeltaId::new(ActorId(1), Lamport(100));
        let b = DeltaId::new(ActorId(2), Lamport(1));
        assert!(a < b);
    }

    #[test]
    fn delta_id_round_trips_as_text() {
        let id = DeltaId::new(ActorId(7), Lamport(42));
        assert_eq!(id.to_string().parse::<DeltaId>(), Ok(id));
        assert!("nonsense".parse::<DeltaId>().is_err());
    }

    #[test]
    fn span_covers_its_ids() {
        let span = DeltaSpan::new(ActorId(1), Lamport(5), 3);
        assert_eq!(span.first(), DeltaId::new(ActorId(1), Lamport(5)));
        assert_eq!(span.last(), DeltaId::new(ActorId(1), Lamport(7)));
        assert_eq!(span.end(), Lamport(8));
        assert!(span.contains(DeltaId::new(ActorId(1), Lamport(6))));
        assert!(!span.contains(DeltaId::new(ActorId(1), Lamport(8))));
        assert!(!span.contains(DeltaId::new(ActorId(2), Lamport(6))));
    }
}
