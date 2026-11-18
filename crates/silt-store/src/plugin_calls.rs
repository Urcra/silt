//! Plugin calls and the grants that let some run without asking (ADR 0093).

use silt_types::ThreadId;

use crate::{Store, StoreError, id_bytes, id_from};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginCallRecord {
    pub id: i64,
    pub thread: ThreadId,
    /// The hash of the token of the run that made it.
    pub run_hash: Vec<u8>,
    pub workspace: String,
    /// The canonical request, as JSON.
    pub request: String,
    pub hash: String,
    pub change: bool,
    pub asked: bool,
    /// `pending`, `running`, `rejected`, `expired` or `ended`.
    pub state: String,
    /// How it ended, as JSON.
    pub outcome: Option<String>,
    pub grant: Option<i64>,
    pub created_ms: i64,
    pub answered_ms: Option<i64>,
    /// `legacy` for `plugins.json`'s kinds, whose request is a canonical
    /// request; `wasm` for a WebAssembly plugin's, whose request is the call
    /// as it prepared it (ADR 0103).
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantRecord {
    pub id: i64,
    pub thread: ThreadId,
    /// For one run only, by its token's hash.
    pub run_hash: Option<Vec<u8>>,
    pub plugin: String,
    pub environment: String,
    pub change: bool,
    pub created_ms: i64,
    pub until_ms: Option<i64>,
    pub revoked_ms: Option<i64>,
    pub from_call: i64,
    /// The one action of a WebAssembly plugin it covers.
    pub action: Option<String>,
}

macro_rules! call_from_row {
    ($row:expr) => {{
        let row = $row;
        Ok::<_, StoreError>(PluginCallRecord {
            id: row.id,
            thread: ThreadId(id_from(&row.thread_id)?),
            run_hash: row.run_hash,
            workspace: row.workspace,
            request: row.request,
            hash: row.hash,
            change: row.change != 0,
            asked: row.asked != 0,
            state: row.state,
            outcome: row.outcome,
            grant: row.grant_id,
            created_ms: row.created_ms,
            answered_ms: row.answered_ms,
            kind: row.kind,
        })
    }};
}

macro_rules! grant_from_row {
    ($row:expr) => {{
        let row = $row;
        Ok::<_, StoreError>(GrantRecord {
            id: row.id,
            thread: ThreadId(id_from(&row.thread_id)?),
            run_hash: row.run_hash,
            plugin: row.plugin,
            environment: row.environment,
            change: row.change != 0,
            created_ms: row.created_ms,
            until_ms: row.until_ms,
            revoked_ms: row.revoked_ms,
            from_call: row.from_call,
            action: row.action,
        })
    }};
}

impl Store {
    /// Keeps a call, answering its id.
    pub async fn add_plugin_call(&self, call: &PluginCallRecord) -> Result<i64, StoreError> {
        let thread = id_bytes(call.thread.0);
        let (change, asked) = (i64::from(call.change), i64::from(call.asked));
        let id = sqlx::query_scalar!(
            r#"INSERT INTO plugin_calls
               (thread_id, run_hash, workspace, request, hash, change, asked, state, outcome, grant_id, created_ms, answered_ms, kind)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id AS "id!""#,
            thread,
            call.run_hash,
            call.workspace,
            call.request,
            call.hash,
            change,
            asked,
            call.state,
            call.outcome,
            call.grant,
            call.created_ms,
            call.answered_ms,
            call.kind
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(id)
    }

    /// Moves call `id` on to `state`, and its outcome once it has one.
    pub async fn set_plugin_call(
        &self,
        id: i64,
        state: &str,
        outcome: Option<&str>,
        answered_ms: Option<i64>,
    ) -> Result<(), StoreError> {
        sqlx::query!(
            "UPDATE plugin_calls SET state = ?, outcome = COALESCE(?, outcome), answered_ms = COALESCE(?, answered_ms) WHERE id = ?",
            state,
            outcome,
            answered_ms,
            id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn plugin_call(&self, id: i64) -> Result<Option<PluginCallRecord>, StoreError> {
        let row = sqlx::query!(
            r#"SELECT id AS "id!", thread_id, run_hash, workspace, request, hash, change, asked, state, outcome, grant_id, created_ms, answered_ms, kind
               FROM plugin_calls WHERE id = ?"#,
            id
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| call_from_row!(row)).transpose()
    }

    /// Calls, newest first: a thread's when one is named, else every
    /// thread's; only those in `state` when it's given.
    pub async fn plugin_calls(
        &self,
        thread: Option<ThreadId>,
        state: Option<&str>,
        limit: i64,
    ) -> Result<Vec<PluginCallRecord>, StoreError> {
        let thread = thread.map(|thread| id_bytes(thread.0));
        let rows = sqlx::query!(
            r#"SELECT id AS "id!", thread_id, run_hash, workspace, request, hash, change, asked, state, outcome, grant_id, created_ms, answered_ms, kind
               FROM plugin_calls
               WHERE (?1 IS NULL OR thread_id = ?1) AND (?2 IS NULL OR state = ?2)
               ORDER BY id DESC LIMIT ?3"#,
            thread,
            state,
            limit
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(|row| call_from_row!(row)).collect()
    }

    pub async fn add_grant(&self, grant: &GrantRecord) -> Result<i64, StoreError> {
        let thread = id_bytes(grant.thread.0);
        let change = i64::from(grant.change);
        let id = sqlx::query_scalar!(
            r#"INSERT INTO plugin_grants
               (thread_id, run_hash, plugin, environment, change, created_ms, until_ms, revoked_ms, from_call, action)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id AS "id!""#,
            thread,
            grant.run_hash,
            grant.plugin,
            grant.environment,
            change,
            grant.created_ms,
            grant.until_ms,
            grant.revoked_ms,
            grant.from_call,
            grant.action
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(id)
    }

    /// The thread's grants, newest first, revoked and lapsed ones too.
    pub async fn grants(&self, thread: ThreadId) -> Result<Vec<GrantRecord>, StoreError> {
        let thread = id_bytes(thread.0);
        let rows = sqlx::query!(
            r#"SELECT id AS "id!", thread_id, run_hash, plugin, environment, change, created_ms, until_ms, revoked_ms, from_call, action
               FROM plugin_grants WHERE thread_id = ? ORDER BY id DESC"#,
            thread
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(|row| grant_from_row!(row)).collect()
    }

    /// A grant in force at `now` that covers a call to `plugin`'s
    /// `environment` from the run whose token hashes to `run_hash`: for a
    /// WebAssembly plugin, to its `action`.
    #[expect(clippy::too_many_arguments, reason = "each is part of what a grant covers")]
    pub async fn grant_for(
        &self,
        thread: ThreadId,
        run_hash: &[u8],
        plugin: &str,
        environment: &str,
        change: bool,
        action: Option<&str>,
        now_ms: i64,
    ) -> Result<Option<GrantRecord>, StoreError> {
        let thread = id_bytes(thread.0);
        let change = i64::from(change);
        let row = sqlx::query!(
            r#"SELECT id AS "id!", thread_id, run_hash, plugin, environment, change, created_ms, until_ms, revoked_ms, from_call, action
               FROM plugin_grants
               WHERE thread_id = ? AND plugin = ? AND environment = ? AND change = ? AND action IS ? AND revoked_ms IS NULL
                 AND (until_ms IS NULL OR until_ms > ?) AND (run_hash IS NULL OR run_hash = ?)
               ORDER BY id DESC LIMIT 1"#,
            thread,
            plugin,
            environment,
            change,
            action,
            now_ms,
            run_hash
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| grant_from_row!(row)).transpose()
    }

    /// Forgets a thread's plugin calls and grants, as it's erased.
    pub async fn remove_plugin_calls(&self, thread: ThreadId) -> Result<(), StoreError> {
        let thread = id_bytes(thread.0);
        let mut tx = self.write_transaction().await?;
        sqlx::query!("DELETE FROM plugin_calls WHERE thread_id = ?", thread).execute(&mut *tx).await?;
        sqlx::query!("DELETE FROM plugin_grants WHERE thread_id = ?", thread).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Ends grant `id`, answering the thread it was for, or none when it
    /// was never there or already ended.
    pub async fn revoke_grant(&self, id: i64, now_ms: i64) -> Result<Option<ThreadId>, StoreError> {
        let row = sqlx::query!(
            "UPDATE plugin_grants SET revoked_ms = ? WHERE id = ? AND revoked_ms IS NULL RETURNING thread_id",
            now_ms,
            id
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| Ok(ThreadId(id_from(&row.thread_id)?))).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(thread: ThreadId) -> PluginCallRecord {
        PluginCallRecord {
            id: 0,
            thread,
            run_hash: b"run".to_vec(),
            workspace: "work".into(),
            request: "{}".into(),
            hash: "abc".into(),
            change: true,
            asked: true,
            state: "pending".into(),
            outcome: None,
            grant: None,
            created_ms: 1,
            answered_ms: None,
            kind: "legacy".into(),
        }
    }

    #[tokio::test]
    async fn a_call_moves_on_and_a_grant_covers_what_it_names_until_revoked() {
        let data = tempfile::tempdir().unwrap();
        let store = Store::open(data.path()).await.unwrap();
        let (thread, fork) = (ThreadId(7), ThreadId(8));
        let id = store.add_plugin_call(&call(thread)).await.unwrap();
        assert_eq!(store.plugin_calls(None, Some("pending"), 10).await.unwrap().len(), 1);
        store.set_plugin_call(id, "ended", Some(r#"{"status":"done","output":"ok"}"#), Some(5)).await.unwrap();
        let ended = store.plugin_call(id).await.unwrap().unwrap();
        assert_eq!((ended.state.as_str(), ended.answered_ms), ("ended", Some(5)));
        assert!(store.plugin_calls(Some(fork), None, 10).await.unwrap().is_empty());

        let grant = GrantRecord {
            id: 0,
            thread,
            run_hash: None,
            plugin: "fleet".into(),
            environment: "staging".into(),
            change: true,
            created_ms: 1,
            until_ms: Some(100),
            revoked_ms: None,
            from_call: id,
            action: None,
        };
        let granted = store.add_grant(&grant).await.unwrap();
        let covers = |thread, change, now| {
            let store = &store;
            async move { store.grant_for(thread, b"run", "fleet", "staging", change, None, now).await.unwrap().is_some() }
        };
        assert!(covers(thread, true, 50).await);
        assert!(!covers(thread, false, 50).await, "a grant of changes isn't one of reads");
        assert!(!covers(thread, true, 100).await, "lapsed");
        assert!(!covers(fork, true, 50).await, "a fork starts with none");
        let run_only =
            store.add_grant(&GrantRecord { run_hash: Some(b"other".to_vec()), until_ms: None, ..grant }).await.unwrap();
        assert!(
            !store
                .grant_for(thread, b"run", "fleet", "staging", true, None, 200)
                .await
                .unwrap()
                .is_some_and(|found| found.id == run_only)
        );
        assert_eq!(store.revoke_grant(granted, 60).await.unwrap(), Some(thread));
        assert!(!covers(thread, true, 50).await);
        assert_eq!(store.revoke_grant(granted, 61).await.unwrap(), None);
        assert_eq!(store.grants(thread).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_plugins_grant_covers_its_one_action() {
        let data = tempfile::tempdir().unwrap();
        let store = Store::open(data.path()).await.unwrap();
        let thread = ThreadId(7);
        let id = store.add_plugin_call(&PluginCallRecord { kind: "wasm".into(), ..call(thread) }).await.unwrap();
        assert_eq!(store.plugin_call(id).await.unwrap().unwrap().kind, "wasm");
        let grant = GrantRecord {
            id: 0,
            thread,
            run_hash: None,
            plugin: "silt.github".into(),
            environment: "acme".into(),
            change: true,
            created_ms: 1,
            until_ms: None,
            revoked_ms: None,
            from_call: id,
            action: Some("push".into()),
        };
        store.add_grant(&grant).await.unwrap();
        let covers = |action: Option<&'static str>| {
            let store = &store;
            async move { store.grant_for(thread, b"run", "silt.github", "acme", true, action, 50).await.unwrap().is_some() }
        };
        assert!(covers(Some("push")).await);
        assert!(!covers(Some("merge")).await, "another action");
        assert!(!covers(None).await, "nor every change");
    }
}
