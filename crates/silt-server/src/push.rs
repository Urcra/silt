//! Web Push (ADR 0028): the server's VAPID keys, each person's push
//! subscriptions, and a push for every notification whose kind notifies
//! them, encrypted for the browser as RFC 8291 says.

use std::path::Path;

use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use silt_types::ThreadId;
use web_push_native::jwt_simple::algorithms::ES256KeyPair;
use web_push_native::jwt_simple::prelude::ECDSAP256PublicKeyLike as _;
use web_push_native::p256::PublicKey;
use web_push_native::{Auth, WebPushBuilder};

use crate::app::Browser;
use crate::{ApiError, Server, UserId, bad_request, now_ms};

pub(crate) const KEY_FILE: &str = "vapid.key";
/// How long a push service keeps a push for a browser that's away.
const PUSH_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// The server's key for signing pushes, made the first time it's needed.
pub(crate) fn load_or_make_key(data_dir: &Path) -> std::io::Result<ES256KeyPair> {
    let path = data_dir.join(KEY_FILE);
    if let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(bytes) = URL_SAFE_NO_PAD.decode(text.trim())
        && let Ok(key) = ES256KeyPair::from_bytes(&bytes)
    {
        return Ok(key);
    }
    let key = ES256KeyPair::generate();
    write_private(&path, &URL_SAFE_NO_PAD.encode(key.to_bytes()))?;
    Ok(key)
}

fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    silt_os::write_private(path, text.as_bytes())
}

pub(crate) fn browser_routes() -> Router<Server> {
    Router::new()
        .route("/app/api/push/key", get(public_key))
        .route("/app/api/push/subscriptions", post(subscribe).delete(unsubscribe))
}

/// The key browsers subscribe with, as `applicationServerKey` takes it.
async fn public_key(State(server): State<Server>) -> Json<String> {
    Json(URL_SAFE_NO_PAD.encode(server.shared.vapid.public_key().public_key().to_bytes_uncompressed()))
}

/// A browser's `PushSubscription`, as its `toJSON()` gives it.
#[derive(Deserialize)]
struct Subscription {
    endpoint: String,
    keys: SubscriptionKeys,
}

#[derive(Deserialize)]
struct SubscriptionKeys {
    p256dh: String,
    auth: String,
}

async fn subscribe(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Json(subscription): Json<Subscription>,
) -> Result<StatusCode, ApiError> {
    // A server on plain HTTP is a developer's, whose push service may be too.
    if !subscription.endpoint.starts_with("https://") && !server.config().public_url().starts_with("http://") {
        return Err(bad_request("a push service is reached over HTTPS"));
    }
    if target(&subscription.endpoint, &subscription.keys.p256dh, &subscription.keys.auth).is_none() {
        return Err(bad_request("that isn't a push subscription"));
    }
    let now = now_ms();
    sqlx::query!(
        "INSERT INTO push_subscriptions (endpoint, user_id, p256dh, auth, created_ms) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT (endpoint) DO UPDATE SET user_id = excluded.user_id, p256dh = excluded.p256dh, auth = excluded.auth",
        subscription.endpoint,
        browser.user,
        subscription.keys.p256dh,
        subscription.keys.auth,
        now,
    )
    .execute(server.db())
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Unsubscribe {
    endpoint: String,
}

async fn unsubscribe(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    Json(body): Json<Unsubscribe>,
) -> Result<StatusCode, ApiError> {
    sqlx::query!("DELETE FROM push_subscriptions WHERE endpoint = ? AND user_id = ?", body.endpoint, browser.user)
        .execute(server.db())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Where and how to reach one browser, from what it subscribed with.
fn target(endpoint: &str, p256dh: &str, auth: &str) -> Option<WebPushBuilder> {
    let public = PublicKey::from_sec1_bytes(&URL_SAFE_NO_PAD.decode(p256dh).ok()?).ok()?;
    let auth: [u8; 16] = URL_SAFE_NO_PAD.decode(auth).ok()?.try_into().ok()?;
    Some(WebPushBuilder::new(endpoint.parse().ok()?, public, Auth::from(auth)).with_valid_duration(PUSH_TTL))
}

/// What a push carries, for the service worker to show.
#[derive(Serialize)]
struct Payload<'a> {
    title: &'a str,
    body: &'a str,
    url: String,
    tag: String,
}

/// Pushes a notification to each of `user`'s browsers, in the background.
pub(crate) fn push_soon(server: &Server, user: UserId, title: &str, body: &str, thread: ThreadId) {
    let payload = Payload { title, body, url: format!("t/{thread}"), tag: format!("silt-{thread}") };
    let Ok(payload) = serde_json::to_vec(&payload) else {
        return;
    };
    let server = server.clone();
    tokio::spawn(async move {
        if let Err(error) = push(&server, user, payload).await {
            tracing::warn!(%error, "could not push a notification");
        }
    });
}

async fn push(server: &Server, user: UserId, payload: Vec<u8>) -> Result<(), sqlx::Error> {
    let subscriptions = sqlx::query!("SELECT endpoint, p256dh, auth FROM push_subscriptions WHERE user_id = ?", user)
        .fetch_all(server.db())
        .await?;
    let contact = server.config().public_url();
    for subscription in subscriptions {
        let Some(target) = target(&subscription.endpoint, &subscription.p256dh, &subscription.auth) else {
            continue;
        };
        let request = match target.with_vapid(&server.shared.vapid, &contact).build(payload.clone()) {
            Ok(request) => request,
            Err(error) => {
                tracing::warn!(%error, "could not encrypt a push");
                continue;
            }
        };
        let (parts, body) = request.into_parts();
        let mut sent = server.http().post(parts.uri.to_string()).body(body);
        for (name, value) in &parts.headers {
            sent = sent.header(name.as_str(), value.as_bytes());
        }
        match sent.send().await {
            // The push service says the browser unsubscribed or went away.
            Ok(response) if matches!(response.status().as_u16(), 404 | 410) => {
                sqlx::query!("DELETE FROM push_subscriptions WHERE endpoint = ?", subscription.endpoint)
                    .execute(server.db())
                    .await?;
            }
            Ok(response) if !response.status().is_success() => {
                tracing::warn!(status = %response.status(), "a push service refused a push");
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "could not reach a push service"),
        }
    }
    Ok(())
}
