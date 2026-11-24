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
    /// Until then: how to read the deltas a snapshot covers, and those
    /// applied after them.
    unread: Mutex<Option<Unread>>,
    /// Whether something read the whole log since [`Log::idle`] last asked.
    read: AtomicBool,
}

struct Unread {
    covered: usize,
    read: ReadCovered,
    after: Vec<Delta>,
}

impl Log {
    /// A log whose first `covered` deltas are read only when something
    /// reads the log.
    pub(super) fn unread(covered: usize, read: ReadCovered) -> Self {
        Self {
            whole: OnceLock::new(),
            unread: Mutex::new(Some(Unread { covered, read, after: Vec::new() })),
            read: AtomicBool::new(false),
        }
    }

    pub(super) fn all(&self) -> &[Delta] {
        self.read.store(true, Ordering::Relaxed);
        self.whole.get_or_init(|| {
            let Unread { covered, read, after } =
                self.unread.lock().unwrap_or_else(PoisonError::into_inner).take().expect("unread until the log is known");
            let mut deltas = read();
            assert_eq!(deltas.len(), covered, "the deltas a snapshot covers read back as many as it covers");
            deltas.extend(after);
            deltas
        })
    }

    pub(super) fn len(&self) -> usize {
        if let Some(whole) = self.whole.get() {
            return whole.len();
        }
        let unread = self.unread.lock().unwrap_or_else(PoisonError::into_inner);
        match unread.as_ref() {
            Some(unread) => unread.covered + unread.after.len(),
            // Being read on another thread.
            None => {
                drop(unread);
                self.all().len()
            }
        }
    }

    pub(super) fn push(&mut self, delta: Delta) {
        match self.whole.get_mut() {
            Some(whole) => whole.push(delta),
            None => {
                let unread = self.unread.get_mut().unwrap_or_else(PoisonError::into_inner);
                unread.as_mut().expect("unread until the log is known").after.push(delta);
            }
        }
    }
}

impl Log {
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
        Self { whole: OnceLock::from(deltas), unread: Mutex::new(None), read: AtomicBool::new(true) }
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
}
