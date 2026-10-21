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
        .route("/app/api/threads/{id}/comments/{comment}/replies", post(reply))
        .route("/app/api/threads/{id}/comments/{comment}/resolve", post(resolve))
        .route("/app/api/mrs/{id}/approve", post(approve))
        .route("/app/api/mrs/{id}/decline", post(decline))
        .route("/app/api/mrs/{id}/request-changes", post(request_changes))
        .route("/app/api/mrs/{id}/guide", post(guide))
}

/// The actor the server writes as for `user`, made on first use.
async fn actor_of(server: &Server, user: UserId) -> Result<ActorId, sqlx::Error> {
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

/// Writes `op` on `thread` as `user`, after the thread's head and `also`.
async fn write_as(
    server: &Server,
    browser: Browser,
    family: FamilyId,
    thread: ThreadId,
    also: Option<DeltaId>,
    op: Op,
) -> Result<DeltaId, ApiError> {
    browser.may(browser.capabilities.review, "comment or review")?;
    let user = browser.user;
    // One at a time: each takes the next lamport, which a write between
    // working it out and pushing would take too.
    let _one = server.shared.writes.lock().await;
    let actor = actor_of(server, user).await?;
    let delta = {
        let slot = server.room(family).await.ok_or_else(not_found)?;
        let head = silt_log::thread_head_in(slot.room.deltas(), slot.room.graph(), thread);
        let parents: Frontier = head.ids().iter().copied().chain(also).collect();
        let span = DeltaSpan::new(actor, slot.room.graph().next_lamport(), op.units() as u64);
        Delta { span, parents, thread, author: Author::User, time_ms: now_ms(), op }
    };
    let id = delta.span.first();
    let caller = Caller { device: DeviceId(0), user };
    let Json(pushed) = crate::push(
        State(server.clone()),
        Extension(caller),
        Path(family.to_string()),
        Json(PushRequest { deltas: vec![delta] }),
    )
    .await?;
    if pushed.accepted == 0 {
        return Err(crate::forbidden("you may not write in this thread"));
    }
    Ok(id)
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
    let CommentOn::Entry { index } = body.on else {
        return Err(bad_request("comment on code from a machine with the thread checked out"));
    };
    let (delta, _) = seen.entries.get(index).ok_or_else(|| bad_request("no such message"))?;
    let op = Op::Comment(CommentOp::Added { target: CommentTarget::Message { delta: *delta }, text, by: seen.me });
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
