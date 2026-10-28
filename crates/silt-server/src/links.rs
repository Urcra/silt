//! Read-only links (ADR 0027): a token for one thread that anyone holding
//! it reads, until it expires or its owner revokes it, on a page the
//! server renders with the thread's own conversation and nothing else.

use std::time::{Duration, Instant};

use axum::extract::{Extension, Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use silt_log::Delta;
use silt_proto::view::{NewLink, ThreadLink};
use silt_types::{FamilyId, ThreadId};

use crate::app::Browser;
use crate::pages::escape;
use crate::{ApiError, Caller, Server, UserId, bad_request, forbidden, hash, id_bytes, id_from, not_found, now_ms};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Tokens that open nothing, server-wide, before the page pauses.
const MISSES_ALLOWED: usize = 30;
const MISS_WINDOW: Duration = Duration::from_secs(60);

pub(crate) fn browser_routes() -> Router<Server> {
    Router::new()
        .route("/app/api/threads/{id}/links", get(browser_list).post(browser_make))
        .route("/app/api/threads/{id}/links/{link}/revoke", post(browser_revoke))
}

pub(crate) fn device_routes() -> Router<Server> {
    Router::new()
        .route("/v1/threads/{id}/links", get(device_list).post(device_make))
        .route("/v1/threads/{id}/links/{link}/revoke", post(device_revoke))
}

pub(crate) fn page_routes() -> Router<Server> {
    Router::new().route("/r/{token}", get(read))
}

/// The thread `selector` names among those `user` owns: only an owner
/// shares one this way.
async fn owned_thread(server: &Server, user: UserId, selector: &str) -> Result<(ThreadId, FamilyId), ApiError> {
    let rows = sqlx::query!("SELECT id, family_id FROM threads WHERE owner_id = ? AND deleted_ms IS NULL", user)
        .fetch_all(server.db())
        .await?;
    let mut found: Vec<(ThreadId, FamilyId)> = rows
        .iter()
        .map(|row| (ThreadId(id_from(&row.id)), FamilyId(id_from(&row.family_id))))
        .filter(|(thread, _)| !selector.is_empty() && thread.to_string().ends_with(selector))
        .collect();
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(forbidden("only a thread's owner makes read-only links to it")),
        _ => Err(bad_request("that names several threads")),
    }
}

/// Whether the thread's org lets its threads have links made at `made_ms`.
async fn org_allows(server: &Server, thread: ThreadId, made_ms: i64) -> Result<bool, sqlx::Error> {
    let id = id_bytes(thread.0);
    let org = sqlx::query!(
        r#"SELECT o.links AS "links: bool", o.links_off_ms FROM threads t
         JOIN projects p ON p.repo = t.repo JOIN orgs o ON o.id = p.org_id WHERE t.id = ?"#,
        id
    )
    .fetch_optional(server.db())
    .await?;
    Ok(org.is_none_or(|org| org.links && made_ms > org.links_off_ms))
}

async fn list(server: &Server, user: UserId, selector: &str) -> Result<Vec<ThreadLink>, ApiError> {
    let (thread, _) = owned_thread(server, user, selector).await?;
    let id = id_bytes(thread.0);
    let rows = sqlx::query!(
        "SELECT id, created_ms, expires_ms, frozen_at, revoked_ms FROM thread_links WHERE thread_id = ? ORDER BY created_ms DESC",
        id
    )
    .fetch_all(server.db())
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| ThreadLink {
            id: row.id,
            created_ms: row.created_ms,
            expires_ms: row.expires_ms,
            frozen: row.frozen_at.is_some(),
            revoked: row.revoked_ms.is_some(),
            url: None,
        })
        .collect())
}

async fn make(server: &Server, user: UserId, selector: &str, new: NewLink) -> Result<ThreadLink, ApiError> {
    if ![1, 7, 30].contains(&new.days) {
        return Err(bad_request("a link lasts 1, 7 or 30 days"));
    }
    let (thread, family) = owned_thread(server, user, selector).await?;
    let now = now_ms();
    if !org_allows(server, thread, now + 1).await? {
        return Err(forbidden("read-only links are turned off for this thread's org"));
    }
    let frozen_at = match (new.frozen, server.room(family).await) {
        (false, _) => None,
        (true, Some(slot)) => Some(i64::try_from(slot.room.deltas().len()).unwrap_or(i64::MAX)),
        (true, None) => Some(0),
    };
    let token = crate::accounts::random_token();
    let (token_hash, thread_id, expires_ms) = (hash(&token), id_bytes(thread.0), now + i64::from(new.days) * DAY_MS);
    let id = sqlx::query_scalar!(
        r#"INSERT INTO thread_links (token_hash, thread_id, owner_id, created_ms, expires_ms, frozen_at)
         VALUES (?, ?, ?, ?, ?, ?) RETURNING id AS "id!""#,
        token_hash,
        thread_id,
        user,
        now,
        expires_ms,
        frozen_at
    )
    .fetch_one(server.db())
    .await?;
    let url = format!("{}/r/{token}", server.config().public_url().trim_end_matches('/'));
    Ok(ThreadLink { id, created_ms: now, expires_ms, frozen: new.frozen, revoked: false, url: Some(url) })
}

async fn revoke(server: &Server, user: UserId, selector: &str, link: i64) -> Result<StatusCode, ApiError> {
    let (thread, _) = owned_thread(server, user, selector).await?;
    let (thread, now) = (id_bytes(thread.0), now_ms());
    let revoked = sqlx::query!(
        "UPDATE thread_links SET revoked_ms = ? WHERE id = ? AND thread_id = ? AND revoked_ms IS NULL",
        now,
        link,
        thread
    )
    .execute(server.db())
    .await?;
    if revoked.rows_affected() == 0 {
        return Err(not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn browser_list(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
) -> Result<Json<Vec<ThreadLink>>, ApiError> {
    Ok(Json(list(&server, browser.user, &id).await?))
}

async fn browser_make(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
    Json(new): Json<NewLink>,
) -> Result<Json<ThreadLink>, ApiError> {
    browser.may(browser.capabilities == silt_proto::Capabilities::ALL, "share threads")?;
    Ok(Json(make(&server, browser.user, &id, new).await?))
}

async fn browser_revoke(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((id, link)): Path<(String, i64)>,
) -> Result<StatusCode, ApiError> {
    revoke(&server, browser.user, &id, link).await
}

async fn device_list(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Result<Json<Vec<ThreadLink>>, ApiError> {
    Ok(Json(list(&server, caller.user, &id).await?))
}

async fn device_make(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
    Json(new): Json<NewLink>,
) -> Result<Json<ThreadLink>, ApiError> {
    Ok(Json(make(&server, caller.user, &id, new).await?))
}

async fn device_revoke(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Path((id, link)): Path<(String, i64)>,
) -> Result<StatusCode, ApiError> {
    revoke(&server, caller.user, &id, link).await
}

/// Counts a token that opened nothing.
fn missed(server: &Server) {
    let mut misses = server.shared.link_misses.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    misses.retain(|at| at.elapsed() < MISS_WINDOW);
    misses.push(Instant::now());
}

fn pausing(server: &Server) -> bool {
    let misses = server.shared.link_misses.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    misses.iter().filter(|at| at.elapsed() < MISS_WINDOW).count() > MISSES_ALLOWED
}

/// Never cached, followed from, or indexed (ADR 0027).
fn private_page(status: StatusCode, html: String) -> Response {
    let mut response = (status, Html(html)).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert("x-robots-tag", HeaderValue::from_static("noindex"));
    response
}

fn shell(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex"><title>{title} · Silt</title>
<style>
body {{ font: 15px/1.55 system-ui, sans-serif; max-width: 46rem; margin: 2rem auto; padding: 0 1rem; color: #1f2328; background: #fff; }}
h1 {{ font-size: 1.35rem; margin-bottom: .2rem; }} .meta {{ color: #59636e; font-size: .85rem; }}
.entry {{ margin: .7rem 0; white-space: pre-wrap; overflow-wrap: anywhere; }} .user {{ font-weight: 600; }}
.tool, .compaction {{ color: #59636e; font-size: .85rem; }} .turn {{ margin-top: 1.4rem; color: #59636e; font-size: .75rem; text-transform: uppercase; letter-spacing: .05em; }}
.rewound {{ opacity: .5; }}
@media (prefers-color-scheme: dark) {{ body {{ background: #0d1117; color: #e6edf3; }} .meta, .tool, .compaction, .turn {{ color: #9198a1; }} }}
</style></head><body>{body}</body></html>"#,
        title = escape(title),
    )
}

fn gone() -> String {
    shell("Not available", "<h1>Not available</h1><p>This link expired, was revoked, or never existed.</p>")
}

/// The thread's own conversation for anyone holding a working link.
async fn read(State(server): State<Server>, Path(token): Path<String>) -> Response {
    if pausing(&server) {
        return private_page(
            StatusCode::TOO_MANY_REQUESTS,
            shell("Wait a minute", "<p>Too many links were tried. Try again in a minute.</p>"),
        );
    }
    match shown(&server, &token).await {
        Ok(Some(html)) => private_page(StatusCode::OK, html),
        Ok(None) => {
            missed(&server);
            private_page(StatusCode::NOT_FOUND, gone())
        }
        Err(error) => {
            tracing::warn!(%error, "could not show a read-only link");
            private_page(StatusCode::INTERNAL_SERVER_ERROR, shell("Something went wrong", "<p>Could not show this thread.</p>"))
        }
    }
}

async fn shown(server: &Server, token: &str) -> Result<Option<String>, sqlx::Error> {
    let (token_hash, now) = (hash(token), now_ms());
    let Some(link) = sqlx::query!(
        r#"SELECT l.thread_id, l.created_ms, l.expires_ms, l.frozen_at, l.revoked_ms,
             t.family_id, t.deleted_ms, t.visibility, u.name AS owner
         FROM thread_links l JOIN threads t ON t.id = l.thread_id JOIN users u ON u.id = l.owner_id
         WHERE l.token_hash = ?"#,
        token_hash
    )
    .fetch_optional(server.db())
    .await?
    else {
        return Ok(None);
    };
    let thread = ThreadId(id_from(&link.thread_id));
    let works = link.revoked_ms.is_none()
        && link.expires_ms > now
        && link.deleted_ms.is_none()
        && link.visibility.as_deref() != Some("private")
        && org_allows(server, thread, link.created_ms).await?;
    if !works {
        return Ok(None);
    }
    let log: Vec<Delta> = match server.room(FamilyId(id_from(&link.family_id))).await {
        Some(slot) => {
            let deltas = slot.room.deltas();
            let upto = link.frozen_at.map_or(deltas.len(), |at| usize::try_from(at).unwrap_or(usize::MAX).min(deltas.len()));
            deltas[..upto].to_vec()
        }
        None => Vec::new(),
    };
    Ok(Some(render(&log, thread, &link.owner, link.frozen_at.map(|_| link.created_ms))))
}

/// The conversation as plain text with turn markers; a fork's note says so.
fn render(log: &[Delta], thread: ThreadId, owner: &str, frozen_at: Option<i64>) -> String {
    let entries = crate::browse::entries(log, thread);
    let own: Vec<&Delta> = log.iter().filter(|delta| delta.thread == thread).collect();
    let title = crate::browse::title(&own, crate::browse::first_prompt(&entries)).unwrap_or_else(|| "A Silt thread".into());
    let forked = own.iter().any(|delta| matches!(delta.op, silt_log::Op::Fork { .. }));
    let mut body = format!("<h1>{}</h1><p class=\"meta\">by {}", escape(&title), escape(owner));
    if let Some(at) = frozen_at {
        body.push_str(&format!(" · as it was at {}", escape(&time_text(at))));
    }
    body.push_str("</p>");
    if forked {
        body.push_str("<p class=\"meta\">This thread is a fork; what came before it isn't shown.</p>");
    }
    let mut turn = 0;
    for (_, entry) in &entries {
        let rewound = if entry.rewound { " rewound" } else { "" };
        let class = match entry.kind.as_str() {
            kind @ ("user" | "wake") => {
                turn += 1;
                let note = if entry.rewound { " · rewound by a revert" } else { "" };
                body.push_str(&format!("<div class=\"turn{rewound}\">Turn {turn}{note}</div>"));
                if kind == "wake" { "compaction" } else { "user" }
            }
            "tool_call" => "tool",
            "compaction" => "compaction",
            _ => "assistant",
        };
        body.push_str(&format!("<div class=\"entry {class}{rewound}\">{}</div>", escape(&entry.text)));
    }
    if entries.is_empty() {
        body.push_str("<p class=\"meta\">Nothing was said in this thread yet.</p>");
    }
    shell(&title, &body)
}

/// A moment as UTC date and time, for the frozen note.
fn time_text(ms: i64) -> String {
    let seconds = ms.div_euclid(1000);
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02} UTC", rest / 3600, rest % 3600 / 60)
}

/// The proleptic Gregorian date `days` after 1970-01-01.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_times_read_as_dates() {
        assert_eq!(time_text(0), "1970-01-01 00:00 UTC");
        assert_eq!(time_text(1_792_547_100_000), "2026-10-21 01:45 UTC");
        assert_eq!(time_text(951_782_400_000), "2000-02-29 00:00 UTC");
    }
}
