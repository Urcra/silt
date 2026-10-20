mod blobs;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use silt_log::Delta;
use silt_types::{ActorId, DeltaId, FamilyId, Frontier, Lamport, ThreadId};
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};

pub use blobs::BlobStore;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error")]
    Database(#[from] sqlx::Error),
    #[error("could not migrate the database")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("stored record could not be decoded")]
    Decode(#[from] postcard::Error),
    #[error("blob store I/O error")]
    Io(#[from] std::io::Error),
    #[error("stored {what} has an invalid length")]
    Corrupt { what: &'static str },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadRecord {
    pub id: ThreadId,
    pub family: FamilyId,
    pub repo_root: PathBuf,
    pub created_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckoutRecord {
    pub id: i64,
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
    pub hash: [u8; 32],
    pub size: u64,
    pub mtime_ns: i64,
    pub version: Frontier,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub agent: String,
    pub session_id: String,
    pub thread: ThreadId,
    pub checkout_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFileOffset {
    pub file: String,
    pub path: PathBuf,
    pub offset: u64,
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
}

impl Store {
    /// Opens (creating if needed) the store in `dir` and migrates it.
    pub async fn open(dir: &Path) -> Result<Self, StoreError> {
        tokio::fs::create_dir_all(dir).await?;
        let options = SqliteConnectOptions::from_str("sqlite://")?
            .filename(dir.join("silt.sqlite"))
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new().max_connections(4).connect_with(options).await?;
        MIGRATOR.run(&pool).await?;
        Ok(Self {
            pool,
            blobs: BlobStore::new(dir.join("blobs")),
        })
    }

    pub fn blobs(&self) -> &BlobStore {
        &self.blobs
    }

    pub async fn create_thread(&self, thread: &ThreadRecord) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT OR IGNORE INTO families (id, created_ms) VALUES (?, ?)")
            .bind(id_bytes(thread.family.0))
            .bind(thread.created_ms)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO threads (id, family_id, repo_root, created_ms) VALUES (?, ?, ?, ?)")
            .bind(id_bytes(thread.id.0))
            .bind(id_bytes(thread.family.0))
            .bind(path_text(&thread.repo_root))
            .bind(thread.created_ms)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn threads(&self) -> Result<Vec<ThreadRecord>, StoreError> {
        let rows = sqlx::query("SELECT id, family_id, repo_root, created_ms FROM threads ORDER BY created_ms")
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|row| {
                Ok(ThreadRecord {
                    id: ThreadId(id_from(row.get("id"))?),
                    family: FamilyId(id_from(row.get("family_id"))?),
                    repo_root: PathBuf::from(row.get::<String, _>("repo_root")),
                    created_ms: row.get("created_ms"),
                })
            })
            .collect()
    }

    pub async fn families(&self) -> Result<Vec<FamilyId>, StoreError> {
        let rows = sqlx::query("SELECT id FROM families ORDER BY created_ms")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(|row| Ok(FamilyId(id_from(row.get("id"))?))).collect()
    }

    /// Appends deltas after the family's existing ones, keeping their order.
    pub async fn append_deltas(&self, family: FamilyId, deltas: &[Delta]) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        let next: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(seq) + 1, 0) FROM deltas WHERE family_id = ?")
            .bind(id_bytes(family.0))
            .fetch_one(&mut *tx)
            .await?;
        for (seq, delta) in (next..).zip(deltas) {
            sqlx::query(
                "INSERT INTO deltas (family_id, seq, actor, lamport, len, data) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(id_bytes(family.0))
            .bind(seq)
            .bind(id_bytes(delta.span.actor.0))
            .bind(lamport_i64(delta.span.start))
            .bind(i64::try_from(delta.span.len).unwrap_or(i64::MAX))
            .bind(postcard::to_allocvec(delta)?)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Every delta of the family, in the order they were appended.
    pub async fn deltas(&self, family: FamilyId) -> Result<Vec<Delta>, StoreError> {
        let rows = sqlx::query("SELECT data FROM deltas WHERE family_id = ? ORDER BY seq")
            .bind(id_bytes(family.0))
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|row| Ok(postcard::from_bytes(row.get::<&[u8], _>("data"))?))
            .collect()
    }

    pub async fn add_checkout(
        &self,
        thread: ThreadId,
        path: &Path,
        actor: ActorId,
        managed: bool,
        now_ms: i64,
    ) -> Result<CheckoutRecord, StoreError> {
        let id = sqlx::query(
            "INSERT INTO checkouts (thread_id, path, actor, managed, created_ms) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id_bytes(thread.0))
        .bind(path_text(path))
        .bind(id_bytes(actor.0))
        .bind(managed)
        .bind(now_ms)
        .execute(&self.pool)
        .await?
        .last_insert_rowid();
        Ok(CheckoutRecord {
            id,
            thread,
            path: path.to_owned(),
            actor,
            managed,
        })
    }

    pub async fn checkouts(&self) -> Result<Vec<CheckoutRecord>, StoreError> {
        let rows = sqlx::query("SELECT id, thread_id, path, actor, managed FROM checkouts ORDER BY id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|row| {
                Ok(CheckoutRecord {
                    id: row.get("id"),
                    thread: ThreadId(id_from(row.get("thread_id"))?),
                    path: PathBuf::from(row.get::<String, _>("path")),
                    actor: ActorId(id_from(row.get("actor"))?),
                    managed: row.get("managed"),
                })
            })
            .collect()
    }

    pub async fn disk_files(&self, checkout: i64) -> Result<HashMap<String, DiskFile>, StoreError> {
        let rows =
            sqlx::query("SELECT path, inode, hash, size, mtime_ns, version FROM disk_files WHERE checkout_id = ?")
            .bind(checkout)
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|row| {
                let hash: &[u8] = row.get("hash");
                Ok((
                    row.get("path"),
                    DiskFile {
                        inode: postcard::from_bytes(row.get::<&[u8], _>("inode"))?,
                        hash: hash.try_into().map_err(|_| StoreError::Corrupt { what: "hash" })?,
                        size: u64::try_from(row.get::<i64, _>("size")).unwrap_or_default(),
                        mtime_ns: row.get("mtime_ns"),
                        version: postcard::from_bytes(row.get::<&[u8], _>("version"))?,
                    },
                ))
            })
            .collect()
    }

    pub async fn put_disk_file(&self, checkout: i64, path: &str, file: &DiskFile) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO disk_files (checkout_id, path, inode, hash, size, mtime_ns, version) VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (checkout_id, path) DO UPDATE SET inode = excluded.inode,
                 hash = excluded.hash, size = excluded.size, mtime_ns = excluded.mtime_ns, version = excluded.version",
        )
        .bind(checkout)
        .bind(path)
        .bind(postcard::to_allocvec(&file.inode)?)
        .bind(file.hash.as_slice())
        .bind(i64::try_from(file.size).unwrap_or(i64::MAX))
        .bind(file.mtime_ns)
        .bind(postcard::to_allocvec(&file.version)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn remove_disk_file(&self, checkout: i64, path: &str) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM disk_files WHERE checkout_id = ? AND path = ?")
            .bind(checkout)
            .bind(path)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn add_session(&self, session: &SessionRecord, now_ms: i64) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO sessions (agent, session_id, thread_id, checkout_id, created_ms) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT (agent, session_id) DO NOTHING",
        )
        .bind(&session.agent)
        .bind(&session.session_id)
        .bind(id_bytes(session.thread.0))
        .bind(session.checkout_id)
        .bind(now_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn sessions(&self) -> Result<Vec<SessionRecord>, StoreError> {
        let rows = sqlx::query("SELECT agent, session_id, thread_id, checkout_id FROM sessions ORDER BY created_ms")
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|row| {
                Ok(SessionRecord {
                    agent: row.get("agent"),
                    session_id: row.get("session_id"),
                    thread: ThreadId(id_from(row.get("thread_id"))?),
                    checkout_id: row.get("checkout_id"),
                })
            })
            .collect()
    }

    pub async fn session_offsets(&self, agent: &str, session_id: &str) -> Result<Vec<SessionFileOffset>, StoreError> {
        let rows = sqlx::query("SELECT file, path, offset FROM session_files WHERE agent = ? AND session_id = ?")
            .bind(agent)
            .bind(session_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .iter()
            .map(|row| SessionFileOffset {
                file: row.get("file"),
                path: PathBuf::from(row.get::<String, _>("path")),
                offset: u64::try_from(row.get::<i64, _>("offset")).unwrap_or_default(),
            })
            .collect())
    }

    pub async fn set_session_offset(
        &self,
        agent: &str,
        session_id: &str,
        file: &SessionFileOffset,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO session_files (agent, session_id, file, path, offset) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT (agent, session_id, file) DO UPDATE SET path = excluded.path, offset = excluded.offset",
        )
        .bind(agent)
        .bind(session_id)
        .bind(&file.file)
        .bind(path_text(&file.path))
        .bind(i64::try_from(file.offset).unwrap_or(i64::MAX))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn index(&self, thread: ThreadId, delta: DeltaId, kind: &str, body: &str) -> Result<(), StoreError> {
        sqlx::query("INSERT INTO search (body, thread_id, actor, lamport, kind) VALUES (?, ?, ?, ?, ?)")
            .bind(body)
            .bind(id_bytes(thread.0))
            .bind(id_bytes(delta.actor.0))
            .bind(lamport_i64(delta.lamport))
            .bind(kind)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Full-text search; `query` uses SQLite FTS5 syntax and words are
    /// matched as prefixes.
    pub async fn search(&self, query: &str, limit: u32) -> Result<Vec<SearchHit>, StoreError> {
        let query = fts_query(query);
        if query.is_empty() {
            return Ok(Vec::new());
        }
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

/// Turns free text into an FTS5 query of quoted prefix terms, so user input
/// can't be misread as query syntax.
fn fts_query(text: &str) -> String {
    text.split_whitespace()
        .map(|word| format!("\"{}\"*", word.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
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
