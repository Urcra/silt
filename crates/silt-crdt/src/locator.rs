use serde::{Deserialize, Serialize};
use smallvec::{SmallVec, smallvec};

/// A dense, totally ordered key: between any two locators another one can be
/// generated. Fragments are ordered by their locators, so a character's place
/// in the document can be compared and sought without counting positions.
///
/// Locators are local to one replica's index and never leave it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
pub struct Locator(SmallVec<[u64; 4]>);

/// The largest step between two fragments.
const STEP: u64 = 1 << 32;

impl Locator {
    pub fn min() -> Self {
        Self(smallvec![u64::MIN])
    }

    pub fn max() -> Self {
        Self(smallvec![u64::MAX])
    }

    /// A locator strictly between `lhs` and `rhs`, which must satisfy `lhs < rhs`.
    pub fn between(lhs: &Self, rhs: &Self) -> Self {
        let lhs_digits = lhs.0.iter().copied().chain(std::iter::repeat(u64::MIN));
        let rhs_digits = rhs.0.iter().copied().chain(std::iter::repeat(u64::MAX));
        let appending = *rhs == Self::max();
        let mut digits = SmallVec::new();
        for (low, high) in lhs_digits.zip(rhs_digits) {
            // Steps from the left stay small, so text typed on, each piece
            // after the last one, fits many times in one gap: appends at the
            // end step by a sliver of the gap, inserts between fragments by
            // at most `STEP`. Halving instead would add a digit every 64.
            let gap = high.saturating_sub(low);
            let digit = low + if appending { gap >> 48 } else { (gap / 2).min(STEP) };
            digits.push(digit);
            if digit > low {
                break;
            }
        }
        let locator = Self(digits);
        debug_assert!(lhs < &locator && &locator < rhs, "{lhs:?} < {locator:?} < {rhs:?}");
        locator
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn between_is_strictly_between() {
        let mut low = Locator::min();
        let high = Locator::max();
        for _ in 0..1000 {
            let middle = Locator::between(&low, &high);
            assert!(low < middle && middle < high);
            low = middle;
        }
        let mut high = Locator::max();
        let low = Locator::min();
        for _ in 0..1000 {
            let middle = Locator::between(&low, &high);
            assert!(low < middle && middle < high);
            high = middle;
        }
    }

    #[test]
    fn typing_on_before_a_fixed_neighbour_keeps_locators_short() {
        let after = Locator::between(&Locator::min(), &Locator::max());
        let mut last = Locator::min();
        for _ in 0..100_000 {
            last = Locator::between(&last, &after);
        }
        assert!(last.0.len() <= 2, "{} digits", last.0.len());
    }

    #[test]
    fn appending_keeps_locators_short() {
        let mut last = Locator::min();
        for _ in 0..100_000 {
            last = Locator::between(&last, &Locator::max());
        }
        assert!(last.0.len() <= 2, "{last:?}");
    }
}
