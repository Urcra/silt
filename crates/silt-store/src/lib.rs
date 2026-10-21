mod blobs;
mod chunks;
mod queue;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use silt_log::Delta;
use silt_types::{ActorId, BlobHash, DeltaId, FamilyId, Frontier, Lamport, ThreadId};
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};

pub use blobs::BlobStore;
pub use queue::QueuedPrompt;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error")]
    Database(#[from] sqlx::Error),
    #[error("could not migrate the database")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("stored record could not be decoded")]
    Decode(#[from] postcard::Error),
    #[error(transparent)]
    Delta(#[from] silt_log::StoredError),
    #[error("blob store I/O error")]
    Io(#[from] std::io::Error),
    #[error("stored {what} has an invalid length")]
    Corrupt { what: &'static str },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadRecord {
    pub id: ThreadId,
    pub family: FamilyId,
    /// The local clone; empty for threads only known from the server.
    pub repo_root: PathBuf,
    /// The repository's normalized origin URL.
    pub repo: String,
    pub created_ms: i64,
    /// The owning user's name on the server; empty when unknown.
    pub owner: String,
}

/// How far a family has been synced with the server.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncState {
    pub generation: Option<u64>,
    pub pushed: u64,
    pub pulled: u64,
    pub last_pulled: Option<DeltaId>,
    /// The server's fingerprint of what this user may see of the family.
    pub view: u64,
}

/// A seen mark on a merge request's file, by the request's `DeltaId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenRecord {
    pub mr: String,
    pub path: String,
    pub hash: String,
    pub seen: bool,
    pub at_ms: i64,
    pub pending: bool,
}

/// How the agent the web UI last ran in a thread stands (ADR 0022); its
/// state, preset and mode by name, as the daemon spells them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRunRecord {
    pub agent: String,
    pub state: String,
    pub error: Option<String>,
    pub preset: String,
    pub mode: Option<String>,
    pub updated_ms: i64,
}

/// A family's encoded projection as of its first `covers` deltas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub format: i64,
    pub covers: u64,
    pub data: Vec<u8>,
}

/// A checkout's row in this machine's store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CheckoutId(pub i64);

impl std::fmt::Display for CheckoutId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckoutRecord {
    pub id: CheckoutId,
    pub thread: ThreadId,
    pub path: PathBuf,
    pub actor: ActorId,
    pub managed: bool,
}

/// What a checkout last read from or wrote to disk for one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskFile {
    /// The id of the delta that created the file's inode.
    pub inode: DeltaId,
    pub hash: BlobHash,
    pub size: u64,
    pub mtime_ns: i64,
    pub version: Frontier,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub agent: String,
    pub session_id: String,
    pub thread: ThreadId,
    pub checkout_id: CheckoutId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFileOffset {
    pub file: String,
    pub path: PathBuf,
    pub offset: u64,
}

/// Text of a delta to find by full-text search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRow {
    pub thread: ThreadId,
    pub delta: DeltaId,
    pub kind: String,
    pub body: String,
}

/// How far a session file has been read with the deltas stored alongside.
#[derive(Debug, Clone, Copy)]
pub struct SessionRead<'a> {
    pub agent: &'a str,
    pub session_id: &'a str,
    pub offset: &'a SessionFileOffset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub thread: ThreadId,
    pub delta: DeltaId,
    pub kind: String,
    pub snippet: String,
}

/// The local database and blob store.
#[derive(Debug, Clone)]
pub struct Store {
    pool: SqlitePool,
    blobs: BlobStore,
    watch: Arc<tokio::sync::Mutex<Watch>>,
}

/// A connection of its own for telling whether the database changed: what
/// SQLite reports there counts every other connection's commits.
#[derive(Debug)]
struct Watch {
    connection: sqlx::SqliteConnection,
    seen: Option<i64>,
}

impl Store {
    /// Every transaction here writes. Taking the write lock at the start
    /// makes a concurrent writer wait for it; a transaction that reads
    /// first would instead fail when it tries to write after another one
    /// committed.
    async fn write_transaction(&self) -> Result<sqlx::Transaction<'static, sqlx::Sqlite>, StoreError> {
        Ok(self.pool.begin_with("BEGIN IMMEDIATE").await?)
    }

    /// Opens (creating if needed) the store in `dir` and migrates it.
    pub async fn open(dir: &Path) -> Result<Self, StoreError> {
        tokio::fs::create_dir_all(dir).await?;
        let options = SqliteConnectOptions::from_str("sqlite://")?
            .filename(dir.join("silt.sqlite"))
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(10))
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new().max_connections(4).connect_with(options.clone()).await?;
        MIGRATOR.run(&pool).await?;
        let watch = Watch { connection: sqlx::ConnectOptions::connect(&options).await?, seen: None };
        Ok(Self { pool, blobs: BlobStore::new(dir.join("blobs")), watch: Arc::new(tokio::sync::Mutex::new(watch)) })
    }

    /// Whether anyone committed to the database since the last call, this
    /// process too through its other connections; true the first time.
    pub async fn changed(&self) -> Result<bool, StoreError> {
        let mut watch = self.watch.lock().await;
        // A pragma, which sqlx's macros can't describe.
        let version: i64 = sqlx::query_scalar("PRAGMA data_version").fetch_one(&mut watch.connection).await?;
        Ok(watch.seen.replace(version) != Some(version))
    }

    /// The position of each family's last stored delta.
    pub async fn positions(&self) -> Result<HashMap<FamilyId, i64>, StoreError> {
        let rows = sqlx::query!("SELECT family_id, seq FROM family_positions").fetch_all(&self.pool).await?;
        rows.into_iter().map(|row| Ok((FamilyId(id_from(&row.family_id)?), row.seq))).collect()
    }

    pub async fn position(&self, family: FamilyId) -> Result<Option<i64>, StoreError> {
        let family = id_bytes(family.0);
        Ok(sqlx::query_scalar!("SELECT seq FROM family_positions WHERE family_id = ?", family).fetch_optional(&self.pool).await?)
    }

    /// The family's deltas stored after position `after`, in order.
    pub async fn deltas_after(&self, family: FamilyId, after: i64) -> Result<Vec<Delta>, StoreError> {
        let family = id_bytes(family.0);
        let rows = sqlx::query!("SELECT format, data FROM deltas WHERE family_id = ? AND seq > ? ORDER BY seq", family, after)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(|row| Ok(silt_log::decode_stored(row.format, &row.data)?)).collect()
    }

    pub fn blobs(&self) -> &BlobStore {
        &self.blobs
    }

    pub async fn create_thread(&self, thread: &ThreadRecord) -> Result<(), StoreError> {
        let mut tx = self.write_transaction().await?;
        let (id, family) = (id_bytes(thread.id.0), id_bytes(thread.family.0));
        sqlx::query!("INSERT OR IGNORE INTO families (id, created_ms) VALUES (?, ?)", family, thread.created_ms)
            .execute(&mut *tx)
            .await?;
        let repo_root = path_text(&thread.repo_root);
        sqlx::query!(
            "INSERT INTO threads (id, family_id, repo_root, repo, created_ms, owner) VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT (id) DO UPDATE SET
                 repo_root = CASE WHEN excluded.repo_root != '' THEN excluded.repo_root ELSE threads.repo_root END,
                 owner = CASE WHEN excluded.owner != '' THEN excluded.owner ELSE threads.owner END",
            id,
            family,
            repo_root,
            thread.repo,
            thread.created_ms,
            thread.owner,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn threads(&self) -> Result<Vec<ThreadRecord>, StoreError> {
        let rows = sqlx::query!("SELECT id, family_id, repo_root, repo, created_ms, owner FROM threads ORDER BY created_ms")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ThreadRecord {
                    id: ThreadId(id_from(&row.id)?),
                    family: FamilyId(id_from(&row.family_id)?),
                    repo_root: PathBuf::from(row.repo_root),
                    repo: row.repo,
                    created_ms: row.created_ms,
                    owner: row.owner,
                })
            })
            .collect()
    }

    pub async fn sync_state(&self, family: FamilyId) -> Result<SyncState, StoreError> {
        let family = id_bytes(family.0);
        let row =
            sqlx::query!("SELECT generation, pushed, pulled, last_pulled, view FROM sync_state WHERE family_id = ?", family)
                .fetch_optional(&self.pool)
                .await?;
        let Some(row) = row else {
            return Ok(SyncState::default());
        };
        Ok(SyncState {
            generation: row.generation.map(i64::cast_unsigned),
            pushed: u64::try_from(row.pushed).unwrap_or_default(),
            pulled: u64::try_from(row.pulled).unwrap_or_default(),
            last_pulled: match row.last_pulled {
                Some(bytes) => Some(postcard::from_bytes(&bytes)?),
                None => None,
            },
            view: row.view.cast_unsigned(),
        })
    }

    pub async fn set_sync_state(&self, family: FamilyId, state: &SyncState) -> Result<(), StoreError> {
        let last_pulled = match state.last_pulled {
            Some(id) => Some(postcard::to_allocvec(&id)?),
            None => None,
        };
        let family = id_bytes(family.0);
        let generation = state.generation.map(u64::cast_signed);
        let (pushed, pulled) = (i64::try_from(state.pushed).unwrap_or(i64::MAX), i64::try_from(state.pulled).unwrap_or(i64::MAX));
        let view = state.view.cast_signed();
        sqlx::query!(
            "INSERT INTO sync_state (family_id, generation, pushed, pulled, last_pulled, view) VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT (family_id) DO UPDATE SET generation = excluded.generation, pushed = excluded.pushed,
                 pulled = excluded.pulled, last_pulled = excluded.last_pulled, view = excluded.view",
            family,
            generation,
            pushed,
            pulled,
            last_pulled,
            view,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn families(&self) -> Result<Vec<FamilyId>, StoreError> {
        let rows = sqlx::query!("SELECT id FROM families ORDER BY created_ms").fetch_all(&self.pool).await?;
        rows.into_iter().map(|row| Ok(FamilyId(id_from(&row.id)?))).collect()
    }

    /// Appends deltas after the family's existing ones, keeping their order.
    pub async fn append_deltas(&self, family: FamilyId, deltas: &[Delta]) -> Result<(), StoreError> {
        let mut tx = self.write_transaction().await?;
        append_deltas_in(&mut tx, family, deltas).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Forgets the family's deltas and sync progress, to read it again from
    /// the server.
    pub async fn clear_deltas(&self, family: FamilyId) -> Result<(), StoreError> {
        let mut tx = self.write_transaction().await?;
        clear_deltas_in(&mut tx, family).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Replaces the family's deltas with `deltas`, after some were purged or
    /// the server's history replaced this machine's, and starts its sync
    /// over. Either all of it happens or none.
    pub async fn replace_deltas(&self, family: FamilyId, deltas: &[Delta]) -> Result<(), StoreError> {
        let mut tx = self.write_transaction().await?;
        clear_deltas_in(&mut tx, family).await?;
        append_deltas_in(&mut tx, family, deltas).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Marks a thread as in the trash (`Some`) or back out of it (`None`).
    pub async fn set_thread_deleted(&self, thread: ThreadId, deleted_ms: Option<i64>) -> Result<(), StoreError> {
        let thread = id_bytes(thread.0);
        sqlx::query!("UPDATE threads SET deleted_ms = ? WHERE id = ?", deleted_ms, thread).execute(&self.pool).await?;
        Ok(())
    }

    /// Remembers secrets this machine redacts, so they stay redacted after
    /// they leave its environment (ADR 0019).
    pub async fn learn_secrets(&self, secrets: &[String]) -> Result<(), StoreError> {
        let mut tx = self.write_transaction().await?;
        for secret in secrets {
            sqlx::query!("INSERT INTO secret_overlay (secret) VALUES (?) ON CONFLICT DO NOTHING", secret)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn secrets(&self) -> Result<Vec<String>, StoreError> {
        Ok(sqlx::query_scalar!("SELECT secret FROM secret_overlay").fetch_all(&self.pool).await?)
    }

    pub async fn remember_clone(&self, repo: &str, path: &Path) -> Result<(), StoreError> {
        let path = path_text(path);
        sqlx::query!(
            "INSERT INTO clones (repo, path) VALUES (?, ?) ON CONFLICT (repo) DO UPDATE SET path = excluded.path",
            repo,
            path
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Clones remembered by repository.
    pub async fn clones(&self) -> Result<Vec<(String, PathBuf)>, StoreError> {
        Ok(sqlx::query!("SELECT repo, path FROM clones")
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|row| (row.repo, PathBuf::from(row.path)))
            .collect())
    }

    /// Where each family syncs, for those given a home (ADR 0032): a
    /// server's URL, or empty for this machine only.
    pub async fn family_homes(&self) -> Result<Vec<(FamilyId, String)>, StoreError> {
        sqlx::query!("SELECT family_id, home FROM family_homes")
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|row| Ok((FamilyId(id_from(&row.family_id)?), row.home)))
            .collect()
    }

    pub async fn set_family_home(&self, family: FamilyId, home: &str) -> Result<(), StoreError> {
        let id = id_bytes(family.0);
        sqlx::query!(
            "INSERT INTO family_homes (family_id, home) VALUES (?, ?) ON CONFLICT (family_id) DO UPDATE SET home = excluded.home",
            id,
            home
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Where each project's new families go, by repository.
    pub async fn project_homes(&self) -> Result<Vec<(String, String)>, StoreError> {
        Ok(sqlx::query!("SELECT repo, home FROM project_homes")
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|row| (row.repo, row.home))
            .collect())
    }

    pub async fn set_project_home(&self, repo: &str, home: &str) -> Result<(), StoreError> {
        sqlx::query!(
            "INSERT INTO project_homes (repo, home) VALUES (?, ?) ON CONFLICT (repo) DO UPDATE SET home = excluded.home",
            repo,
            home
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn deleted_threads(&self) -> Result<Vec<ThreadId>, StoreError> {
        sqlx::query_scalar!("SELECT id FROM threads WHERE deleted_ms IS NOT NULL")
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(|id| Ok(ThreadId(id_from(id)?)))
            .collect()
    }

    /// Forgets a thread here: its sessions, checkouts and search entries.
    /// Files on disk stay.
    pub async fn remove_thread(&self, thread: ThreadId) -> Result<(), StoreError> {
        let id = id_bytes(thread.0);
        let mut tx = self.write_transaction().await?;
        // Sessions go by their thread and by their checkout: a checkout
        // rebound to this thread may still hold sessions of another.
        sqlx::query!(
            "DELETE FROM session_files WHERE (agent, session_id) IN (SELECT agent, session_id FROM sessions
             WHERE thread_id = ?1 OR checkout_id IN (SELECT id FROM checkouts WHERE thread_id = ?1))",
            id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "DELETE FROM sessions WHERE thread_id = ?1 OR checkout_id IN (SELECT id FROM checkouts WHERE thread_id = ?1)",
            id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!("DELETE FROM disk_files WHERE checkout_id IN (SELECT id FROM checkouts WHERE thread_id = ?1)", id)
            .execute(&mut *tx)
            .await?;
        sqlx::query!("DELETE FROM checkouts WHERE thread_id = ?1", id).execute(&mut *tx).await?;
        sqlx::query!("DELETE FROM search WHERE rowid IN (SELECT row_id FROM search_threads WHERE thread_id = ?1)", id)
            .execute(&mut *tx)
            .await?;
        sqlx::query!("DELETE FROM search_threads WHERE thread_id = ?1", id).execute(&mut *tx).await?;
        sqlx::query!("DELETE FROM thread_state WHERE thread_id = ?1", id).execute(&mut *tx).await?;
        sqlx::query!("DELETE FROM threads WHERE id = ?1", id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// This person's state of each thread, as JSON, and whether the server
    /// has yet to hear of it (ADR 0024).
    pub async fn thread_states(&self) -> Result<Vec<(ThreadId, String, bool)>, StoreError> {
        let rows = sqlx::query!("SELECT thread_id, state, pending FROM thread_state").fetch_all(&self.pool).await?;
        rows.into_iter().map(|row| Ok((ThreadId(id_from(&row.thread_id)?), row.state, row.pending != 0))).collect()
    }

    /// This person's seen marks on merge request files, and whether the
    /// server has yet to hear of each (ADR 0024).
    pub async fn seen_marks(&self) -> Result<Vec<SeenRecord>, StoreError> {
        let rows = sqlx::query!("SELECT mr_id, path, hash, seen, at_ms, pending FROM seen_marks").fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|row| SeenRecord {
                mr: row.mr_id,
                path: row.path,
                hash: row.hash,
                seen: row.seen != 0,
                at_ms: row.at_ms,
                pending: row.pending != 0,
            })
            .collect())
    }

    pub async fn put_seen_mark(&self, mark: &SeenRecord) -> Result<(), StoreError> {
        sqlx::query!(
            "INSERT INTO seen_marks (mr_id, path, hash, seen, at_ms, pending) VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT (mr_id, path) DO UPDATE SET hash = excluded.hash, seen = excluded.seen, at_ms = excluded.at_ms,
                 pending = excluded.pending",
            mark.mr,
            mark.path,
            mark.hash,
            mark.seen,
            mark.at_ms,
            mark.pending,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn put_thread_state(&self, thread: ThreadId, state: &str, pending: bool) -> Result<(), StoreError> {
        let thread = id_bytes(thread.0);
        sqlx::query!(
            "INSERT INTO thread_state (thread_id, state, pending) VALUES (?, ?, ?)
             ON CONFLICT (thread_id) DO UPDATE SET state = excluded.state, pending = excluded.pending",
            thread,
            state,
            pending,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Keeps a message for the agent in `thread`, for its next prompt.
    pub async fn add_notice(&self, thread: ThreadId, message: &str, now_ms: i64) -> Result<(), StoreError> {
        let thread = id_bytes(thread.0);
        sqlx::query!("INSERT INTO notices (thread_id, message, created_ms) VALUES (?, ?, ?)", thread, message, now_ms)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The messages waiting for the agent in `thread`, oldest first, taken
    /// so that only one prompt hands them over.
    pub async fn take_notices(&self, thread: ThreadId) -> Result<Vec<String>, StoreError> {
        let thread = id_bytes(thread.0);
        let mut taken = sqlx::query!(r#"DELETE FROM notices WHERE thread_id = ? RETURNING id AS "id!", message"#, thread)
            .fetch_all(&self.pool)
            .await?;
        taken.sort_by_key(|notice| notice.id);
        Ok(taken.into_iter().map(|notice| notice.message).collect())
    }

    pub async fn agent_runs(&self) -> Result<HashMap<ThreadId, AgentRunRecord>, StoreError> {
        let rows = sqlx::query!("SELECT thread_id, agent, state, error, preset, mode, updated_ms FROM agent_runs")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|row| {
                let record = AgentRunRecord {
                    agent: row.agent,
                    state: row.state,
                    error: row.error,
                    preset: row.preset,
                    mode: row.mode,
                    updated_ms: row.updated_ms,
                };
                Ok((ThreadId(id_from(&row.thread_id)?), record))
            })
            .collect()
    }

    pub async fn put_agent_run(&self, thread: ThreadId, run: &AgentRunRecord) -> Result<(), StoreError> {
        let thread = id_bytes(thread.0);
        sqlx::query!(
            "INSERT INTO agent_runs (thread_id, agent, state, error, preset, mode, updated_ms) VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (thread_id) DO UPDATE SET agent = excluded.agent, state = excluded.state, error = excluded.error,
                 preset = excluded.preset, mode = excluded.mode, updated_ms = excluded.updated_ms",
            thread,
            run.agent,
            run.state,
            run.error,
            run.preset,
            run.mode,
            run.updated_ms,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The family's projection snapshot, if one was saved since its deltas
    /// were last replaced.
    pub async fn snapshot(&self, family: FamilyId) -> Result<Option<Snapshot>, StoreError> {
        let family = id_bytes(family.0);
        let row = sqlx::query!("SELECT format, covers, data FROM family_snapshots WHERE family_id = ?", family)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|row| Snapshot { format: row.format, covers: u64::try_from(row.covers).unwrap_or_default(), data: row.data }))
    }

    pub async fn put_snapshot(&self, family: FamilyId, snapshot: &Snapshot) -> Result<(), StoreError> {
        let family = id_bytes(family.0);
        let covers = i64::try_from(snapshot.covers).unwrap_or(i64::MAX);
        sqlx::query!(
            "INSERT INTO family_snapshots (family_id, format, covers, data) VALUES (?, ?, ?, ?)
             ON CONFLICT (family_id) DO UPDATE SET format = excluded.format, covers = excluded.covers, data = excluded.data",
            family,
            snapshot.format,
            covers,
            snapshot.data,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Every delta of the family, in the order they were appended.
    pub async fn deltas(&self, family: FamilyId) -> Result<Vec<Delta>, StoreError> {
        let family = id_bytes(family.0);
        let chunks = sqlx::query!("SELECT last_seq, data FROM delta_chunks WHERE family_id = ? ORDER BY first_seq", family)
            .fetch_all(&self.pool)
            .await?;
        let packed = chunks.last().map_or(-1, |chunk| chunk.last_seq);
        let rows = sqlx::query!("SELECT format, data FROM deltas WHERE family_id = ? AND seq > ? ORDER BY seq", family, packed)
            .fetch_all(&self.pool)
            .await?;
        let mut deltas = Vec::new();
        for chunk in &chunks {
            for (format, data) in chunks::unpack(&chunk.data)? {
                deltas.push(silt_log::decode_stored(format, data)?);
            }
        }
        for row in rows {
            deltas.push(silt_log::decode_stored(row.format, &row.data)?);
        }
        Ok(deltas)
    }

    pub async fn add_checkout(
        &self,
        thread: ThreadId,
        path: &Path,
        actor: ActorId,
        managed: bool,
        now_ms: i64,
    ) -> Result<CheckoutRecord, StoreError> {
        let (thread_id, path_text, actor_id) = (id_bytes(thread.0), path_text(path), id_bytes(actor.0));
        let id = sqlx::query!(
            "INSERT INTO checkouts (thread_id, path, actor, managed, created_ms) VALUES (?, ?, ?, ?, ?)",
            thread_id,
            path_text,
            actor_id,
            managed,
            now_ms,
        )
        .execute(&self.pool)
        .await?
        .last_insert_rowid();
        Ok(CheckoutRecord { id: CheckoutId(id), thread, path: path.to_owned(), actor, managed })
    }

    /// Hands an existing checkout to another thread, forgetting what it
    /// recorded about the disk for the previous one.
    pub async fn rebind_checkout(&self, checkout: CheckoutId, thread: ThreadId, actor: ActorId) -> Result<(), StoreError> {
        let mut tx = self.write_transaction().await?;
        let (checkout, thread, actor) = (checkout.0, id_bytes(thread.0), id_bytes(actor.0));
        sqlx::query!("UPDATE checkouts SET thread_id = ?, actor = ? WHERE id = ?", thread, actor, checkout)
            .execute(&mut *tx)
            .await?;
        sqlx::query!("DELETE FROM disk_files WHERE checkout_id = ?", checkout).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Points a checkout, and the sessions recorded in it, at another thread
    /// of the same family, keeping what it knows about the disk.
    pub async fn move_checkout(&self, checkout: CheckoutId, thread: ThreadId) -> Result<(), StoreError> {
        let mut tx = self.write_transaction().await?;
        let (checkout, thread) = (checkout.0, id_bytes(thread.0));
        sqlx::query!("UPDATE checkouts SET thread_id = ? WHERE id = ?", thread, checkout).execute(&mut *tx).await?;
        sqlx::query!("UPDATE sessions SET thread_id = ? WHERE checkout_id = ?", thread, checkout).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn checkouts(&self) -> Result<Vec<CheckoutRecord>, StoreError> {
        let rows =
            sqlx::query!(r#"SELECT id AS "id!", thread_id, path, actor, managed AS "managed: bool" FROM checkouts ORDER BY id"#)
                .fetch_all(&self.pool)
                .await?;
        rows.into_iter()
            .map(|row| {
                Ok(CheckoutRecord {
                    id: CheckoutId(row.id),
                    thread: ThreadId(id_from(&row.thread_id)?),
                    path: PathBuf::from(row.path),
                    actor: ActorId(id_from(&row.actor)?),
                    managed: row.managed,
                })
            })
            .collect()
    }

    pub async fn disk_files(&self, checkout: CheckoutId) -> Result<HashMap<String, DiskFile>, StoreError> {
        let checkout = checkout.0;
        // As one row: a large checkout's rows one by one take a hook capturing
        // on its own a quarter of its budget (ADR 0018).
        let rows = sqlx::query_scalar!(
            r#"SELECT json_group_array(json_array(path, hex(inode), hex(hash), size, mtime_ns, hex(version))) AS "rows!: String"
               FROM disk_files WHERE checkout_id = ?"#,
            checkout
        )
        .fetch_one(&self.pool)
        .await?;
        let rows: Vec<(String, String, String, i64, i64, String)> =
            serde_json::from_str(&rows).map_err(|_| StoreError::Corrupt { what: "disk state" })?;
        rows.into_iter()
            .map(|(path, inode, hash, size, mtime_ns, version)| {
                let hash = unhex(&hash)?;
                Ok((
                    path,
                    DiskFile {
                        inode: postcard::from_bytes(&unhex(&inode)?)?,
                        hash: BlobHash(hash.as_slice().try_into().map_err(|_| StoreError::Corrupt { what: "hash" })?),
                        size: u64::try_from(size).unwrap_or_default(),
                        mtime_ns,
                        version: postcard::from_bytes(&unhex(&version)?)?,
                    },
                ))
            })
            .collect()
    }

    pub async fn put_disk_file(&self, checkout: CheckoutId, path: &str, file: &DiskFile) -> Result<(), StoreError> {
        let checkout = checkout.0;
        let (inode, hash, version) =
            (postcard::to_allocvec(&file.inode)?, file.hash.0.as_slice(), postcard::to_allocvec(&file.version)?);
        let size = i64::try_from(file.size).unwrap_or(i64::MAX);
        sqlx::query!(
            "INSERT INTO disk_files (checkout_id, path, inode, hash, size, mtime_ns, version) VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (checkout_id, path) DO UPDATE SET inode = excluded.inode,
                 hash = excluded.hash, size = excluded.size, mtime_ns = excluded.mtime_ns, version = excluded.version",
            checkout,
            path,
            inode,
            hash,
            size,
            file.mtime_ns,
            version,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn remove_disk_file(&self, checkout: CheckoutId, path: &str) -> Result<(), StoreError> {
        let checkout = checkout.0;
        sqlx::query!("DELETE FROM disk_files WHERE checkout_id = ? AND path = ?", checkout, path).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn add_session(&self, session: &SessionRecord, now_ms: i64) -> Result<(), StoreError> {
        let (thread, checkout) = (id_bytes(session.thread.0), session.checkout_id.0);
        sqlx::query!(
            "INSERT INTO sessions (agent, session_id, thread_id, checkout_id, created_ms) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT (agent, session_id) DO NOTHING",
            session.agent,
            session.session_id,
            thread,
            checkout,
            now_ms,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn sessions(&self) -> Result<Vec<SessionRecord>, StoreError> {
        let rows = sqlx::query!("SELECT agent, session_id, thread_id, checkout_id FROM sessions ORDER BY created_ms")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|row| {
                Ok(SessionRecord {
                    agent: row.agent,
                    session_id: row.session_id,
                    thread: ThreadId(id_from(&row.thread_id)?),
                    checkout_id: CheckoutId(row.checkout_id),
                })
            })
            .collect()
    }

    pub async fn session_offsets(&self, agent: &str, session_id: &str) -> Result<Vec<SessionFileOffset>, StoreError> {
        let rows =
            sqlx::query!("SELECT file, path, offset FROM session_files WHERE agent = ? AND session_id = ?", agent, session_id)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows
            .into_iter()
            .map(|row| SessionFileOffset {
                file: row.file,
                path: PathBuf::from(row.path),
                offset: u64::try_from(row.offset).unwrap_or_default(),
            })
            .collect())
    }

    pub async fn set_session_offset(&self, agent: &str, session_id: &str, file: &SessionFileOffset) -> Result<(), StoreError> {
        let mut tx = self.write_transaction().await?;
        set_session_offset_in(&mut tx, agent, session_id, file).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Appends deltas with their search rows and, for deltas recorded from
    /// a session file, how far the file has been read: all of it or none,
    /// so nothing is indexed or recorded twice.
    pub async fn append_deltas_with(
        &self,
        family: FamilyId,
        deltas: &[Delta],
        rows: &[SearchRow],
        read: Option<SessionRead<'_>>,
    ) -> Result<(), StoreError> {
        let mut tx = self.write_transaction().await?;
        append_deltas_in(&mut tx, family, deltas).await?;
        for row in rows {
            let (thread, actor, lamport) = (id_bytes(row.thread.0), id_bytes(row.delta.actor.0), lamport_i64(row.delta.lamport));
            let inserted = sqlx::query!(
                "INSERT INTO search (body, thread_id, actor, lamport, kind) VALUES (?, ?, ?, ?, ?)",
                row.body,
                thread,
                actor,
                lamport,
                row.kind,
            )
            .execute(&mut *tx)
            .await?
            .last_insert_rowid();
            sqlx::query!("INSERT INTO search_threads (row_id, thread_id) VALUES (?, ?)", inserted, thread)
                .execute(&mut *tx)
                .await?;
        }
        if let Some(read) = read {
            set_session_offset_in(&mut tx, read.agent, read.session_id, read.offset).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Full-text search; `query` uses SQLite FTS5 syntax and words are
    /// matched as prefixes.
    pub async fn search(&self, query: &str, limit: u32) -> Result<Vec<SearchHit>, StoreError> {
        let query = fts_query(query);
        if query.is_empty() {
            return Ok(Vec::new());
        }
        // Checked at runtime: sqlx can't describe an FTS5 match, and its
        // macro crashes the compiler on this one.
        let rows = sqlx::query(
            "SELECT thread_id, actor, lamport, kind, snippet(search, 0, '[', ']', '…', 12) AS snippet
             FROM search WHERE search MATCH ? ORDER BY rank LIMIT ?",
        )
        .bind(query)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(SearchHit {
                    thread: ThreadId(id_from(row.get("thread_id"))?),
                    delta: DeltaId::new(
                        ActorId(id_from(row.get("actor"))?),
                        Lamport(u64::try_from(row.get::<i64, _>("lamport")).unwrap_or_default()),
                    ),
                    kind: row.get("kind"),
                    snippet: row.get("snippet"),
                })
            })
            .collect()
    }
}

async fn append_deltas_in(tx: &mut sqlx::SqliteConnection, family: FamilyId, deltas: &[Delta]) -> Result<(), StoreError> {
    let family = id_bytes(family.0);
    let next = sqlx::query_scalar!(r#"SELECT COALESCE(MAX(seq) + 1, 0) AS "next!: i64" FROM deltas WHERE family_id = ?"#, family)
        .fetch_one(&mut *tx)
        .await?;
    for (seq, delta) in (next..).zip(deltas) {
        let (actor, lamport, len) =
            (id_bytes(delta.span.actor.0), lamport_i64(delta.span.start), i64::try_from(delta.span.len).unwrap_or(i64::MAX));
        let (format, data) = (silt_log::DELTA_FORMAT, silt_log::encode_stored(delta)?);
        sqlx::query!(
            "INSERT INTO deltas (family_id, seq, actor, lamport, len, format, data) VALUES (?, ?, ?, ?, ?, ?, ?)",
            family,
            seq,
            actor,
            lamport,
            len,
            format,
            data,
        )
        .execute(&mut *tx)
        .await?;
    }
    pack_deltas_in(tx, &family).await
}

/// Packs the family's rows not in a chunk yet, a chunk's worth at a time.
async fn pack_deltas_in(tx: &mut sqlx::SqliteConnection, family: &[u8]) -> Result<(), StoreError> {
    let packed = sqlx::query_scalar!(
        r#"SELECT COALESCE(MAX(last_seq), -1) AS "packed!: i64" FROM delta_chunks WHERE family_id = ?"#,
        family
    )
    .fetch_one(&mut *tx)
    .await?;
    let unpacked =
        sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!: i64" FROM deltas WHERE family_id = ? AND seq > ?"#, family, packed)
            .fetch_one(&mut *tx)
            .await?;
    if usize::try_from(unpacked).unwrap_or_default() < chunks::CHUNK_ROWS {
        return Ok(());
    }
    let rows = sqlx::query!("SELECT seq, format, data FROM deltas WHERE family_id = ? AND seq > ? ORDER BY seq", family, packed)
        .fetch_all(&mut *tx)
        .await?;
    for chunk in rows.as_chunks::<{ chunks::CHUNK_ROWS }>().0 {
        let (first, last) = (chunk[0].seq, chunk[chunks::CHUNK_ROWS - 1].seq);
        let data = chunks::pack(chunk.iter().map(|row| (row.format, row.data.as_slice())));
        sqlx::query!(
            "INSERT INTO delta_chunks (family_id, first_seq, last_seq, data) VALUES (?, ?, ?, ?)",
            family,
            first,
            last,
            data
        )
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

async fn set_session_offset_in(
    tx: &mut sqlx::SqliteConnection,
    agent: &str,
    session_id: &str,
    file: &SessionFileOffset,
) -> Result<(), StoreError> {
    let (path, offset) = (path_text(&file.path), i64::try_from(file.offset).unwrap_or(i64::MAX));
    sqlx::query!(
        "INSERT INTO session_files (agent, session_id, file, path, offset) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT (agent, session_id, file) DO UPDATE SET path = excluded.path, offset = excluded.offset",
        agent,
        session_id,
        file.file,
        path,
        offset,
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Also forgets the family's search rows: the deltas they came from are
/// indexed again when they come back.
async fn clear_deltas_in(tx: &mut sqlx::SqliteConnection, family: FamilyId) -> Result<(), StoreError> {
    let family = id_bytes(family.0);
    sqlx::query!(
        "DELETE FROM search WHERE rowid IN (SELECT row_id FROM search_threads
         WHERE thread_id IN (SELECT id FROM threads WHERE family_id = ?1))",
        family
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!("DELETE FROM search_threads WHERE thread_id IN (SELECT id FROM threads WHERE family_id = ?1)", family)
        .execute(&mut *tx)
        .await?;
    sqlx::query!("DELETE FROM deltas WHERE family_id = ?1", family).execute(&mut *tx).await?;
    sqlx::query!("DELETE FROM delta_chunks WHERE family_id = ?1", family).execute(&mut *tx).await?;
    sqlx::query!("DELETE FROM family_snapshots WHERE family_id = ?1", family).execute(&mut *tx).await?;
    sqlx::query!("DELETE FROM sync_state WHERE family_id = ?1", family).execute(&mut *tx).await?;
    Ok(())
}

/// Turns free text into an FTS5 query of quoted prefix terms, so user input
/// can't be misread as query syntax.
fn fts_query(text: &str) -> String {
    text.split_whitespace().map(|word| format!("\"{}\"*", word.replace('"', "\"\""))).collect::<Vec<_>>().join(" ")
}

/// Bytes from SQLite's `hex()`.
fn unhex(text: &str) -> Result<Vec<u8>, StoreError> {
    let digit = |byte: u8| char::from(byte).to_digit(16).ok_or(StoreError::Corrupt { what: "hex" });
    text.as_bytes()
        .chunks(2)
        .map(|pair| match pair {
            [high, low] => Ok(u8::try_from(digit(*high)? << 4 | digit(*low)?).expect("two hex digits")),
            _ => Err(StoreError::Corrupt { what: "hex" }),
        })
        .collect()
}

fn id_bytes(id: u128) -> Vec<u8> {
    id.to_be_bytes().to_vec()
}

fn id_from(bytes: &[u8]) -> Result<u128, StoreError> {
    let bytes: [u8; 16] = bytes.try_into().map_err(|_| StoreError::Corrupt { what: "id" })?;
    Ok(u128::from_be_bytes(bytes))
}

fn lamport_i64(lamport: Lamport) -> i64 {
    i64::try_from(lamport.0).unwrap_or(i64::MAX)
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
