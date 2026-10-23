//! Projects as their orgs' members see them (ADR 0023).

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Extension, State};
use silt_proto::{GitNotes, ProjectInfo, ProjectsReply, Visibility, project_name};
use silt_types::ThreadId;
use sqlx::sqlite::SqlitePool;

use crate::{ApiError, Caller, Server, UserId, bad_request, not_found};

/// The projects of the caller's orgs, with how many of their threads the
/// caller can see.
pub(crate) async fn list(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<ProjectsReply>, ApiError> {
    let db = server.db();
    let viewer = server.viewer(caller.user).await?;
    let threads =
        sqlx::query!(r#"SELECT id AS "id!: Vec<u8>", repo FROM threads WHERE deleted_ms IS NULL"#).fetch_all(db).await?;
    let mut seen: HashMap<String, u32> = HashMap::new();
    for thread in threads {
        let Ok(bytes) = <[u8; 16]>::try_from(thread.id.as_slice()) else {
            continue;
        };
        if viewer.sees_thread(ThreadId(u128::from_be_bytes(bytes))) {
            *seen.entry(thread.repo).or_default() += 1;
        }
    }
    let rows = sqlx::query!(
        r#"SELECT p.repo AS "repo!", o.name AS org, p.name, p.origin_url, p.visibility, p.default_agent, p.default_preset,
            p.git_notes, p.forge_comments AS "forge_comments: bool"
        FROM projects p LEFT JOIN orgs o ON o.id = p.org_id
        WHERE p.org_id IN (SELECT org_id FROM org_members WHERE user_id = ?)
        ORDER BY p.repo"#,
        caller.user,
    )
    .fetch_all(db)
    .await?;
    let projects = rows
        .into_iter()
        .filter(|row| !row.repo.is_empty())
        .map(|row| ProjectInfo {
            name: row.name.unwrap_or_else(|| project_name(&row.repo)),
            threads: seen.get(&row.repo).copied().unwrap_or_default(),
            org: row.org,
            origin_url: row.origin_url,
            visibility: row.visibility.and_then(|visibility| visibility.parse::<Visibility>().ok()),
            default_agent: row.default_agent,
            default_preset: row.default_preset,
            git_notes: row.git_notes.parse().unwrap_or_default(),
            forge_comments: row.forge_comments,
            repo: row.repo,
        })
        .collect();
    Ok(Json(ProjectsReply { projects }))
}

/// Keeps where the project is cloned from, as the machine registering one
/// of its threads fetches it.
pub(crate) async fn note_origin(db: &SqlitePool, repo: &str, origin_url: &str) -> Result<(), sqlx::Error> {
    sqlx::query!("UPDATE projects SET origin_url = ? WHERE repo = ?", origin_url, repo).execute(db).await?;
    Ok(())
}

/// A setting as the change gives it: `None` to keep it, empty to clear it.
fn cleared(value: Option<String>) -> Option<Option<String>> {
    value.map(|value| Some(value.trim().to_owned()).filter(|value| !value.is_empty()))
}

/// Names agents and presets: what registries and daemons use for ids.
fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 100 && value.chars().all(|c| c.is_ascii_alphanumeric() || "-_.@/".contains(c))
}

/// What a settings change sets; `None` keeps a setting as it is.
pub(crate) struct SettingsChange {
    pub name: Option<String>,
    pub default_agent: Option<String>,
    pub default_preset: Option<String>,
    pub git_notes: Option<GitNotes>,
    pub forge_comments: Option<bool>,
}

pub(crate) async fn change_settings(db: &SqlitePool, caller: UserId, repo: &str, change: SettingsChange) -> Result<(), ApiError> {
    let SettingsChange { name, default_agent, default_preset, git_notes, forge_comments } = change;
    let org = sqlx::query_scalar!("SELECT org_id FROM projects WHERE repo = ?", repo)
        .fetch_optional(db)
        .await?
        .flatten()
        .ok_or_else(not_found)?;
    crate::orgs::require_owner(db, org, caller).await?;
    let (name, agent, preset) = (cleared(name), cleared(default_agent), cleared(default_preset));
    if name.as_ref().and_then(Option::as_ref).is_some_and(|name| name.chars().count() > 100) {
        return Err(bad_request("a project's name is at most 100 characters"));
    }
    if [&agent, &preset].into_iter().flatten().flatten().any(|id| !valid_id(id)) {
        return Err(bad_request("agents and presets are named by their ids"));
    }
    let mut tx = db.begin().await?;
    if let Some(name) = name {
        sqlx::query!("UPDATE projects SET name = ? WHERE repo = ?", name, repo).execute(&mut *tx).await?;
    }
    if let Some(agent) = agent {
        sqlx::query!("UPDATE projects SET default_agent = ? WHERE repo = ?", agent, repo).execute(&mut *tx).await?;
    }
    if let Some(preset) = preset {
        sqlx::query!("UPDATE projects SET default_preset = ? WHERE repo = ?", preset, repo).execute(&mut *tx).await?;
    }
    if let Some(notes) = git_notes.map(|notes| notes.to_string()) {
        sqlx::query!("UPDATE projects SET git_notes = ? WHERE repo = ?", notes, repo).execute(&mut *tx).await?;
    }
    if let Some(on) = forge_comments {
        sqlx::query!("UPDATE projects SET forge_comments = ? WHERE repo = ?", on, repo).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}
