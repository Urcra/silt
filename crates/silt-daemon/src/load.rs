//! Loading one family from the store and nothing else, as a hook capturing
//! without the daemon does (ADR 0018), and saving its snapshots.

use std::sync::Arc;

use silt_git::Repo;
use silt_log::{BaseText, Delta, Family, ReadCovered, ReadFile, SNAPSHOT_FORMAT, is_readable_format, snapshot_files};
use silt_store::{Snapshot, Store, StoredDeltas};
use silt_types::FamilyId;
use tracing::warn;

use crate::core::{CoreError, RepoBases};

/// Deltas recorded since a family's last snapshot before it gets a new one.
pub(crate) const SNAPSHOT_EVERY: usize = 1000;

/// A family as the store holds it.
pub(crate) enum Loaded {
    /// With how much of its log its stored snapshot covers, and whether
    /// that snapshot is in an older format, to take again.
    Projected { family: Box<Family>, covered: usize, outdated: bool },
    /// No clone was there to supply the git texts it starts from.
    Raw(Vec<Delta>),
}

pub(crate) async fn family(store: &Store, family_id: FamilyId, repo: Option<&Repo>) -> Result<Loaded, CoreError> {
    let deltas = store.deltas(family_id).await?;
    let Some(repo) = repo else {
        return Ok(Loaded::Raw(deltas));
    };
    let snapshot = usable(store, store.snapshot(family_id).await?);
    let outdated = is_outdated(snapshot.as_ref());
    let (family, covered) = replay(snapshot.as_ref(), deltas, &mut RepoBases(repo), Some(file_reader(store)));
    Ok(Loaded::Projected { family: Box::new(family), covered, outdated })
}

/// Reads families' files' states from the store as they're first used.
pub(crate) fn file_reader(store: &Store) -> ReadFile {
    let files = store.files().clone();
    Arc::new(move |hash| files.read(hash).ok())
}

/// The snapshot, if it can be read: in a format this build reads, with
/// every file state it names stored.
fn usable(store: &Store, snapshot: Option<Snapshot>) -> Option<Snapshot> {
    let snapshot = snapshot.filter(|snapshot| is_readable_format(snapshot.format))?;
    match snapshot_files(&snapshot.data, snapshot.format) {
        Ok(hashes) if hashes.iter().all(|hash| store.files().contains(*hash)) => Some(snapshot),
        Ok(_) => {
            warn!("replaying the family instead of reading its snapshot, whose files aren't all stored");
            None
        }
        Err(error) => {
            warn!(%error, "replaying the family instead of reading its snapshot");
            None
        }
    }
}

fn is_outdated(snapshot: Option<&Snapshot>) -> bool {
    snapshot.is_some_and(|snapshot| snapshot.format != SNAPSHOT_FORMAT)
}

/// [`family`] for a hook capturing in its own process: the deltas its
/// snapshot covers are decoded only if something reads the log, which a
/// capture doesn't. Decoding them was most of a long family's load.
pub(crate) async fn family_unread(store: &Store, family_id: FamilyId, repo: Option<&Repo>) -> Result<Loaded, CoreError> {
    let Some(repo) = repo else {
        return family(store, family_id, None).await;
    };
    let snapshot = usable(store, store.snapshot(family_id).await?);
    let stored = Arc::new(store.stored_deltas(family_id).await?);
    if let Some(snapshot) = snapshot.as_ref()
        && let Some(loaded) = unread(store, snapshot, &stored, repo)?
    {
        return Ok(loaded);
    }
    let outdated = is_outdated(snapshot.as_ref());
    let (family, covered) =
        replay(snapshot.as_ref(), stored.decode(0..stored.len())?, &mut RepoBases(repo), Some(file_reader(store)));
    Ok(Loaded::Projected { family: Box::new(family), covered, outdated })
}

/// The family `snapshot` was taken of, with the deltas it covers left
/// unread, when they're the first of those stored, every one of them.
fn unread(store: &Store, snapshot: &Snapshot, stored: &Arc<StoredDeltas>, repo: &Repo) -> Result<Option<Loaded>, CoreError> {
    let covered = usize::try_from(snapshot.covers).unwrap_or(usize::MAX);
    if covered == 0 || covered > stored.len() {
        return Ok(None);
    }
    let Some(last) = stored.decode(covered - 1..covered)?.first().map(|delta| delta.span.first()) else {
        return Ok(None);
    };
    let reading = Arc::clone(stored);
    let read: ReadCovered =
        Box::new(move || reading.decode(0..covered).expect("deltas that were read from the store decode again"));
    let files = Some(file_reader(store));
    let Ok(mut family) = Family::from_snapshot_unread(&snapshot.data, snapshot.format, covered, last, read, files) else {
        return Ok(None);
    };
    apply_stored(&mut family, stored.decode(covered..stored.len())?, &mut RepoBases(repo));
    let outdated = is_outdated(Some(snapshot));
    Ok(Some(Loaded::Projected { family: Box::new(family), covered, outdated }))
}

/// Applies deltas read from the store, skipping those that no longer apply.
/// One lost delta leaves every later one built on it without a parent, so
/// what was skipped is told once, not for each of thousands.
pub(crate) fn apply_stored(family: &mut Family, stored: Vec<Delta>, bases: &mut dyn BaseText) {
    let mut skipped = 0;
    let mut first = None;
    for delta in stored {
        let id = delta.span.first();
        if let Err(error) = family.apply(delta, bases) {
            tracing::debug!(%error, %id, "skipping a delta that no longer applies");
            skipped += 1;
            first.get_or_insert((id, error));
        }
    }
    if let Some((id, error)) = first {
        warn!(skipped, first = %id, %error, "skipping stored deltas that no longer apply");
    }
}

/// Saves the family's projection, its files' new states first; returns how
/// much of its log it covers.
pub(crate) async fn save_snapshot(store: &Store, family_id: FamilyId, family: &mut Family) -> Result<usize, CoreError> {
    let covers = family.log_len();
    let taken = family.snapshot().map_err(CoreError::Encode)?;
    for file in &taken.files {
        store.files().put(&file.data).await?;
    }
    let snapshot = Snapshot { format: SNAPSHOT_FORMAT, covers: covers as u64, data: taken.data.clone() };
    store.put_snapshot(family_id, &snapshot).await?;
    family.snapshot_stored(&taken, file_reader(store));
    Ok(covers)
}

/// A family rebuilt from its stored deltas, starting from `snapshot` when it
/// is a snapshot of them, and how much of its log that snapshot covers.
pub(crate) fn replay(
    snapshot: Option<&Snapshot>,
    mut stored: Vec<Delta>,
    bases: &mut dyn BaseText,
    files: Option<ReadFile>,
) -> (Family, usize) {
    let restored = snapshot.filter(|snapshot| is_readable_format(snapshot.format)).and_then(|snapshot| {
        Family::from_snapshot(&snapshot.data, snapshot.format, &mut stored, files)
            .inspect_err(|error| warn!(%error, "replaying the family instead of reading its snapshot"))
            .ok()
    });
    let mut family = restored.unwrap_or_default();
    let covered = family.log_len();
    apply_stored(&mut family, stored, bases);
    (family, covered)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use silt_crdt::{InodeId, InodeKind, Parent, TextOp};
    use silt_log::{Author, Content, FileView, Op};
    use silt_types::{ActorId, ThreadId};

    use silt_log::MemoryFiles;

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

    /// The family's snapshot, its files kept in `files`.
    fn snapshot_of(family: &mut Family, files: &MemoryFiles, format: i64, covers: u64) -> Snapshot {
        let taken = family.snapshot().expect("encodes");
        files.store(&taken);
        family.snapshot_stored(&taken, files.reader());
        Snapshot { format, covers, data: taken.data }
    }

    #[test]
    fn a_family_starts_from_its_snapshot_and_applies_what_came_after() {
        let files = MemoryFiles::default();
        let stored = typed(20);
        let (mut partway, _) = replay(None, stored[..12].to_vec(), &mut HashMap::new(), None);
        let snapshot = snapshot_of(&mut partway, &files, SNAPSHOT_FORMAT, 12);
        let (restored, covered) = replay(Some(&snapshot), stored.clone(), &mut HashMap::new(), Some(files.reader()));
        let (replayed, nothing) = replay(None, stored, &mut HashMap::new(), None);
        assert_eq!((covered, nothing), (12, 0));
        assert_eq!(restored.log(), replayed.log());
        assert_eq!(text(&restored), Some(FileView::Text("x".repeat(20))));
    }

    #[test]
    fn a_snapshot_that_does_not_fit_is_replayed_instead() {
        let files = MemoryFiles::default();
        let stored = typed(3);
        let (mut other, _) = replay(None, typed(5), &mut HashMap::new(), None);
        let foreign = snapshot_of(&mut other, &files, SNAPSHOT_FORMAT, 6);
        let (mut partway, _) = replay(None, stored[..2].to_vec(), &mut HashMap::new(), None);
        let later_format = snapshot_of(&mut partway, &files, SNAPSHOT_FORMAT + 1, 2);
        let garbage = Snapshot { format: SNAPSHOT_FORMAT, covers: 2, data: b"not a snapshot".to_vec() };
        for snapshot in [foreign, later_format, garbage] {
            let (family, covered) = replay(Some(&snapshot), stored.clone(), &mut HashMap::new(), Some(files.reader()));
            assert_eq!(covered, 0);
            assert_eq!(text(&family), Some(FileView::Text("xxx".into())));
        }
    }

    #[tokio::test]
    async fn a_family_whose_snapshot_names_a_file_not_stored_is_replayed() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Store::open(dir.path()).await.expect("opens");
        let family_id = FamilyId(7);
        let repo_dir = tempfile::tempdir().expect("temp dir");
        let repo = Repo::init(repo_dir.path()).expect("inits");
        let thread = silt_store::ThreadRecord {
            id: ThreadId(1),
            family: family_id,
            repo_root: repo_dir.path().to_owned(),
            repo: "example.com/repo".into(),
            created_ms: 1,
            owner: String::new(),
        };
        store.create_thread(&thread).await.expect("thread");
        let stored = typed(4);
        store.append_deltas(family_id, &stored).await.expect("stores");
        let (mut family, _) = replay(None, stored.clone(), &mut HashMap::new(), None);
        save_snapshot(&store, family_id, &mut family).await.expect("saves");
        let Loaded::Projected { covered, outdated, .. } = family_unread(&store, family_id, Some(&repo)).await.expect("loads")
        else {
            panic!("a family with a clone is projected");
        };
        assert_eq!((covered, outdated), (5, false), "read from its snapshot");

        let hash = snapshot_files(&store.snapshot(family_id).await.expect("reads").expect("saved").data, SNAPSHOT_FORMAT)
            .expect("reads its index")[0];
        std::fs::remove_file(dir.path().join("files").join(&hash.to_string()[..2]).join(&hash.to_string()[2..]))
            .expect("removes");
        let Loaded::Projected { family, covered, .. } = family_unread(&store, family_id, Some(&repo)).await.expect("loads")
        else {
            panic!("a family with a clone is projected");
        };
        assert_eq!(covered, 0, "replayed");
        assert_eq!(text(&family), Some(FileView::Text("xxxx".into())));
    }
}
