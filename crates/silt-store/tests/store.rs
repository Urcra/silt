use std::path::PathBuf;

use silt_crdt::{InodeKind, Parent};
use silt_log::{Author, Content, Family, Op};
use silt_store::{
    AgentRunRecord, BlobStore, DiskFile, SearchHit, SearchRow, SessionFileOffset, SessionRecord, Snapshot, Store, ThreadRecord,
};
use silt_types::{ActorId, BlobHash, DeltaId, FamilyId, Frontier, Lamport, ThreadId};

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
            repo: "github.com/u/repo".into(),
            created_ms: 1,
            owner: String::new(),
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
        family.apply(delta.clone(), &mut std::collections::HashMap::new()).expect("applies");
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
async fn a_long_family_reads_the_same_once_packed_into_chunks() {
    let (_dir, store) = store_with_thread().await;
    let mut family = Family::new();
    let mut appended = Vec::new();
    for index in 0..2_500 {
        let op = Op::Conv(silt_log::ConvOp::Title(format!("title {index}")));
        let delta = family.delta(ActorId(1), THREAD, Author::User, 5, family.thread_head(THREAD), op);
        family.apply(delta.clone(), &mut std::collections::HashMap::new()).expect("applies");
        appended.push(delta);
    }
    // Unevenly, so packing happens partway through a batch.
    for batch in appended.chunks(700) {
        store.append_deltas(FAMILY, batch).await.expect("appends");
    }
    assert_eq!(store.deltas(FAMILY).await.expect("loads"), appended);
    assert_eq!(store.deltas_after(FAMILY, 2_400).await.expect("loads"), appended[2_401..]);

    store.replace_deltas(FAMILY, &appended[..10]).await.expect("replaces");
    assert_eq!(store.deltas(FAMILY).await.expect("loads"), appended[..10], "chunks of the replaced deltas are gone");
}

#[tokio::test]
async fn a_queue_keeps_its_order_through_edits_moves_and_takes() {
    let (_dir, store) = store_with_thread().await;
    let texts = |queue: Vec<silt_store::QueuedPrompt>| queue.into_iter().map(|queued| queued.text).collect::<Vec<_>>();
    let first = store.enqueue(THREAD, "ada", "first", 1, false).await.expect("adds");
    let second = store.enqueue(THREAD, "ada", "second", 2, false).await.expect("adds");
    store.enqueue(THREAD, "bob", "urgent", 3, true).await.expect("adds at the front");
    assert_eq!(texts(store.queue(THREAD).await.expect("reads")), ["urgent", "first", "second"]);

    assert!(store.edit_queued(THREAD, first, "first, edited").await.expect("edits"));
    assert!(store.move_queued(THREAD, second, 0).await.expect("moves"));
    assert_eq!(texts(store.queue(THREAD).await.expect("reads")), ["second", "urgent", "first, edited"]);

    assert_eq!(store.take_next(THREAD).await.expect("takes").map(|queued| queued.text), Some("second".into()));
    assert!(store.take_queued(THREAD, second).await.expect("takes").is_none(), "taken once");
    assert!(!store.edit_queued(THREAD, second, "too late").await.expect("edits"), "a sent prompt can't change");
    assert_eq!(texts(store.queue(THREAD).await.expect("reads")), ["urgent", "first, edited"]);
}

#[tokio::test]
async fn a_thread_s_agent_run_is_kept_and_replaced() {
    let (dir, store) = store_with_thread().await;
    assert!(store.agent_runs().await.expect("reads").is_empty());
    let mut run = AgentRunRecord {
        agent: "Codex".into(),
        state: "working".into(),
        error: None,
        preset: "ask_before_commands".into(),
        mode: Some("auto".into()),
        updated_ms: 5,
    };
    store.put_agent_run(THREAD, &run).await.expect("saves");
    run.state = "error".into();
    run.error = Some("it crashed".into());
    store.put_agent_run(THREAD, &run).await.expect("replaces");
    drop(store);
    let store = Store::open(dir.path()).await.expect("reopens");
    assert_eq!(store.agent_runs().await.expect("reads").remove(&THREAD), Some(run));
}

#[tokio::test]
async fn a_snapshot_lasts_until_the_family_s_deltas_are_replaced() {
    let (dir, store) = store_with_thread().await;
    assert_eq!(store.snapshot(FAMILY).await.expect("reads"), None);
    let snapshot = Snapshot { format: 1, covers: 2, data: b"projection".to_vec() };
    store.put_snapshot(FAMILY, &snapshot).await.expect("saves");
    let newer = Snapshot { format: 1, covers: 5, data: b"later".to_vec() };
    store.put_snapshot(FAMILY, &newer).await.expect("replaces");
    drop(store);

    let store = Store::open(dir.path()).await.expect("reopens");
    assert_eq!(store.snapshot(FAMILY).await.expect("reads"), Some(newer));
    store.replace_deltas(FAMILY, &[]).await.expect("replaces");
    assert_eq!(store.snapshot(FAMILY).await.expect("reads"), None, "it was a snapshot of other deltas");
    store.put_snapshot(FAMILY, &snapshot).await.expect("saves");
    store.clear_deltas(FAMILY).await.expect("clears");
    assert_eq!(store.snapshot(FAMILY).await.expect("reads"), None);
}

#[tokio::test]
async fn notices_are_handed_over_once_in_order_and_go_with_their_thread() {
    let (dir, store) = store_with_thread().await;
    store.add_notice(THREAD, "first", 1).await.expect("adds");
    store.add_notice(THREAD, "second", 2).await.expect("adds");
    drop(store);

    let store = Store::open(dir.path()).await.expect("reopens");
    assert_eq!(store.take_notices(THREAD).await.expect("takes"), ["first", "second"]);
    assert!(store.take_notices(THREAD).await.expect("takes").is_empty(), "taken once");
    store.add_notice(THREAD, "unread", 3).await.expect("adds");
    store.remove_thread(THREAD).await.expect("removes");
    assert!(store.take_notices(THREAD).await.expect("takes").is_empty());
}

#[tokio::test]
async fn checkouts_and_disk_state() {
    let (_dir, store) = store_with_thread().await;
    let checkout = store.add_checkout(THREAD, &PathBuf::from("/repo"), ActorId(5), false, 1).await.expect("checkout");
    let file = DiskFile {
        inode: DeltaId::new(ActorId(1), Lamport(1)),
        hash: BlobHash([1; 32]),
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
    let checkout = store.add_checkout(THREAD, &PathBuf::from("/repo"), ActorId(5), false, 1).await.expect("checkout");
    let session =
        SessionRecord { agent: "claude-code".into(), session_id: "abc".into(), thread: THREAD, checkout_id: checkout.id };
    store.add_session(&session, 1).await.expect("session");
    store.add_session(&session, 2).await.expect("adding again is a no-op");
    let offset =
        SessionFileOffset { file: "main".into(), path: PathBuf::from("/home/u/.claude/projects/x/abc.jsonl"), offset: 1234 };
    store.set_session_offset("claude-code", "abc", &offset).await.expect("offset");
    store.set_session_offset("claude-code", "abc", &SessionFileOffset { offset: 2000, ..offset.clone() }).await.expect("advance");
    assert_eq!(store.sessions().await.expect("sessions"), vec![session]);
    assert_eq!(
        store.session_offsets("claude-code", "abc").await.expect("offsets"),
        vec![SessionFileOffset { offset: 2000, ..offset }]
    );
}

#[tokio::test]
async fn a_thread_goes_with_the_sessions_of_a_checkout_rebound_to_it() {
    let (_dir, store) = store_with_thread().await;
    let checkout = store.add_checkout(THREAD, &PathBuf::from("/repo"), ActorId(5), false, 1).await.expect("checkout");
    let session =
        SessionRecord { agent: "claude-code".into(), session_id: "abc".into(), thread: THREAD, checkout_id: checkout.id };
    store.add_session(&session, 1).await.expect("session");
    const RECOVERY: ThreadId = ThreadId(12);
    store
        .create_thread(&ThreadRecord {
            id: RECOVERY,
            family: FAMILY,
            repo_root: PathBuf::from("/repo"),
            repo: "github.com/u/repo".into(),
            created_ms: 2,
            owner: String::new(),
        })
        .await
        .expect("thread");
    store.rebind_checkout(checkout.id, RECOVERY, ActorId(6)).await.expect("rebinds");

    store.remove_thread(RECOVERY).await.expect("removes the thread and the checkout's sessions");
    assert!(store.checkouts().await.expect("checkouts").is_empty());
    assert!(store.sessions().await.expect("sessions").is_empty());
}

#[tokio::test]
async fn full_text_search_finds_prefixes_and_ignores_query_syntax() {
    let (_dir, store) = store_with_thread().await;
    let delta = DeltaId::new(ActorId(1), Lamport(4));
    let row = |delta, kind: &str, body: &str| SearchRow { thread: THREAD, delta, kind: kind.into(), body: body.into() };
    let rows = [
        row(delta, "assistant", "Refactored the tokenizer to handle unicode escapes"),
        row(DeltaId::new(ActorId(1), Lamport(9)), "user", "fix the parser"),
    ];
    store.append_deltas_with(FAMILY, &[], &rows, None).await.expect("index");

    let hits = store.search("tokeni unicode", 10).await.expect("search");
    assert_eq!(hits.len(), 1);
    let SearchHit { thread, delta: found, kind, snippet } = &hits[0];
    assert_eq!((*thread, *found, kind.as_str()), (THREAD, delta, "assistant"));
    assert!(snippet.contains("[tokenizer]"), "{snippet}");

    assert!(store.search("\"unbalanced AND (", 10).await.expect("search").is_empty());
    assert!(store.search("   ", 10).await.expect("search").is_empty());

    store.replace_deltas(FAMILY, &[]).await.expect("replaces");
    assert!(store.search("parser", 10).await.expect("search").is_empty(), "a family read again is indexed again, not twice");
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

#[tokio::test]
async fn sync_state_round_trips() {
    let (_dir, store) = store_with_thread().await;
    assert_eq!(store.sync_state(FAMILY).await.expect("default"), silt_store::SyncState::default());
    let state = silt_store::SyncState {
        generation: Some(u64::MAX - 3),
        pushed: 10,
        pulled: 7,
        last_pulled: Some(DeltaId::new(ActorId(2), Lamport(5))),
        view: u64::MAX - 9,
    };
    store.set_sync_state(FAMILY, &state).await.expect("set");
    assert_eq!(store.sync_state(FAMILY).await.expect("get"), state);
}
