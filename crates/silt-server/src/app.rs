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
        .route("/app/api/artifact-settings", get(artifact_settings))
        .route("/app/api/themes", get(themes))
        .route("/app/api/themes/detail", get(theme_detail))
        .route("/app/api/themes/css", get(theme_css))
        // A synced theme's fonts, which its CSS loads relative to the page (ADR 0099).
        .route("/app/theme-fonts/{id}/{file}", get(theme_font))
        .merge(crate::browse::routes())
        .merge(crate::relayed::routes())
        .merge(crate::rooms::routes())
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
    // The bundle's scripts and styles are the same for everyone, and a
    // read-only link's page loads them before anyone signs in (ADR 0073).
    let asset = uri.path().trim_start_matches("/app/");
    if let Some(file) = asset.strip_prefix("assets/").and_then(|_| silt_web::Assets::get(asset)) {
        let mime = file.metadata.mimetype().to_owned();
        let cache = silt_web::cache_control(asset).to_owned();
        return ([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, cache)], Body::from(file.data.into_owned()))
            .into_response();
    }
    if pages::signed_in(&server, &headers).await.is_none() {
        let next = pages::safe_next(Some(uri.path()));
        return Redirect::to(&format!("/login?next={}", crate::sign_in::encode(&next))).into_response();
    }
    let path = uri.path().trim_start_matches("/app/");
    if let Some(file) = silt_web::Assets::get(path).filter(|_| !path.is_empty()) {
        let mime = file.metadata.mimetype().to_owned();
        return ([(header::CONTENT_TYPE, mime)], Body::from(file.data.into_owned())).into_response();
    }
    match silt_web::Assets::get("index.html") {
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
    let found = silt_web::Assets::iter()
        .find(|path| path.strip_prefix(&prefix).is_some_and(|hash| hash.ends_with(".woff2")))
        .and_then(|path| silt_web::Assets::get(&path));
    match found {
        Some(file) => (
            [(header::CONTENT_TYPE, "font/woff2"), (header::CACHE_CONTROL, "public, max-age=86400")],
            Body::from(file.data.into_owned()),
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The app's page for a read-only link to one artifact, which shows it
/// with the web UI's own viewers and nothing else (ADR 0073); `None` when
/// the web UI wasn't built.
pub(crate) fn link_index() -> Option<String> {
    let file = silt_web::Assets::get("index.html")?;
    let html = hosted_index(&String::from_utf8_lossy(&file.data));
    Some(html.replacen(
        "<head>",
        r#"<head><meta name="silt-link" content="artifact" /><meta name="robots" content="noindex" />"#,
        1,
    ))
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
/// How the app's artifact frames load libraries and fonts (ADR 0071).
async fn artifact_settings(State(server): State<Server>) -> Response {
    Json(json!({ "proxy": server.config().artifact_proxy })).into_response()
}

/// Silt's own themes with the person's synced over them (ADR 0098,
/// ADR 0099); the pick stays in the browser, as the hosted app has no
/// machine to keep it on.
async fn theme_entries(server: &Server, browser: &Browser) -> Vec<silt_proto::theme::ThemeEntry> {
    let copies = crate::me::stored_themes(server, browser.user).await.unwrap_or_default();
    silt_proto::theme::merge(silt_proto::theme::bundled_entries(), silt_proto::theme::copy_entries(&copies))
}

async fn themes(State(server): State<Server>, Extension(browser): Extension<Browser>) -> Response {
    let themes = theme_entries(&server, &browser).await.iter().map(silt_proto::theme::ThemeEntry::summary).collect();
    Json(silt_proto::view::ThemeCatalog {
        themes,
        problems: Vec::new(),
        folder: None,
        appearance: silt_proto::view::AppearanceSettings::default(),
    })
    .into_response()
}

async fn theme_font(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    axum::extract::Path((id, file)): axum::extract::Path<(String, String)>,
) -> Response {
    let copies = crate::me::stored_themes(&server, browser.user).await.unwrap_or_default();
    match silt_proto::theme::synced_font(&copies, &id, &file) {
        Some((bytes, media_type)) => {
            ([(header::CONTENT_TYPE, media_type), (header::CACHE_CONTROL, "private, no-cache")], Body::from(bytes))
                .into_response()
        }
        None => refused(StatusCode::NOT_FOUND, "no such font"),
    }
}

#[derive(Debug, serde::Deserialize)]
struct ThemeQuery {
    #[serde(default)]
    id: String,
    #[serde(default)]
    light: Option<String>,
    #[serde(default)]
    dark: Option<String>,
}

async fn theme_detail(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    axum::extract::Query(query): axum::extract::Query<ThemeQuery>,
) -> Response {
    match theme_entries(&server, &browser).await.iter().find(|entry| entry.theme.id.as_str() == query.id) {
        Some(entry) => Json(entry.detail()).into_response(),
        None => refused(StatusCode::NOT_FOUND, "no such theme"),
    }
}

async fn theme_css(
    State(server): State<Server>,
    Extension(browser): Extension<Browser>,
    axum::extract::Query(query): axum::extract::Query<ThemeQuery>,
) -> Response {
    let defaults = silt_proto::view::AppearanceSettings::default();
    let settings = silt_proto::view::AppearanceSettings {
        light: query.light.unwrap_or(defaults.light),
        dark: query.dark.unwrap_or(defaults.dark),
        ..defaults
    };
    let css = silt_proto::theme::css(&theme_entries(&server, &browser).await, &settings);
    Json(json!({ "css": css, "mode": "system" })).into_response()
}

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
