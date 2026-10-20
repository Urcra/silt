//! Deleting threads to the trash, restoring them, and purging them for good
//! (ADR 0013).

use std::collections::HashSet;
use std::str::FromStr;
use std::time::{Duration, SystemTime};

use axum::Json;
use axum::extract::{Extension, Path as UrlPath, State};
use axum::http::StatusCode;
use silt_log::{Content, ConvOp, Delta, GitOp, Op, kept_after_purge};
use silt_proto::ThreadInfo;
use silt_sync::Room;
use silt_types::{BlobHash, FamilyId, ThreadId};
use sqlx::Row;

use crate::index::{self, Index};
use crate::{ApiError, Caller, Server, ServerError, bad_request, forbidden, id_bytes, id_from, not_found, now_ms};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Blobs this new may belong to a push still on its way, so they stay.
const BLOB_GRACE: Duration = Duration::from_secs(60 * 60);
const RETENTION_CHECK: Duration = Duration::from_secs(60 * 60);

/// The thread and its family, if `caller` owns it.
async fn owned(server: &Server, thread: &str, caller: Caller) -> Result<(ThreadId, FamilyId, Option<i64>), ApiError> {
    let thread = ThreadId::from_str(thread).map_err(|_| bad_request("invalid thread id"))?;
    let row = sqlx::query("SELECT family_id, owner_id, deleted_ms FROM threads WHERE id = ?")
        .bind(id_bytes(thread.0))
        .fetch_optional(server.db())
        .await?
        .ok_or_else(not_found)?;
    if row.get::<i64, _>("owner_id") != caller.user {
        return Err(forbidden("only the thread's owner can delete, restore or purge it"));
    }
    Ok((thread, FamilyId(id_from(row.get("family_id"))), row.get("deleted_ms")))
}

pub(crate) async fn delete(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    UrlPath(thread): UrlPath<String>,
) -> Result<StatusCode, ApiError> {
    let (thread, _, _) = owned(&server, &thread, caller).await?;
    sqlx::query("UPDATE threads SET deleted_ms = ? WHERE id = ? AND deleted_ms IS NULL")
        .bind(now_ms())
        .bind(id_bytes(thread.0))
        .execute(server.db())
        .await?;
    sqlx::query("DELETE FROM leases WHERE thread_id = ?")
        .bind(id_bytes(thread.0))
        .execute(server.db())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn restore(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    UrlPath(thread): UrlPath<String>,
) -> Result<StatusCode, ApiError> {
    let (thread, _, _) = owned(&server, &thread, caller).await?;
    sqlx::query("UPDATE threads SET deleted_ms = NULL WHERE id = ?")
        .bind(id_bytes(thread.0))
        .execute(server.db())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn purge(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    UrlPath(thread): UrlPath<String>,
) -> Result<StatusCode, ApiError> {
    let (thread, family, deleted) = owned(&server, &thread, caller).await?;
    if deleted.is_none() {
        return Err(bad_request("delete the thread first; purging works on the trash"));
    }
    server.purge(&[(thread, family)]).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The caller's deleted threads.
pub(crate) async fn trash(State(server): State<Server>, Extension(caller): Extension<Caller>) -> Result<Json<Vec<ThreadInfo>>, ApiError> {
    let rows = sqlx::query(
        "SELECT t.id, t.family_id, t.repo, t.created_ms, u.name AS owner FROM threads t JOIN users u ON u.id = t.owner_id
         WHERE t.owner_id = ? AND t.deleted_ms IS NOT NULL ORDER BY t.deleted_ms",
    )
    .bind(caller.user)
    .fetch_all(server.db())
    .await?;
    Ok(Json(
        rows.iter()
            .map(|row| ThreadInfo {
                id: ThreadId(id_from(row.get("id"))),
                family: FamilyId(id_from(row.get("family_id"))),
                repo: row.get("repo"),
                created_ms: row.get("created_ms"),
                lease: None,
                owner: row.get("owner"),
                collaborators: Vec::new(),
            })
            .collect(),
    ))
}

/// The blobs a delta needs.
fn blob_of(delta: &Delta) -> Option<BlobHash> {
    match &delta.op {
        Op::Create {
            content: Content::Blob(hash),
            ..
        }
        | Op::SetContent {
            content: Content::Blob(hash),
            ..
        }
        | Op::Conv(ConvOp::Attachment { blob: hash, .. } | ConvOp::SessionTranslated { lines: hash, .. })
        | Op::Git(GitOp::Bundle { blob: hash, .. }) => Some(*hash),
        _ => None,
    }
}

impl Server {
    /// Thread ids deleted and purged, for listings.
    pub(crate) async fn lifecycle(&self) -> Result<(Vec<ThreadId>, Vec<ThreadId>), sqlx::Error> {
        let ids = |rows: Vec<sqlx::sqlite::SqliteRow>, column: &str| -> Vec<ThreadId> {
            rows.iter().map(|row| ThreadId(id_from(row.get(column)))).collect()
        };
        let deleted = sqlx::query("SELECT id FROM threads WHERE deleted_ms IS NOT NULL")
            .fetch_all(self.db())
            .await?;
        let purged = sqlx::query("SELECT thread_id FROM purged_threads").fetch_all(self.db()).await?;
        Ok((ids(deleted, "id"), ids(purged, "thread_id")))
    }

    /// Removes threads for good: their metadata, the deltas no remaining
    /// thread builds on, families left empty with their git mirrors, and
    /// blobs nothing references any more.
    pub(crate) async fn purge(&self, threads: &[(ThreadId, FamilyId)]) -> Result<(), ServerError> {
        if threads.is_empty() {
            return Ok(());
        }
        let db = self.db();
        let mut rooms = self.shared.rooms.lock().await;
        let mut purged = self.shared.purged.lock().await;
        let mut tx = db.begin().await?;
        for (thread, family) in threads {
            purged.insert(*thread);
            sqlx::query("INSERT OR IGNORE INTO purged_threads (thread_id, family_id, purged_ms) VALUES (?, ?, ?)")
                .bind(id_bytes(thread.0))
                .bind(id_bytes(family.0))
                .bind(now_ms())
                .execute(&mut *tx)
                .await?;
            for statement in [
                "DELETE FROM leases WHERE thread_id = ?",
                "DELETE FROM notifications WHERE thread_id = ?",
                "DELETE FROM touches WHERE thread_id = ?",
                "DELETE FROM thread_activity WHERE thread_id = ?",
                "DELETE FROM search WHERE thread_id = ?",
            ] {
                sqlx::query(statement).bind(id_bytes(thread.0)).execute(&mut *tx).await?;
            }
            sqlx::query("DELETE FROM threads WHERE id = ?")
                .bind(id_bytes(thread.0))
                .execute(&mut *tx)
                .await?;
        }
        let families: HashSet<FamilyId> = threads.iter().map(|(_, family)| *family).collect();
        for family in families {
            let remaining: i64 = sqlx::query("SELECT COUNT(*) AS remaining FROM threads WHERE family_id = ?")
                .bind(id_bytes(family.0))
                .fetch_one(&mut *tx)
                .await?
                .get("remaining");
            sqlx::query("DELETE FROM deltas WHERE family_id = ?")
                .bind(id_bytes(family.0))
                .execute(&mut *tx)
                .await?;
            if remaining == 0 {
                rooms.remove(&family);
                sqlx::query("DELETE FROM families WHERE id = ?")
                    .bind(id_bytes(family.0))
                    .execute(&mut *tx)
                    .await?;
                let mirror = self.git_root().join(format!("{family}.git"));
                if mirror.exists() {
                    tokio::fs::remove_dir_all(mirror).await?;
                }
                continue;
            }
            let Some(room) = rooms.get(&family) else {
                continue;
            };
            let deltas = room.deltas().to_vec();
            let kept: Vec<Delta> = deltas
                .into_iter()
                .zip(kept_after_purge(room.deltas(), &purged))
                .filter_map(|(delta, kept)| kept.then_some(delta))
                .collect();
            let mut rebuilt = Room::new(self.shared.generation);
            if !kept.is_empty() {
                rebuilt.push(kept.clone()).map_err(|error| std::io::Error::other(error.to_string()))?;
            }
            for (seq, delta) in (1..).zip(&kept) {
                sqlx::query("INSERT INTO deltas (family_id, seq, data) VALUES (?, ?, ?)")
                    .bind(id_bytes(family.0))
                    .bind(seq)
                    .bind(postcard::to_allocvec(delta)?)
                    .execute(&mut *tx)
                    .await?;
            }
            rooms.insert(family, rebuilt);
        }
        tx.commit().await?;
        let mut index = Index::default();
        let mut rows = Vec::new();
        let mut referenced = HashSet::new();
        for (family, room) in rooms.iter() {
            for delta in room.deltas() {
                rows.extend(index.observe(*family, delta));
                referenced.extend(blob_of(delta));
            }
        }
        let mut tx = db.begin().await?;
        index::clear(&mut tx).await?;
        index::write(&mut tx, &rows).await?;
        tx.commit().await?;
        *self.shared.index.lock().await = index;
        drop(purged);
        drop(rooms);
        for family in threads.iter().map(|(_, family)| *family).collect::<HashSet<_>>() {
            self.waiter(family).await.notify_waiters();
        }
        let now = SystemTime::now();
        for (blob, written) in self.shared.blobs.list().await? {
            let old = now.duration_since(written).is_ok_and(|age| age > BLOB_GRACE);
            if old && !referenced.contains(&blob) {
                self.shared.blobs.remove(blob).await?;
            }
        }
        Ok(())
    }

    /// Purges threads that have been in the trash longer than configured.
    pub(crate) async fn purge_expired(&self) -> Result<(), ServerError> {
        let cutoff = now_ms() - i64::from(self.config().trash_days) * DAY_MS;
        let rows = sqlx::query("SELECT id, family_id FROM threads WHERE deleted_ms IS NOT NULL AND deleted_ms < ?")
            .bind(cutoff)
            .fetch_all(self.db())
            .await?;
        let expired: Vec<(ThreadId, FamilyId)> = rows
            .iter()
            .map(|row| (ThreadId(id_from(row.get("id"))), FamilyId(id_from(row.get("family_id")))))
            .collect();
        if !expired.is_empty() {
            tracing::info!(threads = expired.len(), "purging threads whose time in the trash is up");
        }
        self.purge(&expired).await
    }

    /// Checks the trash every hour while the server runs.
    pub(crate) fn keep_trash_tidy(&self) {
        let server = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(RETENTION_CHECK);
            loop {
                interval.tick().await;
                if let Err(error) = server.purge_expired().await {
                    tracing::warn!(%error, "could not purge the trash");
                }
            }
        });
    }

    /// Drops deltas of purged threads from a push, so a device that hasn't
    /// heard of the purge can't bring them back.
    pub(crate) async fn without_purged(&self, deltas: Vec<Delta>) -> Vec<Delta> {
        let purged = self.shared.purged.lock().await;
        if purged.is_empty() {
            return deltas;
        }
        deltas.into_iter().filter(|delta| !purged.contains(&delta.thread)).collect()
    }
}

/// Purged thread ids, loaded on start.
pub(crate) async fn load_purged(db: &sqlx::sqlite::SqlitePool) -> Result<HashSet<ThreadId>, sqlx::Error> {
    Ok(sqlx::query("SELECT thread_id FROM purged_threads")
        .fetch_all(db)
        .await?
        .iter()
        .map(|row| ThreadId(id_from(row.get("thread_id"))))
        .collect())
}
