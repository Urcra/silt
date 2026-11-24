//! A family's log, whose first deltas a snapshot may leave unread until
//! something reads the log. A hook capturing in its own process reads only
//! the projection, and decoding the 100,000 deltas a snapshot covers took
//! it longer than everything else it did (ADR 0034).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use crate::delta::Delta;

/// Reads the deltas a snapshot covers, in log order.
pub type ReadCovered = Box<dyn FnOnce() -> Vec<Delta> + Send>;

pub(super) struct Log {
    /// The whole log, once it is known.
    whole: OnceLock<Vec<Delta>>,
    /// Until then: how to read the deltas a snapshot covers.
    unread: Mutex<Option<ReadCovered>>,
    /// How many deltas `unread` reads.
    covered: usize,
    /// Those applied after them, until the log is read whole: what a reader
    /// of what's new reads without reading the rest (ADR 0117).
    after: Vec<Delta>,
    /// Whether something read the whole log since [`Log::idle`] last asked.
    read: AtomicBool,
}

impl Log {
    /// A log whose first `covered` deltas are read only when something
    /// reads the log.
    pub(super) fn unread(covered: usize, read: ReadCovered) -> Self {
        Self { whole: OnceLock::new(), unread: Mutex::new(Some(read)), covered, after: Vec::new(), read: AtomicBool::new(false) }
    }

    pub(super) fn all(&self) -> &[Delta] {
        self.read.store(true, Ordering::Relaxed);
        self.whole.get_or_init(|| {
            let read = self.unread.lock().unwrap_or_else(PoisonError::into_inner).take().expect("unread until the log is known");
            let mut deltas = read();
            assert_eq!(deltas.len(), self.covered, "the deltas a snapshot covers read back as many as it covers");
            deltas.extend(self.after.iter().cloned());
            deltas
        })
    }

    pub(super) fn len(&self) -> usize {
        self.whole.get().map_or(self.covered + self.after.len(), Vec::len)
    }

    /// The deltas from `from` on, if they're known without reading the log
    /// whole.
    pub(super) fn since(&self, from: usize) -> Option<&[Delta]> {
        match self.whole.get() {
            Some(whole) => whole.get(from..),
            None => self.after.get(from.checked_sub(self.covered)?..),
        }
    }

    /// The deltas known without reading the log whole, and the position of
    /// the first of them.
    pub(super) fn recent(&self) -> (usize, &[Delta]) {
        match self.whole.get() {
            Some(whole) => (0, whole),
            None => (self.covered, &self.after),
        }
    }

    pub(super) fn push(&mut self, delta: Delta) {
        match self.whole.get_mut() {
            Some(whole) => {
                whole.push(delta);
                // Copied into the whole log when it was read.
                if !self.after.is_empty() {
                    self.after = Vec::new();
                }
            }
            None => self.after.push(delta),
        }
    }

    pub(super) fn is_read(&self) -> bool {
        self.whole.get().is_some()
    }

    /// Whether the log is read whole and nothing read it since the last
    /// time this asked.
    pub(super) fn idle(&mut self) -> bool {
        let read = std::mem::replace(self.read.get_mut(), false);
        self.whole.get().is_some() && !read
    }

    /// Lets go of the log, to read with `read` when next read. `read` must
    /// give back every delta the log holds now, in order.
    pub(super) fn release(&mut self, read: ReadCovered) {
        *self = Self::unread(self.len(), read);
    }
}

impl From<Vec<Delta>> for Log {
    fn from(deltas: Vec<Delta>) -> Self {
        // Just made, as good as just read.
        Self {
            whole: OnceLock::from(deltas),
            unread: Mutex::new(None),
            covered: 0,
            after: Vec::new(),
            read: AtomicBool::new(true),
        }
    }
}

impl Default for Log {
    fn default() -> Self {
        Self::from(Vec::new())
    }
}

impl Clone for Log {
    fn clone(&self) -> Self {
        Self::from(self.all().to_vec())
    }
}

impl std::fmt::Debug for Log {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Log").field("len", &self.len()).field("read", &self.whole.get().is_some()).finish()
    }
}

#[cfg(test)]
mod tests {
    use silt_types::{ActorId, Frontier, ThreadId};

    use super::*;
    use crate::{Author, ConvOp, Family, Op};

    fn titled(family: &Family, title: &str) -> Delta {
        family.delta(ActorId(1), ThreadId(1), Author::User, 0, Frontier::root(), Op::Conv(ConvOp::Title(title.into())))
    }

    #[test]
    fn covered_deltas_are_read_once_and_only_when_the_log_is() {
        let family = Family::new();
        let (covered, after) = (titled(&family, "covered"), titled(&family, "after"));
        let reads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = std::sync::Arc::clone(&reads);
        let read_back = covered.clone();
        let mut log = Log::unread(
            1,
            Box::new(move || {
                counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                vec![read_back]
            }),
        );
        log.push(after.clone());
        assert_eq!(log.len(), 2);
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 0, "knowing the length reads nothing");
        assert_eq!(log.all(), [covered.clone(), after.clone()]);
        log.push(after.clone());
        assert_eq!(log.all(), [covered, after.clone(), after]);
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn a_log_unread_since_last_asked_is_let_go_of_and_read_back() {
        let family = Family::new();
        let deltas = vec![titled(&family, "one"), titled(&family, "two")];
        let mut log = Log::from(deltas.clone());
        assert!(!log.idle(), "just made");
        assert!(log.idle());
        log.all();
        assert!(!log.idle(), "read since");
        assert!(log.idle());
        let read_back = deltas.clone();
        log.release(Box::new(move || read_back));
        assert!(!log.idle(), "let go of, so not read whole");
        assert_eq!(log.len(), 2);
        assert_eq!(log.all(), deltas);
    }

    #[test]
    fn what_came_after_the_covered_deltas_is_known_without_reading_them() {
        let family = Family::new();
        let (covered, after) = (titled(&family, "covered"), titled(&family, "after"));
        let read_back = covered.clone();
        let mut log = Log::unread(1, Box::new(move || vec![read_back]));
        log.push(after.clone());
        assert_eq!(log.since(1), Some(std::slice::from_ref(&after)));
        assert_eq!(log.since(2), Some(&[][..]));
        assert_eq!(log.since(0), None, "the covered delta isn't known yet");
        assert_eq!(log.recent(), (1, std::slice::from_ref(&after)));
        assert!(!log.is_read());
        assert_eq!(log.all(), [covered.clone(), after.clone()]);
        assert_eq!(log.since(0), Some(&[covered, after][..]), "read whole, all of it is");
    }
}
