//! Notifications (ADR 0013): written when a push stores a merge request,
//! a decision on one, a comment or an `@mention`, and read with
//! `GET /v1/inbox`.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use axum::Json;
use axum::extract::{Extension, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use silt_log::{CommentOp, Delta, MrOp, Op};
use silt_proto::{InboxRead, InboxReply, Notification};
use silt_types::ThreadId;
use sqlx::Row;
use sqlx::sqlite::SqlitePool;

use crate::visibility::Viewer;
use crate::{ApiError, Caller, Server, UserId, id_bytes, id_from, now_ms};

const INBOX_LIMIT: i64 = 200;
/// Mentions past this many in one message notify nobody, so one message
/// can't make the server look up every account.
const MENTIONS_LIMIT: usize = 20;

/// One notification to write.
struct Note {
    user: UserId,
    kind: &'static str,
    thread: ThreadId,
    text: String,
}

/// The user names mentioned as `@name` in `text`.
pub(crate) fn mentions(text: &str) -> Vec<String> {
    let mut names: Vec<String> = text
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ';' | ':' | '(' | ')' | '"' | '\'' | '!' | '?'))
        .filter_map(|word| word.strip_prefix('@'))
        .map(|name| name.trim_end_matches('.').to_lowercase())
        .filter(|name| crate::accounts::valid_name(name))
        .collect();
    names.sort();
    names.dedup();
    names
}

async fn owner(db: &SqlitePool, thread: ThreadId) -> Result<Option<UserId>, sqlx::Error> {
    Ok(sqlx::query("SELECT owner_id FROM threads WHERE id = ?")
        .bind(id_bytes(thread.0))
        .fetch_optional(db)
        .await?
        .map(|row| row.get("owner_id")))
}

fn clip(text: &str) -> String {
    let text = text.trim().replace('\n', " ");
    if text.chars().count() > 140 { text.chars().take(140).collect::<String>() + "…" } else { text }
}

/// Viewers loaded once per notified user within one push.
#[derive(Default)]
struct Viewers(HashMap<UserId, std::sync::Arc<Viewer>>);

impl Viewers {
    async fn sees(&mut self, server: &Server, user: UserId, thread: ThreadId) -> Result<bool, sqlx::Error> {
        let viewer = match self.0.entry(user) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(server.viewer(user).await?),
        };
        Ok(viewer.sees_thread(thread))
    }
}

impl Server {
    /// Writes the notifications `stored` deltas, pushed by `actor`, cause.
    pub(crate) async fn notify(&self, stored: &[Delta], actor: Caller) -> Result<(), sqlx::Error> {
        let db = self.db();
        let actor_name: String =
            sqlx::query("SELECT name FROM users WHERE id = ?").bind(actor.user).fetch_one(db).await?.get("name");
        let mut viewers = Viewers::default();
        let mut notes = Vec::new();
        for delta in stored {
            let mut mentioned = String::new();
            match &delta.op {
                Op::Mr(MrOp::Opened { target, .. }) if !viewers.sees(self, actor.user, *target).await? => {}
                Op::Mr(MrOp::Opened { target, title, summary }) => {
                    if let Some(owner) = owner(db, *target).await? {
                        notes.push(Note {
                            user: owner,
                            kind: "mr_opened",
                            thread: *target,
                            text: format!(
                                "{actor_name} asks to merge \"{}\" from fork {} into your thread {}",
                                clip(title),
                                delta.thread.short(),
                                target.short()
                            ),
                        });
                    }
                    mentioned = format!("{title}\n{summary}");
                }
                Op::Mr(decision @ (MrOp::Approved { mr } | MrOp::Declined { mr, .. })) => {
                    let fork = self.shared.index.lock().await.request_fork(*mr);
                    if let Some(fork) = fork
                        && let Some(owner) = owner(db, fork).await?
                    {
                        let (kind, verb) = match decision {
                            MrOp::Declined { reason, .. } if !reason.is_empty() => {
                                ("mr_declined", format!("declined ({})", clip(reason)))
                            }
                            MrOp::Declined { .. } => ("mr_declined", "declined".to_owned()),
                            _ => ("mr_approved", "approved and merged".to_owned()),
                        };
                        notes.push(Note {
                            user: owner,
                            kind,
                            thread: fork,
                            text: format!("{actor_name} {verb} your merge request from fork {}", fork.short()),
                        });
                    }
                }
                Op::Comment(CommentOp::Added { text, .. }) => {
                    if let Some(owner) = owner(db, delta.thread).await? {
                        notes.push(Note {
                            user: owner,
                            kind: "comment",
                            thread: delta.thread,
                            text: format!("{actor_name} commented on thread {}: {}", delta.thread.short(), clip(text)),
                        });
                    }
                    mentioned.clone_from(text);
                }
                _ => {}
            }
            for name in mentions(&mentioned).into_iter().take(MENTIONS_LIMIT) {
                let Some(user) = crate::accounts::user_by_name(db, &name).await? else {
                    continue;
                };
                let thread = match &delta.op {
                    Op::Mr(MrOp::Opened { target, .. }) => *target,
                    _ => delta.thread,
                };
                if viewers.sees(self, user.id, thread).await? {
                    notes.push(Note {
                        user: user.id,
                        kind: "mention",
                        thread,
                        text: format!("{actor_name} mentioned you in thread {}: {}", thread.short(), clip(&mentioned)),
                    });
                }
            }
            for note in notes.drain(..).filter(|note| note.user != actor.user) {
                sqlx::query(
                    "INSERT OR IGNORE INTO notifications (user_id, kind, thread_id, delta, actor, text, created_ms)
                     VALUES (?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(note.user)
                .bind(note.kind)
                .bind(id_bytes(note.thread.0))
                .bind(delta.span.first().to_string())
                .bind(&actor_name)
                .bind(&note.text)
                .bind(now_ms())
                .execute(db)
                .await?;
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
pub(crate) struct InboxQuery {
    #[serde(default)]
    all: bool,
}

pub(crate) async fn list(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Query(query): Query<InboxQuery>,
) -> Result<Json<InboxReply>, ApiError> {
    let rows = sqlx::query(
        "SELECT id, kind, thread_id, actor, text, created_ms, read_ms FROM notifications
         WHERE user_id = ? AND (? OR read_ms IS NULL) ORDER BY id DESC LIMIT ?",
    )
    .bind(caller.user)
    .bind(query.all)
    .bind(INBOX_LIMIT)
    .fetch_all(server.db())
    .await?;
    let viewer = server.viewer(caller.user).await?;
    let notifications = rows
        .iter()
        .map(|row| Notification {
            id: row.get("id"),
            kind: row.get("kind"),
            thread: ThreadId(id_from(row.get("thread_id"))),
            actor: row.get("actor"),
            text: row.get("text"),
            created_ms: row.get("created_ms"),
            read: row.get::<Option<i64>, _>("read_ms").is_some(),
        })
        .filter(|notification| viewer.sees_thread(notification.thread))
        .collect();
    Ok(Json(InboxReply { notifications }))
}

pub(crate) async fn read(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Json(body): Json<InboxRead>,
) -> Result<StatusCode, ApiError> {
    let db = server.db();
    if body.all {
        sqlx::query("UPDATE notifications SET read_ms = ? WHERE user_id = ? AND read_ms IS NULL")
            .bind(now_ms())
            .bind(caller.user)
            .execute(db)
            .await?;
    }
    for id in body.ids {
        sqlx::query("UPDATE notifications SET read_ms = ? WHERE id = ? AND user_id = ? AND read_ms IS NULL")
            .bind(now_ms())
            .bind(id)
            .bind(caller.user)
            .execute(db)
            .await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mentions_are_names_after_an_at() {
        assert_eq!(mentions("Thanks @Bob, and @carol. Mail a@b.c (@dave)"), vec!["bob", "carol", "dave"]);
        assert!(mentions("no one here @").is_empty());
    }
}
