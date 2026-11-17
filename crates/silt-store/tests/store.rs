use std::path::PathBuf;

use silt_crdt::{InodeKind, Parent};
use silt_log::{Author, Content, Family, Op};
use silt_store::{
    AgentRunRecord, BlobStore, DiskFile, SearchHit, SearchRow, SessionFileOffset, SessionRead, SessionRecord, Snapshot, Store,
    ThreadRecord,
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
    let appended = titles(2_500);
    // Unevenly, so packing happens partway through a batch.
    for batch in appended.chunks(700) {
        store.append_deltas(FAMILY, batch).await.expect("appends");
    }
    assert_eq!(store.deltas(FAMILY).await.expect("loads"), appended);
    for after in [0, 998, 999, 1_000, 1_500, 1_999, 2_000, 2_400, 2_499] {
        let from = usize::try_from(after + 1).expect("a position");
        assert_eq!(store.deltas_after(FAMILY, after).await.expect("loads"), appended[from..], "after {after}");
    }

    let stored = store.stored_deltas(FAMILY).await.expect("reads");
    assert_eq!(stored.len(), appended.len());
    for range in [0..0, 0..1, 998..1_002, 999..1_000, 1_999..2_001, 2_400..2_500, 0..2_500] {
        assert_eq!(stored.decode(range.clone()).expect("decodes"), appended[range.clone()], "{range:?}");
    }

    store.replace_deltas(FAMILY, &appended[..10]).await.expect("replaces");
    assert_eq!(store.deltas(FAMILY).await.expect("loads"), appended[..10], "chunks of the replaced deltas are gone");
}

#[tokio::test]
async fn search_rows_written_after_their_deltas_are_pending_until_written() {
    let (_dir, store) = store_with_thread().await;
    store.append_deltas(FAMILY, &titles(10)).await.expect("appends");
    let pulled = titles(31);
    store.append_deltas_unindexed(FAMILY, &pulled[10..30]).await.expect("appends");
    assert_eq!(store.deltas(FAMILY).await.expect("loads"), pulled[..30]);
    assert_eq!(store.pending_search().await.expect("pending"), vec![(FAMILY, 10)]);
    store.append_deltas_unindexed(FAMILY, &pulled[30..]).await.expect("appends");
    assert_eq!(store.pending_search().await.expect("pending"), vec![(FAMILY, 10)], "the first unwritten stays");

    let row = |delta: &silt_log::Delta, body: &str| silt_store::SearchRow {
        thread: THREAD,
        delta: delta.span.first(),
        kind: "user".into(),
        body: body.into(),
    };
    store.write_pending_search(FAMILY, &[row(&pulled[12], "halfway there")], 19).await.expect("writes");
    assert_eq!(store.pending_search().await.expect("pending"), vec![(FAMILY, 20)]);
    store.write_pending_search(FAMILY, &[row(&pulled[25], "the rest")], 30).await.expect("writes");
    assert!(store.pending_search().await.expect("pending").is_empty());
    assert_eq!(store.search("halfway", 10).await.expect("searches").len(), 1);
    assert_eq!(store.search("rest", 10).await.expect("searches").len(), 1);
}

#[tokio::test]
async fn compacting_compresses_the_chunks_stored_plain_and_reads_back_the_same() {
    let (dir, store) = store_with_thread().await;
    let appended = titles(2_500);
    store.append_deltas(FAMILY, &appended).await.expect("appends");
    // A chunk as stores before ADR 0035 kept it: packed, not compressed.
    let raw =
        sqlx::SqlitePool::connect(&format!("sqlite://{}", dir.path().join("silt.sqlite").display())).await.expect("connects");
    let (first, data): (i64, Vec<u8>) = sqlx::query_as("SELECT first_seq, data FROM delta_chunks ORDER BY first_seq LIMIT 1")
        .fetch_one(&raw)
        .await
        .expect("a chunk");
    let packed = zstd::stream::decode_all(data.as_slice()).expect("decompresses");
    sqlx::query("UPDATE delta_chunks SET codec = 0, data = ? WHERE first_seq = ?")
        .bind(&packed)
        .bind(first)
        .execute(&raw)
        .await
        .expect("stores it plain");
    raw.close().await;
    let before = store.reclaimable().await.expect("measures");
    assert_eq!(before.plain_chunks, 1);
    assert!(before.is_worth_it());

    let compacted = store.compact().await.expect("compacts");
    assert_eq!(compacted.recompressed, 1);
    assert!(compacted.after <= compacted.before);
    assert_eq!(store.reclaimable().await.expect("measures").plain_chunks, 0);
    assert!(!store.reclaimable().await.expect("measures").is_worth_it());
    assert_eq!(store.deltas(FAMILY).await.expect("loads"), appended);
}

#[tokio::test]
async fn deltas_appended_one_by_one_or_all_at_once_read_the_same() {
    let appended = titles(2_100);
    let (_dir, one_by_one) = store_with_thread().await;
    for delta in &appended {
        one_by_one.append_deltas(FAMILY, std::slice::from_ref(delta)).await.expect("appends");
    }
    let (_other, at_once) = store_with_thread().await;
    at_once.append_deltas(FAMILY, &appended).await.expect("appends");
    for store in [&one_by_one, &at_once] {
        assert_eq!(store.deltas(FAMILY).await.expect("loads"), appended);
        assert_eq!(store.deltas_after(FAMILY, 1_500).await.expect("loads"), appended[1_501..]);
        assert_eq!(store.position(FAMILY).await.expect("position"), Some(2_099));
    }
}

#[tokio::test]
async fn a_failed_append_stores_none_of_its_deltas_rows_or_progress() {
    let (_dir, store) = store_with_thread().await;
    let checkout = store.add_checkout(THREAD, &PathBuf::from("/repo"), ActorId(5), false, 1).await.expect("checkout");
    let session =
        SessionRecord { agent: "claude-code".into(), session_id: "abc".into(), thread: THREAD, checkout_id: checkout.id };
    store.add_session(&session, 1).await.expect("session");
    let appended = titles(1_500);
    store.append_deltas(FAMILY, &appended[..10]).await.expect("appends");

    // The last delta is one stored already, which the store refuses.
    let mut batch = appended[10..].to_vec();
    batch.push(appended[0].clone());
    let rows: Vec<SearchRow> = batch
        .iter()
        .map(|delta| SearchRow { thread: THREAD, delta: delta.span.first(), kind: "user".into(), body: "unindexed".into() })
        .collect();
    let offset = SessionFileOffset { file: "main".into(), path: PathBuf::from("/t.jsonl"), offset: 99 };
    let read = SessionRead { agent: "claude-code", session_id: "abc", offset: &offset };
    assert!(store.append_deltas_with(FAMILY, &batch, &rows, Some(read)).await.is_err());

    assert_eq!(store.deltas(FAMILY).await.expect("loads"), appended[..10]);
    assert!(store.search("unindexed", 10).await.expect("search").is_empty());
    assert!(store.session_offsets("claude-code", "abc").await.expect("offsets").is_empty());
    store.append_deltas(FAMILY, &appended[10..]).await.expect("appends what's left");
    assert_eq!(store.deltas(FAMILY).await.expect("loads"), appended);
}

#[tokio::test]
async fn search_rows_appended_in_bulk_are_found_and_go_with_their_family() {
    let (_dir, store) = store_with_thread().await;
    let appended = titles(1_200);
    let rows: Vec<SearchRow> = appended
        .iter()
        .enumerate()
        .map(|(index, delta)| SearchRow {
            thread: THREAD,
            delta: delta.span.first(),
            kind: "user".into(),
            body: format!("marker{index} text"),
        })
        .collect();
    store.append_deltas_with(FAMILY, &appended, &rows, None).await.expect("appends");
    let hits = store.search("marker1199", 10).await.expect("search");
    assert_eq!(hits.iter().map(|hit| hit.delta).collect::<Vec<_>>(), [appended[1_199].span.first()]);
    store.replace_deltas(FAMILY, &[]).await.expect("replaces");
    assert!(store.search("marker0", 10).await.expect("search").is_empty());
}

fn titles(count: usize) -> Vec<silt_log::Delta> {
    let mut family = Family::new();
    (0..count)
        .map(|index| {
            let op = Op::Conv(silt_log::ConvOp::Title(format!("title {index}")));
            let delta = family.delta(ActorId(1), THREAD, Author::User, 5, family.thread_head(THREAD), op);
            family.apply(delta.clone(), &mut std::collections::HashMap::new()).expect("applies");
            delta
        })
        .collect()
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
async fn a_clone_has_a_checkout_for_each_thread_working_there() {
    let (_dir, store) = store_with_thread().await;
    const OTHER: ThreadId = ThreadId(13);
    store
        .create_thread(&ThreadRecord {
            id: OTHER,
            family: FAMILY,
            repo_root: PathBuf::from("/repo"),
            repo: "github.com/u/repo".into(),
            created_ms: 2,
            owner: String::new(),
        })
        .await
        .expect("thread");
    let repo = PathBuf::from("/repo");
    let first = store.add_checkout(THREAD, &repo, ActorId(5), false, 1).await.expect("checkout");
    let second = store.add_checkout(OTHER, &repo, ActorId(6), false, 2).await.expect("a second thread's checkout");
    store.add_checkout(OTHER, &PathBuf::from("/elsewhere"), ActorId(7), false, 3).await.expect("checkout");
    assert!(store.add_checkout(OTHER, &repo, ActorId(8), false, 4).await.is_err(), "one per thread");
    assert_eq!(store.checkouts_at(&repo).await.expect("checkouts"), [first.clone(), second.clone()]);

    let session =
        |id: &str, thread, checkout_id| SessionRecord { agent: "claude-code".into(), session_id: id.into(), thread, checkout_id };
    store.add_session(&session("a", THREAD, first.id), 1).await.expect("session");
    store.add_session(&session("b", OTHER, second.id), 1).await.expect("session");
    store.remove_thread(OTHER).await.expect("removes");
    assert_eq!(store.checkouts_at(&repo).await.expect("checkouts"), std::slice::from_ref(&first));
    assert_eq!(store.sessions().await.expect("sessions"), [session("a", THREAD, first.id)]);
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
