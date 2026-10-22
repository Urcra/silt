//! The web app, hosted at `/app/` for people without a daemon at hand
//! (ADR 0026): the same bundle a daemon serves, signed in with the session
//! cookie, and its browser API under `/app/api/`.

use axum::body::Body;
use axum::extract::{Path as UrlPath, Request, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use serde_json::json;

use silt_proto::{Capabilities, ErrorReply};

use crate::{ApiError, Server, UserId, pages};

/// Longest error answer rewritten for the app.
const ERROR_BODY: usize = 64 * 1024;

#[derive(rust_embed::Embed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
struct Assets;

/// The person a browser request is from, by their session cookie, and what
/// that session may do: everything, unless it's a paired phone's.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Browser {
    pub(crate) user: UserId,
    pub(crate) capabilities: Capabilities,
}

impl Browser {
    /// Refuses what this session may not do (ADR 0028).
    pub(crate) fn may(&self, allowed: bool, what: &str) -> Result<(), ApiError> {
        if allowed {
            Ok(())
        } else {
            Err(crate::forbidden(format!("this phone may not {what}; change that on the Devices page")))
        }
    }
}

pub(crate) fn router(server: &Server) -> Router<Server> {
    let api = Router::new()
        .route("/app/api/status", get(status))
        .merge(crate::browse::routes())
        .merge(crate::relayed::routes())
        .merge(crate::devices::browser_routes())
        .merge(crate::writes::routes())
        .merge(crate::push::browser_routes())
        .merge(crate::pairing::browser_routes())
        .merge(crate::links::browser_routes())
        // Not the page: the app would read it as an empty answer.
        .route(
            "/app/api/{*rest}",
            axum::routing::any(|| async { refused(StatusCode::NOT_FOUND, "the server doesn't answer this yet") }),
        )
        .layer(middleware::from_fn(as_the_app_reads))
        .layer(middleware::from_fn_with_state(server.clone(), browser_session));
    Router::new()
        .route("/app", get(|| async { Redirect::permanent("/app/") }))
        .route("/app/", get(page))
        .route("/app/{*path}", get(page))
        .merge(api)
}

/// The page or one of its assets; someone not signed in signs in first.
async fn page(State(server): State<Server>, headers: HeaderMap, uri: Uri) -> Response {
    if pages::signed_in(&server, &headers).await.is_none() {
        let next = pages::safe_next(Some(uri.path()));
        return Redirect::to(&format!("/login?next={}", crate::sign_in::encode(&next))).into_response();
    }
    let path = uri.path().trim_start_matches("/app/");
    if let Some(file) = Assets::get(path).filter(|_| !path.is_empty()) {
        let mime = file.metadata.mimetype().to_owned();
        return ([(header::CONTENT_TYPE, mime)], Body::from(file.data.into_owned())).into_response();
    }
    match Assets::get("index.html") {
        Some(file) => {
            let html = hosted_index(&String::from_utf8_lossy(&file.data));
            ([(header::CONTENT_TYPE, "text/html; charset=utf-8".to_owned())], html).into_response()
        }
        None => (StatusCode::NOT_FOUND, "the web UI was not built; run `pnpm -C web build` and rebuild silt").into_response(),
    }
}

/// A font of the app's, for the server's own pages, which anyone may load
/// before signing in. `name` is the file's name without the hash the build
/// gives it, as `ibm-plex-sans-latin-400-normal.woff2`.
pub(crate) async fn font(UrlPath(name): UrlPath<String>) -> Response {
    let Some(stem) = name.strip_suffix(".woff2").filter(|stem| stem.starts_with("ibm-plex-")) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let prefix = format!("assets/{stem}-");
    let found = Assets::iter()
        .find(|path| path.strip_prefix(&prefix).is_some_and(|hash| hash.ends_with(".woff2")))
        .and_then(|path| Assets::get(&path));
    match found {
        Some(file) => (
            [(header::CONTENT_TYPE, "font/woff2"), (header::CACHE_CONTROL, "public, max-age=86400")],
            Body::from(file.data.into_owned()),
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The app's page as the server hosts it: under `/app/`, saying the server
/// serves it.
fn hosted_index(html: &str) -> String {
    html.replace(r#"<base href="/" />"#, r#"<base href="/app/" />"#)
        .replace(r#"<base href="/">"#, r#"<base href="/app/">"#)
        .replace(r#"name="silt-served-by" content="daemon""#, r#"name="silt-served-by" content="server""#)
}

fn refused(status: StatusCode, error: &str) -> Response {
    (status, Json(json!({ "error": error }))).into_response()
}

/// Errors as the app reads them, `{"error": message}`, where the handlers
/// it shares with devices answer the device API's way.
async fn as_the_app_reads(request: Request, next: Next) -> Response {
    let response = next.run(request).await;
    if response.status().is_success() {
        return response;
    }
    let (parts, body) = response.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, ERROR_BODY).await else {
        return refused(parts.status, "the answer was too long");
    };
    match serde_json::from_slice::<ErrorReply>(&bytes) {
        Ok(ErrorReply::Invalid { message } | ErrorReply::Forbidden { message }) => refused(parts.status, &message),
        Ok(ErrorReply::NotFound) => refused(parts.status, "not found"),
        Ok(ErrorReply::Unauthorized) => refused(parts.status, "sign in at /login"),
        Ok(_) => refused(parts.status, "the server refused this"),
        Err(_) => Response::from_parts(parts, Body::from(bytes)),
    }
}

/// Lets through requests with a session, and changes only with the header
/// other sites can't send and from this server's own pages (ADR 0026).
async fn browser_session(State(server): State<Server>, mut request: Request, next: Next) -> Response {
    let Some(user) = pages::signed_in(&server, request.headers()).await else {
        return refused(StatusCode::UNAUTHORIZED, "sign in at /login");
    };
    let reads = matches!(*request.method(), Method::GET | Method::HEAD);
    let marked = request.headers().get("x-silt-request").is_some_and(|value| value == "1");
    if !reads && !(marked && pages::same_origin(&server, request.headers())) {
        return refused(StatusCode::FORBIDDEN, "this request didn't come from Silt's own pages");
    }
    let token = pages::cookie_value(request.headers(), pages::SESSION_COOKIE).unwrap_or_default();
    let capabilities = match crate::accounts::session_capabilities(server.db(), &token).await {
        Ok(capabilities) => capabilities,
        Err(_) => return refused(StatusCode::INTERNAL_SERVER_ERROR, "could not read the session"),
    };
    request.extensions_mut().insert(Browser { user: user.id, capabilities });
    next.run(request).await
}

/// What the app asks first: where it is, and that no daemon serves it.
async fn status(State(server): State<Server>, Extension(browser): Extension<Browser>) -> Response {
    let threads = match server.viewer(browser.user).await {
        Ok(viewer) => viewer.visible_threads().count(),
        Err(error) => return refused(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    };
    Json(json!({
        "server": server.config().public_url(),
        "device": null,
        "sync_error": null,
        "threads": threads,
        "diverged": [],
        "recovered": [],
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hosted_page_is_based_at_app_and_says_the_server_serves_it() {
        let built =
            r#"<head><base href="/" /><meta name="silt-served-by" content="daemon" /><script src="./assets/a.js"></script>"#;
        let hosted = hosted_index(built);
        assert!(hosted.contains(r#"<base href="/app/" />"#), "{hosted}");
        assert!(hosted.contains(r#"content="server""#), "{hosted}");
        assert!(hosted.contains(r#"src="./assets/a.js""#));
    }

    #[tokio::test]
    async fn only_the_apps_fonts_are_served_before_signing_in() {
        for name in [
            "index.html",
            "assets/index.js",
            "ibm-plex-sans-latin-400-normal.woff",
            "../ibm-plex-sans-latin-400-normal.woff2",
            "ibm-plex-nonesuch.woff2",
        ] {
            let response = font(UrlPath(name.to_owned())).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{name}");
        }
    }
}
