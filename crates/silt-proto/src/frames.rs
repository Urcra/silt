//! Frame keys (ADR 0074): an HTML artifact's frame loads from `/f/<key>/`,
//! outside the session's or the token's check, as a frame has neither. A
//! key opens one version of one artifact, and lasts an hour from when it
//! was last used; both serving modes keep them in memory alone.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long a frame key works.
pub const FRAME_TTL: Duration = Duration::from_secs(60 * 60);
/// The most keys kept for one person, and for everyone; past either, the
/// oldest go first, the person's own before anyone else's, so nobody's
/// asking for frames closes another's.
const PERSONS_KEYS: usize = 128;
const FRAME_KEYS: usize = 4096;

/// What each frame key opens, and whose it is.
#[derive(Debug)]
pub struct FrameKeys<G> {
    grants: HashMap<String, (G, Instant, String)>,
}

impl<G> Default for FrameKeys<G> {
    fn default() -> Self {
        Self { grants: HashMap::new() }
    }
}

impl<G: Clone> FrameKeys<G> {
    /// Keeps `key`, made of random bits by the caller, for `grant`, asked
    /// for by `owner`.
    pub fn insert(&mut self, key: String, grant: G, owner: &str) {
        self.insert_at(key, grant, owner, Instant::now());
    }

    fn insert_at(&mut self, key: String, grant: G, owner: &str, now: Instant) {
        self.grants.retain(|_, (_, used, _)| now.duration_since(*used) < FRAME_TTL);
        let oldest = |grants: &HashMap<String, (G, Instant, String)>, mine: bool| {
            grants
                .iter()
                .filter(|(_, (_, _, whose))| !mine || whose == owner)
                .min_by_key(|(_, (_, used, _))| *used)
                .map(|(key, _)| key.clone())
        };
        while self.grants.values().filter(|(_, _, whose)| whose == owner).count() >= PERSONS_KEYS {
            let Some(oldest) = oldest(&self.grants, true) else { break };
            self.grants.remove(&oldest);
        }
        while self.grants.len() >= FRAME_KEYS {
            let Some(oldest) = oldest(&self.grants, false) else { break };
            self.grants.remove(&oldest);
        }
        self.grants.insert(key, (grant, now, owner.to_owned()));
    }

    /// What `key` opens, while it works. Using it starts its hour again, so
    /// an open frame keeps loading its files.
    pub fn get(&mut self, key: &str) -> Option<G> {
        self.get_at(key, Instant::now())
    }

    /// What `key` opens, as [`FrameKeys::get`] answers, and then `spend`
    /// applied to what's kept: a page's load takes what's for it alone, such
    /// as the viewer's storage, which then isn't held with the key.
    pub fn open(&mut self, key: &str, spend: impl FnOnce(&mut G)) -> Option<G> {
        let grant = self.get(key)?;
        if let Some((kept, _, _)) = self.grants.get_mut(key) {
            spend(kept);
        }
        Some(grant)
    }

    fn get_at(&mut self, key: &str, now: Instant) -> Option<G> {
        if !is_key(key) {
            return None;
        }
        let (grant, used, _) = self.grants.get_mut(key).filter(|(_, used, _)| now.duration_since(*used) < FRAME_TTL)?;
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

    #[test]
    fn a_frames_first_load_takes_what_was_kept_for_it_alone() {
        let mut keys = FrameKeys::default();
        let key = "a".repeat(64);
        keys.insert(key.clone(), (1, Some("storage")), "me");
        assert_eq!(keys.open(&key, |grant| grant.1 = None), Some((1, Some("storage"))));
        assert_eq!(keys.get(&key), Some((1, None)), "the key still opens the page, without the storage");
    }

    fn key(n: usize) -> String {
        format!("{n:064x}")
    }

    #[test]
    fn a_key_opens_its_grant_for_an_hour_and_nothing_else() {
        let mut keys = FrameKeys::default();
        let now = Instant::now();
        keys.insert_at(key(1), "first", "ada", now);
        assert_eq!(keys.get_at(&key(1), now + Duration::from_secs(60)), Some("first"));
        assert_eq!(keys.get_at(&key(1), now + FRAME_TTL), Some("first"), "using it started its hour again");
        assert_eq!(keys.get_at(&key(1), now + FRAME_TTL * 3), None, "it expires unused");
        assert_eq!(keys.get_at(&key(2), now), None);
        assert!(!is_key("../etc") && !is_key(&"g".repeat(64)) && is_key(&key(3)));
    }

    #[test]
    fn one_persons_keys_never_push_out_anothers() {
        let mut keys = FrameKeys::default();
        let now = Instant::now();
        keys.insert_at(key(0), 0, "ada", now);
        for at in 1..=PERSONS_KEYS * 3 {
            keys.insert_at(key(at), at, "mallory", now + Duration::from_millis(at as u64));
        }
        assert_eq!(keys.grants.len(), PERSONS_KEYS + 1);
        let later = now + Duration::from_secs(1);
        assert_eq!(keys.get_at(&key(0), later), Some(0), "ada's key stayed");
        assert_eq!(keys.get_at(&key(1), later), None, "mallory's oldest went");
        assert_eq!(keys.get_at(&key(PERSONS_KEYS * 3), later), Some(PERSONS_KEYS * 3));
    }
}
