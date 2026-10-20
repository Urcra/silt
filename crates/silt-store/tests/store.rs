use std::path::PathBuf;

use silt_crdt::{InodeKind, Parent};
use silt_log::{Author, Content, Family, Op};
use silt_store::{BlobStore, DiskFile, SearchHit, SessionFileOffset, SessionRecord, Store, ThreadRecord};
use silt_types::{ActorId, DeltaId, FamilyId, Frontier, Lamport, ThreadId};

const THREAD: ThreadId = ThreadId(11);
const FAMILY: FamilyId = FamilyId(22);

async fn store_with_thread() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = Store::open(dir.path()).await.expect("opens");
    store
        .create_thread(&ThreadRecord {
            id: THREAD,
            family: FAMILY,
            repo_root: PathBuf::from("/repo"),
            created_ms: 1,
        })
        .await
        .expect("thread");
    (dir, store)
}

#[tokio::test]
async fn deltas_round_trip_in_order_across_reopen() {
    let (dir, store) = store_with_thread().await;
    let mut family = Family::new();
    let mut appended = Vec::new();
    for name in ["a.txt", "b.txt"] {
        let delta = family.delta(
            ActorId(1),
            THREAD,
            Author::User,
            5,
            family.thread_head(THREAD),
            Op::Create {
                kind: InodeKind::File,
                parent: Parent::Root,
                name: name.into(),
                content: Content::Empty,
                executable: false,
            },
        );
        family
            .apply(delta.clone(), &mut std::collections::HashMap::new())
            .expect("applies");
        store.append_deltas(FAMILY, std::slice::from_ref(&delta)).await.expect("appends");
        appended.push(delta);
    }
    drop(store);

    let store = Store::open(dir.path()).await.expect("reopens");
    assert_eq!(store.deltas(FAMILY).await.expect("loads"), appended);
    assert_eq!(store.families().await.expect("families"), vec![FAMILY]);
    assert_eq!(store.threads().await.expect("threads")[0].id, THREAD);
}

#[tokio::test]
async fn checkouts_and_disk_state() {
    let (_dir, store) = store_with_thread().await;
    let checkout = store
        .add_checkout(THREAD, &PathBuf::from("/repo"), ActorId(5), false, 1)
        .await
        .expect("checkout");
    let file = DiskFile {
        inode: DeltaId::new(ActorId(1), Lamport(1)),
        hash: [1; 32],
        size: 3,
        mtime_ns: 99,
        version: Frontier::single(DeltaId::new(ActorId(5), Lamport(2))),
    };
    store.put_disk_file(checkout.id, "src/main.rs", &file).await.expect("put");
    let updated = DiskFile { size: 4, ..file };
    store.put_disk_file(checkout.id, "src/main.rs", &updated).await.expect("update");
    assert_eq!(store.disk_files(checkout.id).await.expect("files")["src/main.rs"], updated);
    store.remove_disk_file(checkout.id, "src/main.rs").await.expect("remove");
    assert!(store.disk_files(checkout.id).await.expect("files").is_empty());
    assert_eq!(store.checkouts().await.expect("checkouts"), vec![checkout]);
}

#[tokio::test]
async fn session_offsets_persist() {
    let (_dir, store) = store_with_thread().await;
    let checkout = store
        .add_checkout(THREAD, &PathBuf::from("/repo"), ActorId(5), false, 1)
        .await
        .expect("checkout");
    let session = SessionRecord {
        agent: "claude-code".into(),
        session_id: "abc".into(),
        thread: THREAD,
        checkout_id: checkout.id,
    };
    store.add_session(&session, 1).await.expect("session");
    store.add_session(&session, 2).await.expect("adding again is a no-op");
    let offset = SessionFileOffset {
        file: "main".into(),
        path: PathBuf::from("/home/u/.claude/projects/x/abc.jsonl"),
        offset: 1234,
    };
    store.set_session_offset("claude-code", "abc", &offset).await.expect("offset");
    store
        .set_session_offset("claude-code", "abc", &SessionFileOffset { offset: 2000, ..offset.clone() })
        .await
        .expect("advance");
    assert_eq!(store.sessions().await.expect("sessions"), vec![session]);
    assert_eq!(
        store.session_offsets("claude-code", "abc").await.expect("offsets"),
        vec![SessionFileOffset { offset: 2000, ..offset }]
    );
}

#[tokio::test]
async fn full_text_search_finds_prefixes_and_ignores_query_syntax() {
    let (_dir, store) = store_with_thread().await;
    let delta = DeltaId::new(ActorId(1), Lamport(4));
    store
        .index(THREAD, delta, "assistant", "Refactored the tokenizer to handle unicode escapes")
        .await
        .expect("index");
    store.index(THREAD, DeltaId::new(ActorId(1), Lamport(9)), "user", "fix the parser").await.expect("index");

    let hits = store.search("tokeni unicode", 10).await.expect("search");
    assert_eq!(hits.len(), 1);
    let SearchHit { thread, delta: found, kind, snippet } = &hits[0];
    assert_eq!((*thread, *found, kind.as_str()), (THREAD, delta, "assistant"));
    assert!(snippet.contains("[tokenizer]"), "{snippet}");

    assert!(store.search("\"unbalanced AND (", 10).await.expect("search").is_empty());
    assert!(store.search("   ", 10).await.expect("search").is_empty());
}

#[tokio::test]
async fn blobs_are_content_addressed() {
    let dir = tempfile::tempdir().expect("temp dir");
    let blobs = BlobStore::new(dir.path());
    let hash = blobs.put(b"binary \0 data").await.expect("put");
    assert_eq!(hash, BlobStore::hash(b"binary \0 data"));
    assert_eq!(blobs.put(b"binary \0 data").await.expect("put again"), hash);
    assert_eq!(blobs.get(hash).await.expect("get"), b"binary \0 data");
}
