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
use silt_proto::{InboxAction, InboxRead, InboxReply, KindSetting, NOTIFICATION_KINDS, Notification};
use silt_types::ThreadId;
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
    let thread = id_bytes(thread.0);
    sqlx::query_scalar!(r#"SELECT owner_id AS "owner_id: UserId" FROM threads WHERE id = ?"#, thread).fetch_optional(db).await
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
        let actor_name = sqlx::query_scalar!("SELECT name FROM users WHERE id = ?", actor.user).fetch_one(db).await?;
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
                Op::Mr(MrOp::ChangesRequested { mr, reason }) => {
                    let fork = self.shared.index.lock().await.request_fork(*mr);
                    if let Some(fork) = fork
                        && let Some(owner) = owner(db, fork).await?
                    {
                        notes.push(Note {
                            user: owner,
                            kind: "mr_changes_requested",
                            thread: fork,
                            text: format!(
                                "{actor_name} asks for changes to your merge request from fork {}: {}",
                                fork.short(),
                                clip(reason)
                            ),
                        });
                    }
                    mentioned.clone_from(reason);
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
                Op::Comment(CommentOp::Replied { parent, text, .. }) => {
                    // The comment's author and those who replied before.
                    let people = self.shared.index.lock().await.comment_people(*parent);
                    for name in people.iter().filter(|name| **name != actor_name) {
                        let Some(user) = crate::accounts::user_by_name(db, name).await? else {
                            continue;
                        };
                        if viewers.sees(self, user.id, delta.thread).await? {
                            notes.push(Note {
                                user: user.id,
                                kind: "reply",
                                thread: delta.thread,
                                text: format!("{actor_name} replied on thread {}: {}", delta.thread.short(), clip(text)),
                            });
                        }
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
                let (thread, delta_id, now) = (id_bytes(note.thread.0), delta.span.first().to_string(), now_ms());
                sqlx::query!(
                    "INSERT OR IGNORE INTO notifications (user_id, kind, thread_id, delta, actor, text, created_ms)
                     VALUES (?, ?, ?, ?, ?, ?, ?)",
                    note.user,
                    note.kind,
                    thread,
                    delta_id,
                    actor_name,
                    note.text,
                    now,
                )
                .execute(db)
                .await?;
            }
        }
        Ok(())
    }
}

/// What a device did or reports, for its person (ADR 0028).
pub(crate) struct PersonNote<'a> {
    pub(crate) user: UserId,
    pub(crate) kind: &'static str,
    pub(crate) thread: ThreadId,
    /// The device it's about.
    pub(crate) actor: &'a str,
    pub(crate) text: String,
    pub(crate) action: Option<InboxAction>,
}

/// Writes `note`, bringing an unread one of the same kind and thread up to
/// date rather than repeating it.
pub(crate) async fn notify_person(db: &SqlitePool, note: PersonNote<'_>) -> Result<(), sqlx::Error> {
    let (thread, now) = (id_bytes(note.thread.0), now_ms());
    let action = note.action.as_ref().and_then(|action| serde_json::to_string(action).ok());
    let updated = sqlx::query!(
        "UPDATE notifications SET text = ?, actor = ?, action = ?, created_ms = ?
         WHERE user_id = ? AND kind = ? AND thread_id = ? AND read_ms IS NULL",
        note.text,
        note.actor,
        action,
        now,
        note.user,
        note.kind,
        thread,
    )
    .execute(db)
    .await?;
    if updated.rows_affected() == 0 {
        // Not from a delta: a key of its own keeps it apart from the rest.
        let key = format!("{}:{}", note.kind, uuid::Uuid::new_v4().simple());
        sqlx::query!(
            "INSERT INTO notifications (user_id, kind, thread_id, delta, actor, text, created_ms, action)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            note.user,
            note.kind,
            thread,
            key,
            note.actor,
            note.text,
            now,
            action,
        )
        .execute(db)
        .await?;
    }
    Ok(())
}

/// Each kind of notification and whether it notifies `user`.
async fn settings_of(db: &SqlitePool, user: UserId) -> Result<Vec<KindSetting>, sqlx::Error> {
    let chosen: HashMap<String, bool> =
        sqlx::query!(r#"SELECT kind, notify AS "notify: bool" FROM notification_settings WHERE user_id = ?"#, user)
            .fetch_all(db)
            .await?
            .into_iter()
            .map(|row| (row.kind, row.notify))
            .collect();
    Ok(NOTIFICATION_KINDS
        .iter()
        .map(|(kind, default)| KindSetting { kind: (*kind).to_owned(), notify: chosen.get(*kind).copied().unwrap_or(*default) })
        .collect())
}

pub(crate) async fn settings(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<Vec<KindSetting>>, ApiError> {
    Ok(Json(settings_of(server.db(), caller.user).await?))
}

pub(crate) async fn change_setting(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Json(setting): Json<KindSetting>,
) -> Result<Json<Vec<KindSetting>>, ApiError> {
    if !NOTIFICATION_KINDS.iter().any(|(kind, _)| *kind == setting.kind) {
        return Err(crate::bad_request(format!("there are no {} notifications", setting.kind)));
    }
    sqlx::query!(
        "INSERT INTO notification_settings (user_id, kind, notify) VALUES (?, ?, ?)
         ON CONFLICT (user_id, kind) DO UPDATE SET notify = excluded.notify",
        caller.user,
        setting.kind,
        setting.notify,
    )
    .execute(server.db())
    .await?;
    Ok(Json(settings_of(server.db(), caller.user).await?))
}

#[derive(Deserialize)]
pub(crate) struct InboxQuery {
    #[serde(default)]
    pub(crate) all: bool,
}

pub(crate) async fn list(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Query(query): Query<InboxQuery>,
) -> Result<Json<InboxReply>, ApiError> {
    let rows = sqlx::query!(
        r#"SELECT id AS "id!", kind, thread_id, actor, text, created_ms, read_ms, action FROM notifications
         WHERE user_id = ? AND (? OR read_ms IS NULL) ORDER BY id DESC LIMIT ?"#,
        caller.user,
        query.all,
        INBOX_LIMIT,
    )
    .fetch_all(server.db())
    .await?;
    let viewer = server.viewer(caller.user).await?;
    let notifies: HashMap<String, bool> =
        settings_of(server.db(), caller.user).await?.into_iter().map(|setting| (setting.kind, setting.notify)).collect();
    let notifications = rows
        .into_iter()
        .map(|row| Notification {
            id: row.id,
            notify: notifies.get(&row.kind).copied().unwrap_or_default(),
            kind: row.kind,
            thread: ThreadId(id_from(&row.thread_id)),
            actor: row.actor,
            text: row.text,
            created_ms: row.created_ms,
            read: row.read_ms.is_some(),
            // Only while it's unread: what it answers has moved on since.
            action: row.action.filter(|_| row.read_ms.is_none()).and_then(|action| serde_json::from_str(&action).ok()),
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
    let now = now_ms();
    if body.all {
        sqlx::query!("UPDATE notifications SET read_ms = ? WHERE user_id = ? AND read_ms IS NULL", now, caller.user)
            .execute(db)
            .await?;
    }
    for id in body.ids {
        sqlx::query!(
            "UPDATE notifications SET read_ms = ? WHERE id = ? AND user_id = ? AND read_ms IS NULL",
            now,
            id,
            caller.user
        )
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
