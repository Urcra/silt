//! The forge bridge (ADR 0042): a thread's open pull request gets one
//! comment naming the thread, kept current as the family changes.

use std::collections::{BTreeSet, HashSet};
use std::sync::Weak;
use std::time::Duration;

use serde_json::Value;
use silt_log::{ConvOp, Delta, GitOp, MrStatus, Op, merge_requests_in};
use silt_types::{FamilyId, ThreadId};
use tokio::sync::mpsc;
use tracing::warn;

use crate::config::Forge;
use crate::forge::{self, Api, Call, Linked, Method, ThreadComment};
use crate::{Server, Shared, id_bytes, id_from};

/// How long the bridge lets a family's changes gather before it looks.
const SETTLE: Duration = Duration::from_secs(2);

pub(crate) type Jobs = mpsc::UnboundedSender<FamilyId>;

/// Starts the bridge when the config names a forge.
pub(crate) fn start(shared: Weak<Shared>, jobs: mpsc::UnboundedReceiver<FamilyId>) {
    tokio::spawn(run(shared, jobs));
}

/// Whether `deltas` change what a comment would say.
pub(crate) fn touches(deltas: &[Delta]) -> bool {
    deltas.iter().any(|delta| {
        matches!(
            delta.op,
            Op::Git(GitOp::Committed { .. } | GitOp::Genesis { .. })
                | Op::Conv(ConvOp::Title(_) | ConvOp::Merged { .. })
                | Op::Fork { .. }
                | Op::Merge { .. }
                | Op::Mr(_)
        )
    })
}

async fn run(shared: Weak<Shared>, mut jobs: mpsc::UnboundedReceiver<FamilyId>) {
    while let Some(first) = jobs.recv().await {
        tokio::time::sleep(SETTLE).await;
        let mut families = BTreeSet::from([first]);
        while let Ok(more) = jobs.try_recv() {
            families.insert(more);
        }
        let Some(shared) = shared.upgrade() else {
            return;
        };
        let server = Server { shared };
        for family in families {
            if let Err(error) = sync_family(&server, family).await {
                warn!(%error, %family, "could not update the family's pull request comments");
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum BridgeError {
    #[error("database error")]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Config(#[from] crate::ConfigError),
}

async fn sync_family(server: &Server, family: FamilyId) -> Result<(), BridgeError> {
    let family_id = id_bytes(family.0);
    let rows = sqlx::query!(
        r#"SELECT t.id, t.repo FROM threads t JOIN projects p ON p.repo = t.repo
         WHERE t.family_id = ? AND t.deleted_ms IS NULL AND p.forge_comments = 1"#,
        family_id
    )
    .fetch_all(server.db())
    .await?;
    if rows.is_empty() {
        return Ok(());
    }
    let forks = server.shared.index.lock().await.forks().clone();
    let shown = crate::visibility::not_private(server.db(), &forks).await?;
    let Some(slot) = server.room(family).await else {
        return Ok(());
    };
    let log = slot.room.deltas().to_vec();
    let heads: Vec<(ThreadId, silt_types::Frontier)> = rows
        .iter()
        .map(|row| ThreadId(id_from(&row.id)))
        .map(|thread| (thread, silt_log::thread_head_in(&log, slot.room.graph(), thread)))
        .collect();
    let seen: Vec<(ThreadId, silt_crdt::VersionVector)> =
        heads.iter().map(|(thread, head)| (*thread, slot.room.graph().version_vector(head))).collect();
    drop(slot);
    let link = |thread: ThreadId| format!("{}/app/t/{thread}", server.config().public_url());
    for (row, (thread, seen)) in rows.iter().zip(seen) {
        if !shown.contains(&thread) {
            continue;
        }
        let Some((host, path)) = forge::split_repo(&row.repo) else {
            continue;
        };
        let Some(config) = server.config().forges.iter().find(|forge| forge.host.eq_ignore_ascii_case(host)) else {
            continue;
        };
        // Only the thread's own commits: a fork that hasn't committed would
        // otherwise take its source's branch and comment on its pull request.
        let own = |delta: &&Delta| delta.thread == thread && seen.contains(delta.span.first());
        let branch = log.iter().rev().filter(own).find_map(|delta| match &delta.op {
            Op::Git(GitOp::Committed { branch, .. } | GitOp::Genesis { branch, .. }) => branch.clone(),
            _ => None,
        });
        let Some(branch) = branch else {
            continue;
        };
        let comment = comment_of(&log, thread, &shown, &link);
        let api = Api {
            kind: config.kind,
            base: config
                .api_url
                .clone()
                .unwrap_or_else(|| forge::default_api(config.kind, host))
                .trim_end_matches('/')
                .to_owned(),
            path: path.to_owned(),
        };
        if let Err(error) = post(server, config, &api, &branch, &comment).await {
            warn!(error, thread = %thread, host, "could not comment on the thread's pull request");
        }
    }
    Ok(())
}

/// What the comment on `thread`'s pull request says, naming only what
/// `shown` holds.
fn comment_of(log: &[Delta], thread: ThreadId, shown: &HashSet<ThreadId>, link: &dyn Fn(ThreadId) -> String) -> ThreadComment {
    let title_of = |thread: ThreadId| {
        let own: Vec<&Delta> = log.iter().filter(|delta| delta.thread == thread).collect();
        let history = crate::browse::entries(log, thread);
        crate::browse::title(&own, crate::browse::first_prompt(&history)).unwrap_or_else(|| format!("thread {}", thread.short()))
    };
    let requests = merge_requests_in(log);
    let forks = log
        .iter()
        .filter_map(|delta| match delta.op {
            Op::Fork { from } if from == thread && shown.contains(&delta.thread) => Some(delta.thread),
            _ => None,
        })
        .map(|fork| {
            let latest = requests.iter().filter(|request| request.fork == fork).max_by_key(|request| request.opened_ms);
            let state = match latest.map(|request| &request.status) {
                Some(MrStatus::Approved) => "merged",
                Some(MrStatus::Declined { .. }) => "declined",
                Some(MrStatus::Open) => "merge request open",
                None => "in progress",
            };
            Linked { title: title_of(fork), link: link(fork), state: state.into() }
        })
        .collect();
    let merged = log
        .iter()
        .filter(|delta| delta.thread == thread)
        .filter_map(|delta| match delta.op {
            Op::Merge { from, mr: Some(_) } if shown.contains(&from) => Some(from),
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|from| Linked { title: title_of(from), link: link(from), state: "merged".into() })
        .collect();
    ThreadComment {
        thread: thread.to_string(),
        title: title_of(thread),
        link: link(thread),
        turns: log
            .iter()
            .filter(|delta| {
                delta.thread == thread
                    && matches!(delta.op, Op::Conv(ConvOp::Turn { mark: silt_log::TurnMark::PromptSubmitted, .. }))
            })
            .count(),
        commits: silt_log::commits_in(log, thread).len(),
        summary: requests
            .iter()
            .filter(|request| request.fork == thread && request.status == MrStatus::Open)
            .max_by_key(|request| request.opened_ms)
            .map(|request| request.summary.clone()),
        forks,
        merged,
    }
}

/// Posts the comment on the open pull request from `branch`, or edits the
/// one posted before.
async fn post(server: &Server, config: &Forge, api: &Api, branch: &str, comment: &ThreadComment) -> Result<(), String> {
    let token = crate::config::secret(config.token.as_ref(), config.token_file.as_ref()).map_err(|error| error.to_string())?;
    let me = send(server, api, &token, &api.me()).await?;
    let me = forge::read_me(api.kind, &me).ok_or("the forge didn't say who the token is")?;
    let pulls = send(server, api, &token, &api.open_pulls(branch)).await?;
    let Some(pull) = forge::read_pull(api.kind, &pulls, branch, &api.path) else {
        return Ok(());
    };
    let comments = send(server, api, &token, &api.comments(pull)).await?;
    let body = comment.body();
    let call = match forge::read_comment(api.kind, &comments, &forge::marker(&comment.thread), &me) {
        Some(id) => api.update(pull, id, &body),
        None => api.create(pull, &body),
    };
    send(server, api, &token, &call).await?;
    Ok(())
}

async fn send(server: &Server, api: &Api, token: &str, call: &Call) -> Result<Value, String> {
    let method = match call.method {
        Method::Get => reqwest::Method::GET,
        Method::Post => reqwest::Method::POST,
        Method::Patch => reqwest::Method::PATCH,
        Method::Put => reqwest::Method::PUT,
    };
    let (header, value) = forge::auth_header(api.kind, token);
    let mut request = server
        .http()
        .request(method, &call.url)
        .header(header, value)
        .header("Accept", "application/json")
        .header("User-Agent", "silt");
    if let Some(body) = &call.body {
        request = request.json(body);
    }
    let response = request.send().await.map_err(|error| error.to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("{} {} answered {status}", call_name(call.method), call.url));
    }
    response.json().await.map_err(|error| error.to_string())
}

fn call_name(method: Method) -> &'static str {
    match method {
        Method::Get => "GET",
        Method::Post => "POST",
        Method::Patch => "PATCH",
        Method::Put => "PUT",
    }
}
