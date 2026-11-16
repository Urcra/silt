//! Deleting threads to the trash, restoring them, and purging them for good
//! (ADR 0013).

use std::collections::{BTreeMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use axum::Json;
use axum::extract::{Extension, Path as UrlPath, State};
use axum::http::StatusCode;
use silt_log::{Content, ConvOp, Delta, GitOp, Op, kept_after_purge};
use silt_proto::ThreadInfo;
use silt_sync::Room;
use silt_types::{BlobHash, FamilyId, ThreadId};
use tokio::sync::OwnedMutexGuard;

use crate::index::{self, Index};
use crate::visibility::Viewer;
use crate::{
    ApiError, Caller, RoomSlot, Server, ServerError, SharedRoom, UserId, bad_request, forbidden, id_bytes, id_from, not_found,
    now_ms,
};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Blobs this new may belong to a push still on its way, so they stay.
const BLOB_GRACE: Duration = Duration::from_secs(60 * 60);
const RETENTION_CHECK: Duration = Duration::from_secs(60 * 60);

/// The thread and its family, if `caller` owns it.
async fn owned(server: &Server, thread: &str, caller: Caller) -> Result<(ThreadId, FamilyId, Option<i64>), ApiError> {
    let thread = ThreadId::from_str(thread).map_err(|_| bad_request("invalid thread id"))?;
    let thread_id = id_bytes(thread.0);
    let row =
        sqlx::query!(r#"SELECT family_id, owner_id AS "owner_id: UserId", deleted_ms FROM threads WHERE id = ?"#, thread_id)
            .fetch_optional(server.db())
            .await?
            .ok_or_else(not_found)?;
    if row.owner_id != caller.user {
        return Err(forbidden("only the thread's owner can delete, restore or purge it"));
    }
    Ok((thread, FamilyId(id_from(&row.family_id)), row.deleted_ms))
}

pub(crate) async fn delete(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    UrlPath(thread): UrlPath<String>,
) -> Result<StatusCode, ApiError> {
    let (thread, _, _) = owned(&server, &thread, caller).await?;
    let (thread, now) = (id_bytes(thread.0), now_ms());
    sqlx::query!("UPDATE threads SET deleted_ms = ? WHERE id = ? AND deleted_ms IS NULL", now, thread)
        .execute(server.db())
        .await?;
    sqlx::query!("DELETE FROM leases WHERE thread_id = ?", thread).execute(server.db()).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn restore(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    UrlPath(thread): UrlPath<String>,
) -> Result<StatusCode, ApiError> {
    let (thread, _, _) = owned(&server, &thread, caller).await?;
    let thread = id_bytes(thread.0);
    sqlx::query!("UPDATE threads SET deleted_ms = NULL WHERE id = ?", thread).execute(server.db()).await?;
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
pub(crate) async fn trash(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<Vec<ThreadInfo>>, ApiError> {
    let rows = sqlx::query!(
        "SELECT t.id, t.family_id, t.repo, t.created_ms, u.name AS owner FROM threads t JOIN users u ON u.id = t.owner_id
         WHERE t.owner_id = ? AND t.deleted_ms IS NOT NULL ORDER BY t.deleted_ms",
        caller.user,
    )
    .fetch_all(server.db())
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|row| ThreadInfo {
                id: ThreadId(id_from(&row.id)),
                family: FamilyId(id_from(&row.family_id)),
                repo: row.repo,
                created_ms: row.created_ms,
                lease: None,
                owner: row.owner,
                collaborators: Vec::new(),
            })
            .collect(),
    ))
}

/// The blobs a delta needs.
fn blob_of(delta: &Delta) -> Option<BlobHash> {
    match &delta.op {
        Op::Create { content: Content::Blob(hash), .. }
        | Op::SetContent { content: Content::Blob(hash), .. }
        | Op::Conv(ConvOp::Attachment { blob: hash, .. } | ConvOp::SessionTranslated { lines: hash, .. })
        | Op::Git(GitOp::Bundle { blob: hash, .. }) => Some(*hash),
        _ => None,
    }
}

impl Server {
    /// Thread ids deleted and purged, for `viewer`'s listing: those in
    /// families they see, and purged ones whose family is gone altogether,
    /// so a device that had it still drops it.
    pub(crate) async fn lifecycle(&self, viewer: &Viewer) -> Result<(Vec<ThreadId>, Vec<ThreadId>), sqlx::Error> {
        let deleted = sqlx::query!("SELECT id, family_id FROM threads WHERE deleted_ms IS NOT NULL")
            .fetch_all(self.db())
            .await?
            .iter()
            .filter(|row| viewer.sees_family(id_from(&row.family_id)))
            .map(|row| ThreadId(id_from(&row.id)))
            .collect();
        let purged = sqlx::query!(
            r#"SELECT p.thread_id, p.family_id, EXISTS (SELECT 1 FROM families f WHERE f.id = p.family_id) AS "remains!: bool"
             FROM purged_threads p"#
        )
        .fetch_all(self.db())
        .await?
        .iter()
        .filter(|row| !row.remains || viewer.sees_family(id_from(&row.family_id)))
        .map(|row| ThreadId(id_from(&row.thread_id)))
        .collect();
        Ok((deleted, purged))
    }

    /// Removes threads for good: their metadata, the deltas no remaining
    /// thread builds on, families left empty with their git mirrors, and
    /// blobs nothing references any more.
    pub(crate) async fn purge(&self, threads: &[(ThreadId, FamilyId)]) -> Result<(), ServerError> {
        if threads.is_empty() {
            return Ok(());
        }
        let db = self.db();
        // Every room, as the index is rebuilt from all of them.
        let mut rooms = self.shared.rooms.write().await;
        let mut ordered: Vec<(FamilyId, SharedRoom)> = rooms.iter().map(|(family, room)| (*family, Arc::clone(room))).collect();
        ordered.sort_by_key(|(family, _)| *family);
        let mut slots: BTreeMap<FamilyId, OwnedMutexGuard<RoomSlot>> = BTreeMap::new();
        for (family, room) in ordered {
            slots.insert(family, room.lock_owned().await);
        }
        let mut purged = self.shared.purged.lock().await;
        let _indexing = self.shared.indexing.lock().await;
        let mut now_purged = purged.clone();
        now_purged.extend(threads.iter().map(|(thread, _)| *thread));
        // What changes in memory is decided here and done only after the
        // commit: `None` drops the family's room.
        let mut rebuilt: Vec<(FamilyId, Option<Room>)> = Vec::new();
        let mut tx = crate::write_transaction(db).await?;
        for (thread, family) in threads {
            let (thread, family, now) = (id_bytes(thread.0), id_bytes(family.0), now_ms());
            sqlx::query!(
                "INSERT OR IGNORE INTO purged_threads (thread_id, family_id, purged_ms) VALUES (?, ?, ?)",
                thread,
                family,
                now,
            )
            .execute(&mut *tx)
            .await?;
            sqlx::query!("DELETE FROM leases WHERE thread_id = ?", thread).execute(&mut *tx).await?;
            sqlx::query!("DELETE FROM notifications WHERE thread_id = ?", thread).execute(&mut *tx).await?;
            sqlx::query!("DELETE FROM thread_state WHERE thread_id = ?", thread).execute(&mut *tx).await?;
            sqlx::query!("DELETE FROM touches WHERE thread_id = ?", thread).execute(&mut *tx).await?;
            sqlx::query!("DELETE FROM thread_activity WHERE thread_id = ?", thread).execute(&mut *tx).await?;
            sqlx::query!("DELETE FROM search WHERE thread_id = ?", thread).execute(&mut *tx).await?;
            sqlx::query!("DELETE FROM artifact_grants WHERE thread_id = ?", thread).execute(&mut *tx).await?;
            sqlx::query!("DELETE FROM artifact_pins WHERE thread_id = ?", thread).execute(&mut *tx).await?;
            sqlx::query!("DELETE FROM threads WHERE id = ?", thread).execute(&mut *tx).await?;
        }
        let families: HashSet<FamilyId> = threads.iter().map(|(_, family)| *family).collect();
        let mut gone = Vec::new();
        for family in families {
            let family_id = id_bytes(family.0);
            let remaining =
                sqlx::query_scalar!("SELECT COUNT(*) FROM threads WHERE family_id = ?", family_id).fetch_one(&mut *tx).await?;
            // A fork whose device hasn't registered it yet still has deltas
            // here, and keeps the family alive.
            let kept: Vec<Delta> = slots
                .get(&family)
                .map(|slot| &slot.room)
                .map(|room| {
                    room.deltas()
                        .iter()
                        .zip(kept_after_purge(room.deltas(), &now_purged))
                        .filter(|(_, kept)| *kept)
                        .map(|(delta, _)| delta.clone())
                        .collect()
                })
                .unwrap_or_default();
            sqlx::query!("DELETE FROM deltas WHERE family_id = ?", family_id).execute(&mut *tx).await?;
            if remaining == 0 && kept.is_empty() {
                sqlx::query!("DELETE FROM families WHERE id = ?", family_id).execute(&mut *tx).await?;
                rebuilt.push((family, None));
                gone.push(family);
                continue;
            }
            if !slots.contains_key(&family) {
                continue;
            }
            let mut room = Room::new(self.shared.generation);
            if !kept.is_empty() {
                room.push(kept.clone())?;
            }
            crate::insert_deltas(&mut tx, &family_id, 1, &kept).await?;
            rebuilt.push((family, Some(room)));
        }
        tx.commit().await?;
        *purged = now_purged;
        for (family, room) in rebuilt {
            match (room, slots.get_mut(&family)) {
                (Some(room), Some(slot)) => {
                    slot.room = room;
                    slot.views.clear();
                    slot.browsed.clear();
                }
                (Some(_), None) => {}
                (None, slot) => {
                    if let Some(slot) = slot {
                        slot.retired = true;
                    }
                    rooms.remove(&family);
                }
            }
        }
        let mut index = Index::default();
        let mut rows = Vec::new();
        let mut referenced = HashSet::new();
        for (family, slot) in slots.iter().filter(|(_, slot)| !slot.retired) {
            for delta in slot.room.deltas() {
                rows.extend(index.observe(*family, delta));
                referenced.extend(blob_of(delta));
            }
        }
        let mut tx = crate::write_transaction(db).await?;
        index::clear(&mut tx).await?;
        index::write(&mut tx, &rows).await?;
        tx.commit().await?;
        {
            let mut current = self.shared.index.lock().await;
            index.forks_version = current.forks_version + 1;
            *current = index;
        }
        drop(purged);
        drop(slots);
        drop(rooms);
        for family in threads.iter().map(|(_, family)| *family).collect::<HashSet<_>>() {
            self.wake_pulls(family);
        }
        for family in gone {
            let mirror = self.git_root().join(format!("{family}.git"));
            if mirror.exists() {
                tokio::fs::remove_dir_all(mirror).await?;
            }
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
        let rows = sqlx::query!("SELECT id, family_id FROM threads WHERE deleted_ms IS NOT NULL AND deleted_ms < ?", cutoff)
            .fetch_all(self.db())
            .await?;
        let expired: Vec<(ThreadId, FamilyId)> =
            rows.iter().map(|row| (ThreadId(id_from(&row.id)), FamilyId(id_from(&row.family_id)))).collect();
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
                if let Err(error) = crate::accounts::sweep_expired(server.db()).await {
                    tracing::warn!(%error, "could not remove expired sign-ins");
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
    Ok(sqlx::query_scalar!("SELECT thread_id FROM purged_threads")
        .fetch_all(db)
        .await?
        .iter()
        .map(|thread| ThreadId(id_from(thread)))
        .collect())
}
