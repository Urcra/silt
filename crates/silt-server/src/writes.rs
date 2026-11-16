//! What the hosted web app changes in a family (ADR 0026): comments,
//! replies and merge request verdicts. The server writes each op itself,
//! with an actor of its own for the person, and pushes it as one of their
//! devices would, through the same checks and notifications.

use axum::extract::{Extension, Path, State};
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use silt_log::{Author, CommentOp, CommentTarget, ConvOp, Delta, MrOp, MrStatus, Op};
use silt_proto::PushRequest;
use silt_proto::view::{CommentOn, CommentView, MrSummary};
use silt_types::{ActorId, DeltaId, DeltaSpan, DeviceId, FamilyId, Frontier, ThreadId};

use crate::app::Browser;
use crate::browse::{self, short};
use crate::{ApiError, Caller, Server, UserId, bad_request, id_bytes, id_from, not_found, now_ms};

pub(crate) fn routes() -> Router<Server> {
    Router::new()
        .route("/app/api/threads/{id}/comments", post(add_comment))
        .route("/app/api/threads/{id}/comments/sent", post(comments_sent))
        .route("/app/api/threads/{id}/comments/{comment}/replies", post(reply))
        .route("/app/api/threads/{id}/comments/{comment}/resolve", post(resolve))
        .route("/app/api/mrs/{id}/approve", post(approve))
        .route("/app/api/mrs/{id}/decline", post(decline))
        .route("/app/api/mrs/{id}/request-changes", post(request_changes))
        .route("/app/api/mrs/{id}/guide", post(guide))
}

/// The actor the server writes as for `user`, made on first use.
pub(crate) async fn actor_of(server: &Server, user: UserId) -> Result<ActorId, sqlx::Error> {
    if let Some(actor) =
        sqlx::query_scalar!("SELECT actor FROM server_actors WHERE user_id = ?", user).fetch_optional(server.db()).await?
    {
        return Ok(ActorId(id_from(&actor)));
    }
    let actor = id_bytes(uuid::Uuid::new_v4().as_u128());
    sqlx::query!("INSERT OR IGNORE INTO server_actors (user_id, actor) VALUES (?, ?)", user, actor).execute(server.db()).await?;
    let actor = sqlx::query_scalar!("SELECT actor FROM server_actors WHERE user_id = ?", user).fetch_one(server.db()).await?;
    Ok(ActorId(id_from(&actor)))
}

/// Writes `op` on `thread` as `user`, after the thread's head and `also`,
/// when the device may comment or review: what a phone limited on the
/// Devices page may not do (ADR 0028), such as publishing a version.
pub(crate) async fn write_as(
    server: &Server,
    browser: Browser,
    family: FamilyId,
    thread: ThreadId,
    also: Option<DeltaId>,
    op: Op,
) -> Result<DeltaId, ApiError> {
    browser.may(browser.capabilities.review, "comment or review")?;
    write_as_with(server, browser, family, thread, also, op).await
}

/// [`write_as`] for what any session that can read a thread may write:
/// only an artifact's page data and the files people add to a page
/// (ADR 0072), whose rules the push checks.
pub(crate) async fn write_as_with(
    server: &Server,
    browser: Browser,
    family: FamilyId,
    thread: ThreadId,
    also: Option<DeltaId>,
    op: Op,
) -> Result<DeltaId, ApiError> {
    let written = write_all_as_with(server, browser, family, thread, also, vec![op]).await?;
    written.first().copied().ok_or_else(|| crate::forbidden("you may not write in this thread"))
}

/// [`write_all_as_with`] when the device may comment or review.
pub(crate) async fn write_all_as(
    server: &Server,
    browser: Browser,
    family: FamilyId,
    thread: ThreadId,
    also: Option<DeltaId>,
    ops: Vec<Op>,
) -> Result<Vec<DeltaId>, ApiError> {
    browser.may(browser.capabilities.review, "comment or review")?;
    write_all_as_with(server, browser, family, thread, also, ops).await
}

/// Writes `ops` on `thread` as `user`, each after the one before, as one
/// push under one turn of the write lock: none of another request's
/// writes lands between them, and the push takes them or refuses the rest.
pub(crate) async fn write_all_as_with(
    server: &Server,
    browser: Browser,
    family: FamilyId,
    thread: ThreadId,
    also: Option<DeltaId>,
    ops: Vec<Op>,
) -> Result<Vec<DeltaId>, ApiError> {
    if ops.is_empty() {
        return Ok(Vec::new());
    }
    // One at a time: each takes the next lamport, which a write between
    // working it out and pushing would take too.
    let one = server.shared.writes.lock().await;
    push_all(server, &one, browser, family, thread, also, ops).await
}

/// [`write_all_as_with`] for a caller already holding the write lock, as
/// `one`, so what it checked before writing still holds when it writes.
pub(crate) async fn push_all(
    server: &Server,
    _one: &tokio::sync::MutexGuard<'_, ()>,
    browser: Browser,
    family: FamilyId,
    thread: ThreadId,
    also: Option<DeltaId>,
    ops: Vec<Op>,
) -> Result<Vec<DeltaId>, ApiError> {
    let user = browser.user;
    if ops.is_empty() {
        return Ok(Vec::new());
    }
    let actor = actor_of(server, user).await?;
    let deltas = {
        let slot = server.room(family).await.ok_or_else(not_found)?;
        let head = silt_log::thread_head_in(slot.room.deltas(), slot.room.graph(), thread);
        let mut parents: Frontier = head.ids().iter().copied().chain(also).collect();
        let mut lamport = slot.room.graph().next_lamport();
        let mut deltas = Vec::with_capacity(ops.len());
        for op in ops {
            let span = DeltaSpan::new(actor, lamport, op.units() as u64);
            lamport = silt_types::Lamport(lamport.0 + op.units() as u64);
            let delta = Delta { span, parents, thread, author: Author::User, time_ms: now_ms(), op };
            parents = Frontier::single(delta.span.last());
            deltas.push(delta);
        }
        deltas
    };
    let ids: Vec<DeltaId> = deltas.iter().map(|delta| delta.span.first()).collect();
    let wanted = deltas.len();
    let caller = Caller { device: DeviceId(0), user };
    let Json(pushed) =
        crate::push(State(server.clone()), Extension(caller), Path(family.to_string()), Json(PushRequest { deltas })).await?;
    if pushed.accepted == 0 {
        return Err(crate::forbidden("you may not write in this thread"));
    }
    if (pushed.accepted as usize) < wanted {
        return Err(crate::forbidden("only some of that could be written"));
    }
    Ok(ids)
}

async fn comment_view(server: &Server, browser: Browser, thread: ThreadId, id: DeltaId) -> Result<Json<CommentView>, ApiError> {
    let Json(views) = browse::comments(State(server.clone()), Extension(browser), Path(thread.to_string())).await?;
    views.into_iter().find(|view| view.id == short(id)).map(Json).ok_or_else(not_found)
}

#[derive(Deserialize)]
struct NewComment {
    on: CommentOn,
    text: String,
}

fn said(text: &str) -> Result<String, ApiError> {
    let text = text.trim();
    if text.is_empty() { Err(bad_request("there is nothing to say")) } else { Ok(text.to_owned()) }
}

async fn add_comment(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
    Json(body): Json<NewComment>,
) -> Result<Json<CommentView>, ApiError> {
    let text = said(&body.text)?;
    let seen = browse::seen_thread(&server, browser.user, &id).await?;
    let target = match body.on {
        CommentOn::Entry { index } => {
            let (delta, _) = seen.entries.get(index).ok_or_else(|| bad_request("no such message"))?;
            CommentTarget::Message { delta: *delta }
        }
        CommentOn::Doc { artifact, start, end } => {
            let (artifact, doc) = browse::seen_doc(&server, browser.user, &id, &artifact).await?;
            silt_proto::edited::doc_comment_target(&doc, artifact, start, end).map_err(bad_request)?
        }
        CommentOn::Cell { artifact, sheet, address } => {
            let (artifact, cells) = browse::seen_sheet(&server, browser.user, &id, &artifact, sheet).await?;
            silt_proto::edited::cell_comment_target(&cells, artifact, sheet, &address).map_err(bad_request)?
        }
        CommentOn::Element { artifact, anchor } => {
            let html = seen
                .entries
                .iter()
                .filter_map(|(_, entry)| entry.artifact.as_ref())
                .any(|view| view.id == artifact && view.kind == "html");
            let id: silt_types::ArtifactId = artifact.parse().map_err(|_| bad_request("not an artifact id"))?;
            if !html {
                return Err(bad_request(format!("this thread has no page {artifact}")));
            }
            CommentTarget::Element { artifact: id, anchor: silt_proto::artifact::element_anchor(&anchor).map_err(bad_request)? }
        }
        CommentOn::Lines { .. } | CommentOn::Anchored { .. } => {
            return Err(bad_request("comment on code from a machine with the thread checked out"));
        }
    };
    let op = Op::Comment(CommentOp::Added { target, text, by: seen.me });
    let comment = write_as(&server, browser, seen.family, seen.thread, None, op).await?;
    comment_view(&server, browser, seen.thread, comment).await
}

#[derive(Deserialize)]
struct NewReply {
    text: String,
}

async fn reply(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((id, comment)): Path<(String, String)>,
    Json(body): Json<NewReply>,
) -> Result<Json<CommentView>, ApiError> {
    let text = said(&body.text)?;
    let seen = browse::seen_thread(&server, browser.user, &id).await?;
    let parent = seen.comments.iter().find(|candidate| short(candidate.id) == comment).ok_or_else(not_found)?.id;
    let op = Op::Comment(CommentOp::Replied { parent, text, by: seen.me });
    write_as(&server, browser, seen.family, seen.thread, None, op).await?;
    comment_view(&server, browser, seen.thread, parent).await
}

async fn resolve(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path((id, comment)): Path<(String, String)>,
) -> Result<Json<CommentView>, ApiError> {
    let seen = browse::seen_thread(&server, browser.user, &id).await?;
    let comment = seen.comments.iter().find(|candidate| short(candidate.id) == comment).ok_or_else(not_found)?.id;
    write_as(&server, browser, seen.family, seen.thread, None, Op::Comment(CommentOp::Resolved { comment })).await?;
    comment_view(&server, browser, seen.thread, comment).await
}

#[derive(Deserialize)]
struct SentComments {
    comments: Vec<String>,
}

/// Records that comments went to the thread's agent in a prompt, as a
/// machine does once a review or an artifact's comments are sent (ADR 0055,
/// ADR 0068).
async fn comments_sent(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
    Json(body): Json<SentComments>,
) -> Result<Json<Vec<CommentView>>, ApiError> {
    let seen = browse::seen_thread(&server, browser.user, &id).await?;
    let comments = body
        .comments
        .iter()
        .map(|wanted| {
            seen.comments.iter().find(|candidate| short(candidate.id) == *wanted).map(|found| found.id).ok_or_else(not_found)
        })
        .collect::<Result<Vec<DeltaId>, ApiError>>()?;
    if comments.is_empty() {
        return Err(bad_request("name the comments that were sent"));
    }
    write_as(&server, browser, seen.family, seen.thread, None, Op::Comment(CommentOp::Sent { comments: comments.clone() }))
        .await?;
    let mut views = Vec::new();
    for comment in comments {
        views.push(comment_view(&server, browser, seen.thread, comment).await?.0);
    }
    Ok(Json(views))
}

async fn summary(server: &Server, browser: Browser, id: DeltaId) -> Result<Json<MrSummary>, ApiError> {
    let Json(details) = browse::merge_request(State(server.clone()), Extension(browser), Path(id.to_string())).await?;
    Ok(Json(details.mr))
}

/// Merges the fork into its target as approving it on a daemon does: the
/// merge, the verdict, and the merge in the target's conversation.
async fn approve(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
) -> Result<Json<MrSummary>, ApiError> {
    let (family, request, _) = browse::seen_request(&server, browser.user, &id).await?;
    if request.status != MrStatus::Open {
        return Err(bad_request("the merge request isn't open"));
    }
    let merge = Op::Merge { from: request.fork, mr: Some(request.id) };
    write_as(&server, browser, family, request.target, Some(request.id), merge).await?;
    write_as(&server, browser, family, request.target, None, Op::Mr(MrOp::Approved { mr: request.id })).await?;
    let merged = ConvOp::Merged { from: request.fork, mr: request.id, title: request.title, summary: request.summary };
    write_as(&server, browser, family, request.target, None, Op::Conv(merged)).await?;
    summary(&server, browser, request.id).await
}

#[derive(Deserialize, Default)]
struct Reason {
    #[serde(default)]
    reason: Option<String>,
}

async fn decline(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
    body: Option<Json<Reason>>,
) -> Result<Json<MrSummary>, ApiError> {
    let reason = body.map(|Json(body)| body).unwrap_or_default().reason.unwrap_or_default();
    let (family, request, _) = browse::seen_request(&server, browser.user, &id).await?;
    if request.status != MrStatus::Open {
        return Err(bad_request("the merge request isn't open"));
    }
    let op = Op::Mr(MrOp::Declined { mr: request.id, reason });
    write_as(&server, browser, family, request.target, None, op).await?;
    summary(&server, browser, request.id).await
}

async fn request_changes(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
    Json(body): Json<Reason>,
) -> Result<Json<MrSummary>, ApiError> {
    let reason = said(body.reason.as_deref().unwrap_or_default()).map_err(|_| bad_request("say what should change"))?;
    let (family, request, _) = browse::seen_request(&server, browser.user, &id).await?;
    if request.status != MrStatus::Open {
        return Err(bad_request("the merge request isn't open"));
    }
    let op = Op::Mr(MrOp::ChangesRequested { mr: request.id, reason });
    write_as(&server, browser, family, request.target, None, op).await?;
    summary(&server, browser, request.id).await
}

#[derive(Deserialize, Default)]
struct Guide {
    #[serde(default)]
    text: Option<String>,
}

/// Keeps a change guide written in the browser; having the fork's agent
/// write one needs the machine it runs on.
async fn guide(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
    body: Option<Json<Guide>>,
) -> Result<Json<&'static str>, ApiError> {
    let text = body.map(|Json(body)| body).unwrap_or_default().text.unwrap_or_default();
    let guide = said(&text).map_err(|_| bad_request("write the guide here, or ask the fork's agent from its machine"))?;
    let (family, request, me) = browse::seen_request(&server, browser.user, &id).await?;
    let op = Op::Mr(MrOp::Guided { mr: request.id, guide, by: me });
    write_as(&server, browser, family, request.fork, None, op).await?;
    Ok(Json("kept"))
}
