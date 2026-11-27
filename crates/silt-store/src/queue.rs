//! Prompts waiting for a thread's agent (ADR 0022).

use silt_types::ThreadId;

use crate::{Store, StoreError, id_bytes};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedPrompt {
    pub id: i64,
    pub author: String,
    pub text: String,
    pub created_ms: i64,
    /// The files it goes with, in order; the daemon keeps them.
    pub files: Vec<QueuedFile>,
}

/// A file a queued prompt goes with, as it was attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedFile {
    pub name: String,
    pub mime: String,
}

fn files_json(files: &[QueuedFile]) -> String {
    serde_json::Value::Array(
        files.iter().map(|file| serde_json::json!({"name": file.name, "mime": file.mime})).collect(),
    )
    .to_string()
}

/// The files a row names; a row this build can't read names none.
fn files_of(json: &str) -> Vec<QueuedFile> {
    let parsed: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    parsed
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|file| {
            Some(QueuedFile { name: file["name"].as_str()?.to_owned(), mime: file["mime"].as_str()?.to_owned() })
        })
        .collect()
}

impl Store {
    /// The thread's queue, next first.
    pub async fn queue(&self, thread: ThreadId) -> Result<Vec<QueuedPrompt>, StoreError> {
        let thread = id_bytes(thread.0);
        let rows = sqlx::query!(
            r#"SELECT id AS "id!", author, text, created_ms, files FROM prompt_queue WHERE thread_id = ? ORDER BY position, id"#,
            thread
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| QueuedPrompt {
                id: row.id,
                author: row.author,
                text: row.text,
                created_ms: row.created_ms,
                files: files_of(&row.files),
            })
            .collect())
    }

    /// Adds a prompt, with the files it goes with, at the end of the
    /// thread's queue, or at its front.
    pub async fn enqueue(
        &self,
        thread: ThreadId,
        author: &str,
        text: &str,
        files: &[QueuedFile],
        created_ms: i64,
        first: bool,
    ) -> Result<i64, StoreError> {
        let files = files_json(files);
        let thread = id_bytes(thread.0);
        let mut tx = self.write_transaction().await?;
        let ends = sqlx::query!(
            r#"SELECT COALESCE(MIN(position), 0) AS "first!: i64", COALESCE(MAX(position), 0) AS "last!: i64"
               FROM prompt_queue WHERE thread_id = ?"#,
            thread
        )
        .fetch_one(&mut *tx)
        .await?;
        let position = if first { ends.first - 1 } else { ends.last + 1 };
        let id = sqlx::query_scalar!(
            r#"INSERT INTO prompt_queue (thread_id, author, text, files, position, created_ms) VALUES (?, ?, ?, ?, ?, ?)
               RETURNING id AS "id!""#,
            thread,
            author,
            text,
            files,
            position,
            created_ms
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(id)
    }

    /// Changes a queued prompt's text; false when it was sent or removed.
    pub async fn edit_queued(&self, thread: ThreadId, id: i64, text: &str) -> Result<bool, StoreError> {
        let thread = id_bytes(thread.0);
        let done = sqlx::query!("UPDATE prompt_queue SET text = ? WHERE id = ? AND thread_id = ?", text, id, thread)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() > 0)
    }

    /// Takes a prompt out of the queue, to send or drop; `None` when it was
    /// sent or removed already.
    pub async fn take_queued(&self, thread: ThreadId, id: i64) -> Result<Option<QueuedPrompt>, StoreError> {
        let thread = id_bytes(thread.0);
        let row = sqlx::query!(
            r#"DELETE FROM prompt_queue WHERE id = ? AND thread_id = ? RETURNING id AS "id!", author, text, created_ms, files"#,
            id,
            thread
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| QueuedPrompt {
            id: row.id,
            author: row.author,
            text: row.text,
            created_ms: row.created_ms,
            files: files_of(&row.files),
        }))
    }

    /// Every queued prompt's id, in any thread: the daemon drops the files
    /// of any other.
    pub async fn queued_ids(&self) -> Result<Vec<i64>, StoreError> {
        Ok(sqlx::query_scalar!(r#"SELECT id AS "id!" FROM prompt_queue"#).fetch_all(&self.pool).await?)
    }

    /// Takes the next prompt out of the thread's queue.
    pub async fn take_next(&self, thread: ThreadId) -> Result<Option<QueuedPrompt>, StoreError> {
        match self.queue(thread).await?.first() {
            Some(next) => self.take_queued(thread, next.id).await,
            None => Ok(None),
        }
    }

    /// Moves a queued prompt to `to`, counted from the front.
    pub async fn move_queued(&self, thread: ThreadId, id: i64, to: usize) -> Result<bool, StoreError> {
        let mut order: Vec<i64> = self.queue(thread).await?.into_iter().map(|queued| queued.id).collect();
        let Some(from) = order.iter().position(|queued| *queued == id) else {
            return Ok(false);
        };
        let moved = order.remove(from);
        order.insert(to.min(order.len()), moved);
        let thread = id_bytes(thread.0);
        let mut tx = self.write_transaction().await?;
        for (position, id) in (0_i64..).zip(&order) {
            sqlx::query!("UPDATE prompt_queue SET position = ? WHERE id = ? AND thread_id = ?", position, id, thread)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(true)
    }
}
