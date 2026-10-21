//! Pairing a phone (ADR 0028): a signed-in device or browser asks for a
//! one-time code, shown as a QR code. Opening it on a phone and confirming
//! signs that phone's browser in to the hosted app, as a session with a
//! phone's capabilities.

use axum::extract::{Extension, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use silt_proto::Capabilities;
use silt_proto::view::PhonePairing;

use crate::app::Browser;
use crate::pages::{error_page, escape, page, same_origin, with_session};
use crate::{ApiError, Caller, Server, UserId, hash, now_ms};

const CODE_TTL_MS: i64 = 10 * 60 * 1000;
/// Easy to read off a screen and type: no 0, O, 1 or I.
const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const CODE_LEN: usize = 8;
/// Where a paired phone starts.
const FIRST_SCREEN: &str = "/app/needs-you";

pub(crate) fn device_routes() -> Router<Server> {
    Router::new().route("/v1/pair", post(device_pair))
}

pub(crate) fn browser_routes() -> Router<Server> {
    Router::new().route("/app/api/pair", post(browser_pair))
}

/// The pages a phone opens, before it's signed in.
pub(crate) fn page_routes() -> Router<Server> {
    Router::new().route("/pair", get(enter)).route("/pair/{code}", get(confirm).post(pair))
}

fn new_code() -> String {
    uuid::Uuid::new_v4()
        .as_bytes()
        .iter()
        .take(CODE_LEN)
        .map(|byte| char::from(ALPHABET[usize::from(*byte) % ALPHABET.len()]))
        .collect()
}

fn normalized(code: &str) -> String {
    code.chars().filter(char::is_ascii_alphanumeric).map(|char| char.to_ascii_uppercase()).collect()
}

async fn make(server: &Server, user: UserId) -> Result<PhonePairing, ApiError> {
    let code = new_code();
    let (code_hash, expires_ms) = (hash(&code), now_ms() + CODE_TTL_MS);
    sqlx::query!("INSERT INTO phone_pairings (code_hash, user_id, expires_ms) VALUES (?, ?, ?)", code_hash, user, expires_ms)
        .execute(server.db())
        .await?;
    let url = format!("{}/pair/{code}", server.config().public_url().trim_end_matches('/'));
    let qr_svg = qrcode::QrCode::new(url.as_bytes())
        .map_err(|_| crate::bad_request("the server's address is too long for a QR code"))?
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(220, 220)
        .build();
    Ok(PhonePairing { code, url, qr_svg, expires_ms })
}

async fn device_pair(State(server): State<Server>, Extension(caller): Extension<Caller>) -> Result<Json<PhonePairing>, ApiError> {
    Ok(Json(make(&server, caller.user).await?))
}

async fn browser_pair(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
) -> Result<Json<PhonePairing>, ApiError> {
    // A phone pairs no further phones: what it may do stays with its person.
    browser.may(browser.capabilities == Capabilities::ALL, "pair other phones")?;
    Ok(Json(make(&server, browser.user).await?))
}

#[derive(Deserialize)]
struct Entered {
    #[serde(default)]
    code: Option<String>,
}

/// Where a phone types the short code, for when the QR code can't be read.
async fn enter(Query(entered): Query<Entered>) -> Response {
    if let Some(code) = entered.code.map(|code| normalized(&code)).filter(|code| !code.is_empty()) {
        return Redirect::to(&format!("/pair/{code}")).into_response();
    }
    page(
        "Pair this phone",
        r#"<p>Type the code your Devices page shows.</p>
<form method="get" action="/pair"><label>Code <input name="code" autocomplete="off" autocapitalize="characters" required></label>
<button type="submit">Continue</button></form>"#,
    )
    .into_response()
}

/// The person the code is for, if it still works.
async fn code_user(server: &Server, code: &str) -> Result<Option<String>, sqlx::Error> {
    let (code_hash, now) = (hash(&normalized(code)), now_ms());
    sqlx::query_scalar!(
        "SELECT u.name FROM phone_pairings p JOIN users u ON u.id = p.user_id WHERE p.code_hash = ? AND p.expires_ms > ?",
        code_hash,
        now
    )
    .fetch_optional(server.db())
    .await
}

fn used_up() -> Response {
    error_page(StatusCode::NOT_FOUND, "This pairing code is used up or expired. Make a new one on the Devices page.")
}

/// Asks before signing in, so a link someone else sent can't sign this
/// phone in to their account unawares.
async fn confirm(State(server): State<Server>, Path(code): Path<String>) -> Response {
    match code_user(&server, &code).await {
        Ok(Some(name)) => page(
            "Pair this phone",
            &format!(
                r#"<p>Sign this phone in to Silt as <strong>{}</strong>? It will answer your agents' permission requests and review merge requests, and you can give it more from the Devices page.</p>
<form method="post"><button type="submit">Sign in</button></form>"#,
                escape(&name)
            ),
        )
        .into_response(),
        Ok(None) => used_up(),
        Err(_) => error_page(StatusCode::INTERNAL_SERVER_ERROR, "Could not read the pairing code."),
    }
}

async fn pair(State(server): State<Server>, headers: HeaderMap, Path(code): Path<String>) -> Response {
    if !same_origin(&server, &headers) {
        return error_page(StatusCode::FORBIDDEN, "This form was sent from another site.");
    }
    let (code_hash, now) = (hash(&normalized(&code)), now_ms());
    let user = sqlx::query_scalar!(
        r#"DELETE FROM phone_pairings WHERE code_hash = ? AND expires_ms > ? RETURNING user_id AS "user_id: UserId""#,
        code_hash,
        now
    )
    .fetch_optional(server.db())
    .await;
    let user = match user {
        Ok(Some(user)) => user,
        Ok(None) => return used_up(),
        Err(_) => return error_page(StatusCode::INTERNAL_SERVER_ERROR, "Could not read the pairing code."),
    };
    match crate::accounts::create_paired_session(server.db(), user, "Phone", Capabilities::PHONE).await {
        Ok(token) => with_session(&server, &token, FIRST_SCREEN),
        Err(_) => error_page(StatusCode::INTERNAL_SERVER_ERROR, "Could not sign this phone in."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_easy_to_read_and_type() {
        let code = new_code();
        assert_eq!(code.len(), CODE_LEN);
        assert!(code.bytes().all(|byte| ALPHABET.contains(&byte)), "{code}");
        assert_eq!(normalized(" ab-cd 23 "), "ABCD23");
    }
}
