//! The person's devices (ADR 0026): which are online, what runs on each,
//! and what each may do to the others, for a device and for the hosted app.

use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use silt_proto::Capabilities;
use silt_proto::view::{DeviceAgent, DeviceChange, DeviceView};
use silt_types::DeviceId;
use sqlx::SqlitePool;

use crate::app::Browser;
use crate::{ApiError, Caller, Server, UserId, bad_request, id_bytes, not_found};

const MAX_NAME: usize = 64;

pub(crate) fn routes() -> Router<Server> {
    Router::new()
        .route("/v1/devices", get(device_list))
        .route("/v1/devices/{id}", patch(device_change))
        .route("/v1/devices/{id}/revoke", post(device_revoke))
}

pub(crate) fn browser_routes() -> Router<Server> {
    Router::new()
        .route("/app/api/devices", get(browser_list))
        .route("/app/api/devices/{id}", patch(browser_change))
        .route("/app/api/devices/{id}/revoke", post(browser_revoke))
}

/// What `device` may do: everything unless its person chose otherwise, and
/// nothing once it's signed out.
pub(crate) async fn capabilities_of(db: &SqlitePool, device: DeviceId) -> Result<Capabilities, sqlx::Error> {
    let id = id_bytes(device.0);
    let row = sqlx::query!("SELECT capabilities FROM devices WHERE id = ?", id).fetch_optional(db).await?;
    Ok(match row {
        None => Capabilities::default(),
        Some(row) => row.capabilities.map_or(Capabilities::ALL, |json| serde_json::from_str(&json).unwrap_or_default()),
    })
}

async fn list(server: &Server, user: UserId, this: Option<DeviceId>) -> Result<Vec<DeviceView>, ApiError> {
    let titles = crate::browse::titles(server, user).await?;
    let mut views = Vec::new();
    for device in crate::accounts::devices_of(server.db(), user).await? {
        let (online, report) = server.shared.control.heard(device.id).await;
        views.push(DeviceView {
            id: device.id.to_string(),
            kind: "device".into(),
            name: device.name,
            created_ms: device.created_ms,
            last_used_ms: device.last_used_ms,
            online,
            this: this == Some(device.id),
            capabilities: capabilities_of(server.db(), device.id).await?,
            agents: report
                .agents
                .into_iter()
                .filter_map(|agent| {
                    let title = titles.get(&agent.thread)?.clone();
                    Some(DeviceAgent {
                        thread: agent.thread.to_string(),
                        title,
                        agent: agent.agent,
                        agent_id: agent.agent_id,
                        state: agent.state,
                    })
                })
                .collect(),
            startable: report.startable,
        });
    }
    for phone in crate::accounts::paired_sessions(server.db(), user).await? {
        views.push(DeviceView {
            id: format!("{PHONE}{}", phone.id),
            kind: "phone".into(),
            name: phone.name,
            created_ms: phone.created_ms,
            last_used_ms: phone.created_ms,
            online: false,
            this: false,
            capabilities: phone.capabilities,
            agents: Vec::new(),
            startable: Vec::new(),
        });
    }
    Ok(views)
}

/// How a paired phone's session is named among devices.
const PHONE: &str = "phone-";

fn phone_session(id: &str) -> Option<i64> {
    id.strip_prefix(PHONE)?.parse().ok()
}

fn valid_name(name: &str) -> Result<&str, ApiError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME {
        return Err(bad_request(format!("a device's name is 1 to {MAX_NAME} characters")));
    }
    Ok(name)
}

/// The person's device `id` names, by its full id or the end of it, and its
/// name.
pub(crate) async fn own_device(server: &Server, user: UserId, id: &str) -> Result<(DeviceId, String), ApiError> {
    let mut found: Vec<(DeviceId, String)> = crate::accounts::devices_of(server.db(), user)
        .await?
        .into_iter()
        .filter(|device| !id.is_empty() && device.id.to_string().ends_with(id))
        .map(|device| (device.id, device.name))
        .collect();
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(not_found()),
        _ => Err(bad_request("that names several devices")),
    }
}

async fn change(server: &Server, user: UserId, id: &str, change: DeviceChange) -> Result<DeviceView, ApiError> {
    let name = change.name.as_deref().map(valid_name).transpose()?;
    if let Some(session) = phone_session(id) {
        let found = crate::accounts::change_paired_session(server.db(), user, session, name, change.capabilities, false).await?;
        if !found {
            return Err(not_found());
        }
        let views = list(server, user, None).await?;
        return views.into_iter().find(|view| view.id == id).ok_or_else(not_found);
    }
    let (device, _) = own_device(server, user, id).await?;
    let key = id_bytes(device.0);
    if let Some(name) = name {
        sqlx::query!("UPDATE devices SET name = ? WHERE id = ?", name, key).execute(server.db()).await?;
    }
    if let Some(capabilities) = change.capabilities {
        let json = serde_json::to_string(&capabilities).map_err(crate::ServerError::from)?;
        sqlx::query!("UPDATE devices SET capabilities = ? WHERE id = ?", json, key).execute(server.db()).await?;
    }
    let views = list(server, user, None).await?;
    views.into_iter().find(|view| view.id == device.to_string()).ok_or_else(not_found)
}

async fn revoke(server: &Server, user: UserId, id: &str) -> Result<StatusCode, ApiError> {
    if let Some(session) = phone_session(id) {
        let ended = crate::accounts::change_paired_session(server.db(), user, session, None, None, true).await?;
        return if ended { Ok(StatusCode::NO_CONTENT) } else { Err(not_found()) };
    }
    let (device, _) = own_device(server, user, id).await?;
    if !crate::accounts::revoke_device(server.db(), user, device).await? {
        return Err(not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn device_list(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<Vec<DeviceView>>, ApiError> {
    Ok(Json(list(&server, caller.user, Some(caller.device)).await?))
}

async fn device_change(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
    Json(body): Json<DeviceChange>,
) -> Result<Json<DeviceView>, ApiError> {
    let mut view = change(&server, caller.user, &id, body).await?;
    view.this = view.id == caller.device.to_string();
    Ok(Json(view))
}

async fn device_revoke(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    revoke(&server, caller.user, &id).await
}

async fn browser_list(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
) -> Result<Json<Vec<DeviceView>>, ApiError> {
    Ok(Json(list(&server, browser.user, None).await?))
}

async fn browser_change(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
    Json(body): Json<DeviceChange>,
) -> Result<Json<DeviceView>, ApiError> {
    // Else a phone could give itself more than its person did.
    browser.may(browser.capabilities == Capabilities::ALL, "change devices")?;
    Ok(Json(change(&server, browser.user, &id, body).await?))
}

async fn browser_revoke(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    browser.may(browser.capabilities == Capabilities::ALL, "sign devices out")?;
    revoke(&server, browser.user, &id).await
}
