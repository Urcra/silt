//! Orgs, their members and teams, and which org each project belongs to
//! (ADR 0013).

use axum::Json;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use silt_proto::{ErrorReply, OrgChange, OrgInfo, OrgMember, OrgRole, OrgsReply, TeamInfo, Visibility};
use sqlx::sqlite::SqlitePool;
use sqlx::{SqliteConnection, SqliteExecutor};

use crate::accounts::{self, valid_name};
use crate::{ApiError, Caller, Server, UserId, bad_request, forbidden, now_ms};

/// The org new users join unless an invite names another; it holds what
/// existed before orgs.
pub(crate) const DEFAULT_ORG: &str = "default";

pub(crate) async fn join_default_org(db: impl SqliteExecutor<'_>, user: UserId) -> Result<(), sqlx::Error> {
    let role = OrgRole::Member.as_str();
    sqlx::query!(
        "INSERT OR IGNORE INTO org_members (org_id, user_id, role) SELECT id, ?, ? FROM orgs WHERE name = ?",
        user,
        role,
        DEFAULT_ORG,
    )
    .execute(db)
    .await?;
    Ok(())
}

pub(crate) async fn add_member(db: impl SqliteExecutor<'_>, org: &str, user: UserId, owner: bool) -> Result<bool, sqlx::Error> {
    let role = if owner { OrgRole::Owner } else { OrgRole::Member }.as_str();
    let added = sqlx::query!(
        "INSERT INTO org_members (org_id, user_id, role) SELECT id, ?, ? FROM orgs WHERE name = ?
         ON CONFLICT (org_id, user_id) DO UPDATE SET role = excluded.role",
        user,
        role,
        org,
    )
    .execute(db)
    .await?
    .rows_affected();
    Ok(added > 0)
}

/// Puts a project nobody placed yet into its first thread's owner's org:
/// their only org, or else the first they joined.
pub(crate) async fn place_project(db: &SqlitePool, repo: &str, owner: UserId) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT OR IGNORE INTO projects (repo, org_id)
         SELECT ?, org_id FROM org_members WHERE user_id = ? ORDER BY rowid LIMIT 1",
        repo,
        owner,
    )
    .execute(db)
    .await?;
    Ok(())
}

async fn org_id(db: &SqlitePool, name: &str) -> Result<i64, ApiError> {
    sqlx::query_scalar!(r#"SELECT id AS "id!" FROM orgs WHERE name = ?"#, name)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| bad_request(format!("no org named {name}")))
}

/// The id of the user `asker` named. Unknown names count against the asker,
/// so nobody signed in can use this to list the server's accounts.
pub(crate) async fn named_user(server: &Server, asker: UserId, name: &str) -> Result<UserId, ApiError> {
    let guesses = format!("user-names:{asker}");
    let checks = &server.shared.passwords;
    if checks.cooling_down(&guesses) {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            ErrorReply::Invalid { message: "too many unknown user names; try again in a while".into() },
        ));
    }
    match accounts::user_by_name(server.db(), name).await? {
        Some(user) => Ok(user.id),
        None => {
            checks.record(&guesses, false);
            Err(bad_request(format!("no user named {name}")))
        }
    }
}

pub(crate) async fn is_admin(db: &SqlitePool, user: UserId) -> Result<bool, ApiError> {
    Ok(accounts::user_by_id(db, user).await?.is_some_and(|user| user.admin))
}

/// What a stored role says; an unknown one counts as a plain member.
fn stored_role(text: &str) -> OrgRole {
    text.parse().unwrap_or(OrgRole::Member)
}

async fn role(db: &SqlitePool, org: i64, user: UserId) -> Result<Option<OrgRole>, sqlx::Error> {
    Ok(sqlx::query_scalar!("SELECT role FROM org_members WHERE org_id = ? AND user_id = ?", org, user)
        .fetch_optional(db)
        .await?
        .map(|role| stored_role(&role)))
}

async fn owns(db: &SqlitePool, org: i64, user: UserId) -> Result<bool, ApiError> {
    Ok(role(db, org, user).await? == Some(OrgRole::Owner) || is_admin(db, user).await?)
}

/// Fails unless `user` owns `org` or administers the server.
pub(crate) async fn require_owner(db: &SqlitePool, org: i64, user: UserId) -> Result<(), ApiError> {
    if owns(db, org, user).await? { Ok(()) } else { Err(forbidden("only the org's owners and server admins can change it")) }
}

async fn team_id(db: &SqlitePool, org: i64, name: &str) -> Result<i64, ApiError> {
    sqlx::query_scalar!(r#"SELECT id AS "id!" FROM teams WHERE org_id = ? AND name = ?"#, org, name)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| bad_request(format!("no team named {name}")))
}

pub(crate) async fn list(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<OrgsReply>, ApiError> {
    let db = server.db();
    let admin = is_admin(db, caller.user).await?;
    let rows = sqlx::query!(
        r#"SELECT o.id AS "id!", o.name, o.default_visibility, o.links AS "links: bool", o.policy, m.role AS "role?" FROM orgs o
         LEFT JOIN org_members m ON m.org_id = o.id AND m.user_id = ?
         ORDER BY o.name"#,
        caller.user,
    )
    .fetch_all(db)
    .await?;
    let mut orgs = Vec::new();
    for row in rows {
        let role = match (row.role, admin) {
            (Some(role), _) => stored_role(&role),
            (None, true) => OrgRole::Admin,
            (None, false) => continue,
        };
        let members = sqlx::query!(
            "SELECT u.name, m.role FROM org_members m JOIN users u ON u.id = m.user_id WHERE m.org_id = ? ORDER BY u.name",
            row.id,
        )
        .fetch_all(db)
        .await?
        .into_iter()
        .map(|member| OrgMember { user: member.name, role: stored_role(&member.role) })
        .collect();
        let mut teams = Vec::new();
        for team in
            sqlx::query!(r#"SELECT id AS "id!", name FROM teams WHERE org_id = ? ORDER BY name"#, row.id).fetch_all(db).await?
        {
            let members = sqlx::query_scalar!(
                "SELECT u.name FROM team_members t JOIN users u ON u.id = t.user_id WHERE t.team_id = ? ORDER BY u.name",
                team.id,
            )
            .fetch_all(db)
            .await?;
            teams.push(TeamInfo { name: team.name, members });
        }
        let projects =
            sqlx::query_scalar!("SELECT repo FROM projects WHERE org_id = ? ORDER BY repo", row.id).fetch_all(db).await?;
        orgs.push(OrgInfo {
            name: row.name,
            may_change: admin || role == OrgRole::Owner,
            role,
            default_visibility: row.default_visibility.parse().unwrap_or(Visibility::Org),
            members,
            teams,
            projects,
            links: row.links,
            policy: serde_json::from_str(&row.policy).unwrap_or_default(),
        });
    }
    Ok(Json(OrgsReply { orgs }))
}

pub(crate) async fn change(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Json(change): Json<OrgChange>,
) -> Result<StatusCode, ApiError> {
    let db = server.db();
    match change {
        OrgChange::Create { org } => {
            if !is_admin(db, caller.user).await? {
                return Err(forbidden("only server admins create orgs"));
            }
            if !valid_name(&org) {
                return Err(bad_request("org names follow the rules for user names"));
            }
            let mut tx = crate::write_transaction(db).await?;
            let now = now_ms();
            let created = sqlx::query_scalar!(
                r#"INSERT INTO orgs (name, created_ms) VALUES (?, ?) ON CONFLICT (name) DO NOTHING RETURNING id AS "id!""#,
                org,
                now,
            )
            .fetch_optional(&mut *tx)
            .await?;
            let id = created.ok_or_else(|| bad_request(format!("an org named {org} exists")))?;
            let role = OrgRole::Owner.as_str();
            sqlx::query!("INSERT INTO org_members (org_id, user_id, role) VALUES (?, ?, ?)", id, caller.user, role)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        OrgChange::AddMember { org, user, owner } => {
            let org_id = org_id(db, &org).await?;
            require_owner(db, org_id, caller.user).await?;
            let user = named_user(&server, caller.user, &user).await?;
            let mut tx = crate::write_transaction(db).await?;
            add_member(&mut *tx, &org, user, owner).await?;
            keep_an_owner(&mut tx, org_id).await?;
            tx.commit().await?;
        }
        OrgChange::RemoveMember { org, user } => {
            let org_id = org_id(db, &org).await?;
            require_owner(db, org_id, caller.user).await?;
            let user = named_user(&server, caller.user, &user).await?;
            let mut tx = crate::write_transaction(db).await?;
            sqlx::query!(
                "DELETE FROM team_members WHERE user_id = ? AND team_id IN (SELECT id FROM teams WHERE org_id = ?)",
                user,
                org_id,
            )
            .execute(&mut *tx)
            .await?;
            sqlx::query!("DELETE FROM org_members WHERE org_id = ? AND user_id = ?", org_id, user).execute(&mut *tx).await?;
            keep_an_owner(&mut tx, org_id).await?;
            tx.commit().await?;
        }
        OrgChange::CreateTeam { org, team } => {
            let org_id = org_id(db, &org).await?;
            require_owner(db, org_id, caller.user).await?;
            if !valid_name(&team) {
                return Err(bad_request("team names follow the rules for user names"));
            }
            sqlx::query!("INSERT OR IGNORE INTO teams (org_id, name) VALUES (?, ?)", org_id, team).execute(db).await?;
        }
        OrgChange::RemoveTeam { org, team } => {
            let org_id = org_id(db, &org).await?;
            require_owner(db, org_id, caller.user).await?;
            let id = team_id(db, org_id, &team).await?;
            let mut tx = crate::write_transaction(db).await?;
            sqlx::query!("DELETE FROM teams WHERE id = ?", id).execute(&mut *tx).await?;
            forget_team_visibility(&mut tx, org_id, &Visibility::Team(team).to_string()).await?;
            tx.commit().await?;
        }
        OrgChange::AddToTeam { org, team, user } => {
            let org_id = org_id(db, &org).await?;
            require_owner(db, org_id, caller.user).await?;
            let team = team_id(db, org_id, &team).await?;
            let user = named_user(&server, caller.user, &user).await?;
            if role(db, org_id, user).await?.is_none() {
                return Err(bad_request("add the user to the org first"));
            }
            sqlx::query!("INSERT OR IGNORE INTO team_members (team_id, user_id) VALUES (?, ?)", team, user).execute(db).await?;
        }
        OrgChange::RemoveFromTeam { org, team, user } => {
            let org_id = org_id(db, &org).await?;
            require_owner(db, org_id, caller.user).await?;
            let team = team_id(db, org_id, &team).await?;
            let user = named_user(&server, caller.user, &user).await?;
            sqlx::query!("DELETE FROM team_members WHERE team_id = ? AND user_id = ?", team, user).execute(db).await?;
        }
        OrgChange::DefaultVisibility { org, visibility } => {
            let org_id = org_id(db, &org).await?;
            require_owner(db, org_id, caller.user).await?;
            if let Visibility::Team(team) = &visibility {
                team_id(db, org_id, team).await?;
            }
            let visibility = visibility.to_string();
            sqlx::query!("UPDATE orgs SET default_visibility = ? WHERE id = ?", visibility, org_id).execute(db).await?;
        }
        OrgChange::Links { org, on } => {
            let org_id = org_id(db, &org).await?;
            require_owner(db, org_id, caller.user).await?;
            // Turned off, every link of the org's threads made until now stops for good.
            let now = now_ms();
            sqlx::query!(
                "UPDATE orgs SET links = ?, links_off_ms = CASE WHEN ? THEN links_off_ms ELSE ? END WHERE id = ?",
                on,
                on,
                now,
                org_id
            )
            .execute(db)
            .await?;
        }
        OrgChange::Policy { org, policy } => {
            let org_id = org_id(db, &org).await?;
            require_owner(db, org_id, caller.user).await?;
            let policy = serde_json::to_string(&policy).map_err(|error| bad_request(error.to_string()))?;
            sqlx::query!("UPDATE orgs SET policy = ? WHERE id = ?", policy, org_id).execute(db).await?;
        }
        OrgChange::MoveProject { repo, .. } | OrgChange::ProjectSettings { repo, .. } if silt_types::is_chat(&repo) => {
            return Err(bad_request("chats belong to no org and have no settings of a project"));
        }
        OrgChange::MoveProject { repo, org } => {
            let target = org_id(db, &org).await?;
            require_owner(db, target, caller.user).await?;
            let current =
                sqlx::query_scalar!("SELECT org_id FROM projects WHERE repo = ?", repo).fetch_optional(db).await?.flatten();
            if let Some(current) = current.filter(|current| *current != target) {
                require_owner(db, current, caller.user).await?;
            }
            let mut tx = crate::write_transaction(db).await?;
            sqlx::query!(
                "INSERT INTO projects (repo, org_id) VALUES (?, ?) ON CONFLICT (repo) DO UPDATE SET org_id = excluded.org_id",
                repo,
                target,
            )
            .execute(&mut *tx)
            .await?;
            if current != Some(target) {
                // Team names belong to an org; the new org's team of the same
                // name are different people.
                sqlx::query!("UPDATE projects SET visibility = 'private' WHERE repo = ? AND visibility LIKE 'team:%'", repo)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query!("UPDATE threads SET visibility = 'private' WHERE repo = ? AND visibility LIKE 'team:%'", repo)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
        }
        OrgChange::ProjectSettings { repo, name, default_agent, default_preset, git_notes, forge_comments } => {
            let change = crate::projects::SettingsChange { name, default_agent, default_preset, git_notes, forge_comments };
            crate::projects::change_settings(db, caller.user, &repo, change).await?;
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Fails unless `org` still has an owner. It runs after the change in the
/// same transaction, which then holds the write lock, so two concurrent
/// changes can't both remove an owner.
async fn keep_an_owner(tx: &mut SqliteConnection, org: i64) -> Result<(), ApiError> {
    let role = OrgRole::Owner.as_str();
    let owners = sqlx::query_scalar!("SELECT COUNT(*) FROM org_members WHERE org_id = ? AND role = ?", org, role)
        .fetch_one(&mut *tx)
        .await?;
    if owners == 0 {
        return Err(bad_request("an org keeps at least one owner"));
    }
    Ok(())
}

/// Makes what a removed team could see private, so a later team of the same
/// name doesn't inherit it.
async fn forget_team_visibility(tx: &mut SqliteConnection, org: i64, team: &str) -> Result<(), sqlx::Error> {
    sqlx::query!("UPDATE orgs SET default_visibility = 'private' WHERE id = ? AND default_visibility = ?", org, team)
        .execute(&mut *tx)
        .await?;
    sqlx::query!("UPDATE projects SET visibility = 'private' WHERE org_id = ? AND visibility = ?", org, team)
        .execute(&mut *tx)
        .await?;
    sqlx::query!(
        "UPDATE threads SET visibility = 'private' WHERE visibility = ? AND repo IN (SELECT repo FROM projects WHERE org_id = ?)",
        team,
        org,
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(crate) async fn is_placed(db: &SqlitePool, repo: &str) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query_scalar!("SELECT 1 FROM projects WHERE repo = ? AND org_id IS NOT NULL", repo)
        .fetch_optional(db)
        .await?
        .is_some())
}

/// Whether placing a project in `user`'s first org would let them decide
/// its visibility: they own that org, or administer the server.
pub(crate) async fn may_place_project(db: &SqlitePool, user: UserId) -> Result<bool, ApiError> {
    let first = sqlx::query_scalar!("SELECT org_id FROM org_members WHERE user_id = ? ORDER BY rowid LIMIT 1", user)
        .fetch_optional(db)
        .await?;
    match first {
        Some(org) => owns(db, org, user).await,
        None => is_admin(db, user).await,
    }
}

/// Whether `user` may set the visibility of every thread of `repo`: an
/// owner of its org or a server admin.
pub(crate) async fn may_set_project(db: &SqlitePool, repo: &str, user: UserId) -> Result<bool, ApiError> {
    let org = sqlx::query_scalar!("SELECT org_id FROM projects WHERE repo = ?", repo).fetch_optional(db).await?.flatten();
    match org {
        Some(org) => owns(db, org, user).await,
        None => is_admin(db, user).await,
    }
}
