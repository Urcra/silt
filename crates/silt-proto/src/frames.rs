//! Frame keys (ADR 0074): an HTML artifact's frame loads from `/f/<key>/`,
//! outside the session's or the token's check, as a frame has neither. A
//! key opens one version of one artifact, and lasts an hour from when it
//! was last used; both serving modes keep them in memory alone.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long a frame key works.
pub const FRAME_TTL: Duration = Duration::from_secs(60 * 60);
/// The most keys kept; the oldest go first.
const FRAME_KEYS: usize = 512;

/// What each frame key opens.
#[derive(Debug)]
pub struct FrameKeys<G> {
    grants: HashMap<String, (G, Instant)>,
}

impl<G> Default for FrameKeys<G> {
    fn default() -> Self {
        Self { grants: HashMap::new() }
    }
}

impl<G: Clone> FrameKeys<G> {
    /// Keeps `key`, made of random bits by the caller, for `grant`.
    pub fn insert(&mut self, key: String, grant: G) {
        self.insert_at(key, grant, Instant::now());
    }

    fn insert_at(&mut self, key: String, grant: G, now: Instant) {
        self.grants.retain(|_, (_, made)| now.duration_since(*made) < FRAME_TTL);
        while self.grants.len() >= FRAME_KEYS {
            let Some(oldest) = self.grants.iter().min_by_key(|(_, (_, made))| *made).map(|(key, _)| key.clone()) else {
                break;
            };
            self.grants.remove(&oldest);
        }
        self.grants.insert(key, (grant, now));
    }

    /// What `key` opens, while it works. Using it starts its hour again, so
    /// an open frame keeps loading its files.
    pub fn get(&mut self, key: &str) -> Option<G> {
        self.get_at(key, Instant::now())
    }

    fn get_at(&mut self, key: &str, now: Instant) -> Option<G> {
        if !is_key(key) {
            return None;
        }
        let (grant, used) = self.grants.get_mut(key).filter(|(_, used)| now.duration_since(*used) < FRAME_TTL)?;
        *used = now;
        Some(grant.clone())
    }
}

/// Whether `key` has a key's shape: 64 hex characters.
pub fn is_key(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: usize) -> String {
        format!("{n:064x}")
    }

    #[test]
    fn a_key_opens_its_grant_for_an_hour_and_nothing_else() {
        let mut keys = FrameKeys::default();
        let now = Instant::now();
        keys.insert_at(key(1), "first", now);
        assert_eq!(keys.get_at(&key(1), now + Duration::from_secs(60)), Some("first"));
        assert_eq!(keys.get_at(&key(1), now + FRAME_TTL), Some("first"), "using it started its hour again");
        assert_eq!(keys.get_at(&key(1), now + FRAME_TTL * 3), None, "it expires unused");
        assert_eq!(keys.get_at(&key(2), now), None);
        assert!(!is_key("../etc") && !is_key(&"g".repeat(64)) && is_key(&key(3)));
    }

    #[test]
    fn the_oldest_keys_go_when_there_are_too_many() {
        let mut keys = FrameKeys::default();
        let now = Instant::now();
        keys.insert_at(key(0), 0, now);
        for at in 1..=FRAME_KEYS {
            keys.insert_at(key(at), at, now + Duration::from_millis(at as u64));
        }
        assert_eq!(keys.grants.len(), FRAME_KEYS);
        assert_eq!(keys.get_at(&key(0), now + Duration::from_secs(1)), None);
    }
}
