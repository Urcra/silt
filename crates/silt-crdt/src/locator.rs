use serde::{Deserialize, Serialize};
use smallvec::{SmallVec, smallvec};

/// A dense, totally ordered key: between any two locators another one can be
/// generated. Fragments are ordered by their locators, so a character's place
/// in the document can be compared and sought without counting positions.
///
/// Locators are local to one replica's index and never leave it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
pub struct Locator(SmallVec<[u64; 4]>);

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
            // Appends at the end take a small step so repeated appends stay
            // short; anything between two fragments takes the midpoint so
            // repeated splits of the same gap stay short too.
            let gap = high.saturating_sub(low);
            let digit = low + if appending { gap >> 48 } else { gap / 2 };
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
    fn appending_keeps_locators_short() {
        let mut last = Locator::min();
        for _ in 0..100_000 {
            last = Locator::between(&last, &Locator::max());
        }
        assert!(last.0.len() <= 2, "{last:?}");
    }
}
