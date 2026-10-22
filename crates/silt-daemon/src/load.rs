//! Loading one family from the store and nothing else, as a hook capturing
//! without the daemon does (ADR 0018), and saving its snapshots.

use silt_git::Repo;
use silt_log::{BaseText, Delta, Family, SNAPSHOT_FORMAT};
use silt_store::{Snapshot, Store};
use silt_types::FamilyId;
use tracing::warn;

use crate::core::{CoreError, RepoBases};

/// Deltas recorded since a family's last snapshot before it gets a new one.
pub(crate) const SNAPSHOT_EVERY: usize = 1000;

/// A family as the store holds it.
pub(crate) enum Loaded {
    /// With how much of its log its stored snapshot covers.
    Projected { family: Box<Family>, covered: usize },
    /// No clone was there to supply the git texts it starts from.
    Raw(Vec<Delta>),
}

pub(crate) async fn family(store: &Store, family_id: FamilyId, repo: Option<&Repo>) -> Result<Loaded, CoreError> {
    let deltas = store.deltas(family_id).await?;
    let Some(repo) = repo else {
        return Ok(Loaded::Raw(deltas));
    };
    let snapshot = store.snapshot(family_id).await?;
    let (family, covered) = replay(snapshot.as_ref(), deltas, &mut RepoBases(repo));
    Ok(Loaded::Projected { family: Box::new(family), covered })
}

/// Saves the family's projection; returns how much of its log it covers.
pub(crate) async fn save_snapshot(store: &Store, family_id: FamilyId, family: &mut Family) -> Result<usize, CoreError> {
    let covers = family.log().len();
    let data = family.snapshot().map_err(CoreError::Encode)?;
    store.put_snapshot(family_id, &Snapshot { format: SNAPSHOT_FORMAT, covers: covers as u64, data }).await?;
    Ok(covers)
}

/// A family rebuilt from its stored deltas, starting from `snapshot` when it
/// is a snapshot of them, and how much of its log that snapshot covers.
pub(crate) fn replay(snapshot: Option<&Snapshot>, mut stored: Vec<Delta>, bases: &mut dyn BaseText) -> (Family, usize) {
    let restored = snapshot.filter(|snapshot| snapshot.format == SNAPSHOT_FORMAT).and_then(|snapshot| {
        Family::from_snapshot(&snapshot.data, &mut stored)
            .inspect_err(|error| warn!(%error, "replaying the family instead of reading its snapshot"))
            .ok()
    });
    let mut family = restored.unwrap_or_default();
    let covered = family.log().len();
    for delta in stored {
        if let Err(error) = family.apply(delta, bases) {
            warn!(%error, "skipping a delta that no longer applies");
        }
    }
    (family, covered)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use silt_crdt::{InodeId, InodeKind, Parent, TextOp};
    use silt_log::{Author, Content, FileView, Op};
    use silt_types::{ActorId, ThreadId};

    use super::*;

    /// A file created and then typed into, one character per delta.
    fn typed(characters: usize) -> Vec<Delta> {
        let thread = ThreadId(1);
        let mut family = Family::new();
        let mut stored = Vec::new();
        let mut write = |family: &mut Family, op: Op| {
            let delta = family.delta(ActorId(1), thread, Author::User, 0, family.thread_head(thread), op);
            family.apply(delta.clone(), &mut HashMap::new()).expect("applies");
            stored.push(delta);
        };
        let create = Op::Create {
            kind: InodeKind::File,
            parent: Parent::Root,
            name: "a".into(),
            content: Content::Empty,
            executable: false,
        };
        write(&mut family, create);
        let inode = InodeId(family.log()[0].span.first());
        for pos in 0..characters {
            write(&mut family, Op::Text { inode, op: TextOp::Insert { pos, content: "x".into() } });
        }
        stored
    }

    fn text(family: &Family) -> Option<FileView> {
        family.file(InodeId(family.log()[0].span.first()))
    }

    #[test]
    fn a_family_starts_from_its_snapshot_and_applies_what_came_after() {
        let stored = typed(20);
        let (mut partway, _) = replay(None, stored[..12].to_vec(), &mut HashMap::new());
        let snapshot = Snapshot { format: SNAPSHOT_FORMAT, covers: 12, data: partway.snapshot().expect("encodes") };
        let (restored, covered) = replay(Some(&snapshot), stored.clone(), &mut HashMap::new());
        let (replayed, nothing) = replay(None, stored, &mut HashMap::new());
        assert_eq!((covered, nothing), (12, 0));
        assert_eq!(restored.log(), replayed.log());
        assert_eq!(text(&restored), Some(FileView::Text("x".repeat(20))));
    }

    #[test]
    fn a_snapshot_that_does_not_fit_is_replayed_instead() {
        let stored = typed(3);
        let (mut other, _) = replay(None, typed(5), &mut HashMap::new());
        let foreign = Snapshot { format: SNAPSHOT_FORMAT, covers: 6, data: other.snapshot().expect("encodes") };
        let (mut partway, _) = replay(None, stored[..2].to_vec(), &mut HashMap::new());
        let later_format = Snapshot { format: SNAPSHOT_FORMAT + 1, covers: 2, data: partway.snapshot().expect("encodes") };
        let garbage = Snapshot { format: SNAPSHOT_FORMAT, covers: 2, data: b"not a snapshot".to_vec() };
        for snapshot in [foreign, later_format, garbage] {
            let (family, covered) = replay(Some(&snapshot), stored.clone(), &mut HashMap::new());
            assert_eq!(covered, 0);
            assert_eq!(text(&family), Some(FileView::Text("xxx".into())));
        }
    }
}
