//! A family's log, whose first deltas a snapshot may leave unread until
//! something reads the log. A hook capturing in its own process reads only
//! the projection, and decoding the 100,000 deltas a snapshot covers took
//! it longer than everything else it did (ADR 0034).

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
        Self { whole: OnceLock::new(), unread: Mutex::new(Some(Unread { covered, read, after: Vec::new() })) }
    }

    pub(super) fn all(&self) -> &[Delta] {
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

impl From<Vec<Delta>> for Log {
    fn from(deltas: Vec<Delta>) -> Self {
        Self { whole: OnceLock::from(deltas), unread: Mutex::new(None) }
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
}
