//! The server's few pages (ADR 0013): signing in with a password or a
//! provider, signing up with an invite, and approving a device. Plain
//! server-rendered HTML; the full web app is the daemon's.

use std::collections::HashMap;

use axum::Form;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::Server;
use crate::accounts::{self, NewUser, SignInState, User};
use crate::config::SignupPolicy;
use crate::sign_in;

const SESSION_COOKIE: &str = "silt_session";

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn page(title: &str, body: &str) -> Html<String> {
    Html(format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} · Silt</title>
<style>
body {{ font: 16px/1.5 system-ui, sans-serif; max-width: 26rem; margin: 3rem auto; padding: 0 1rem; color: #1f2328; background: #fff; }}
h1 {{ font-size: 1.4rem; }} label {{ display: block; margin-top: .8rem; }}
input {{ width: 100%; box-sizing: border-box; padding: .45rem; border: 1px solid #c9cdd2; border-radius: 6px; font: inherit; }}
button, .button {{ display: inline-block; margin-top: 1rem; padding: .5rem 1rem; border: 1px solid #1f6feb; border-radius: 6px; background: #1f6feb; color: #fff; font: inherit; text-decoration: none; cursor: pointer; }}
.secondary {{ background: #fff; color: #1f6feb; }} .error {{ color: #b42318; }} code {{ font-size: 1.2rem; letter-spacing: .1em; }}
@media (prefers-color-scheme: dark) {{ body {{ background: #0d1117; color: #e6edf3; }} input {{ background: #161b22; color: #e6edf3; border-color: #30363d; }} .secondary {{ background: #0d1117; }} }}
</style></head><body><h1>{title}</h1>{body}</body></html>"#,
        title = escape(title),
    ))
}

/// Only paths on this server, so sign-in can't redirect elsewhere. Browsers
/// drop tabs and line breaks from URLs, turning `/\t/evil.example` into
/// `//evil.example`, so no whitespace or control character passes.
fn safe_next(next: Option<&str>) -> String {
    match next {
        Some(next)
            if next.starts_with('/')
                && !next.starts_with("//")
                && !next.contains('\\')
                && !next.chars().any(|char| char.is_control() || char.is_whitespace()) =>
        {
            next.to_owned()
        }
        _ => "/".to_owned(),
    }
}

/// The cookie that ties a sign-in through a provider to the browser that
/// started it: without it, someone could send a victim the callback of a
/// sign-in they started themselves and sign the victim in as them.
const SIGN_IN_COOKIE: &str = "silt_sign_in";

fn secure_attribute(server: &Server) -> &'static str {
    if server.config().public_url().starts_with("https://") { "; Secure" } else { "" }
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_owned())
}

async fn signed_in(server: &Server, headers: &HeaderMap) -> Option<User> {
    let token = cookie_value(headers, SESSION_COOKIE)?;
    accounts::session_user(server.db(), &token).await.ok().flatten()
}

/// Rejects form posts sent from other sites, on top of the SameSite cookie.
fn same_origin(server: &Server, headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|value| value.to_str().ok()) else {
        return true;
    };
    let public = server.config().public_url();
    let host = headers.get(header::HOST).and_then(|value| value.to_str().ok()).unwrap_or_default();
    origin == public || origin.split_once("://").is_some_and(|(_, rest)| rest == host)
}

/// Starts a browser session for `user` and continues to `next`.
async fn start_session(server: &Server, user: i64, next: &str) -> Response {
    let token = match accounts::create_session(server.db(), user).await {
        Ok(token) => token,
        Err(_) => return error_page(StatusCode::INTERNAL_SERVER_ERROR, "Could not sign you in."),
    };
    let secure = if server.config().public_url().starts_with("https://") { "; Secure" } else { "" };
    let cookie = format!("{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=2592000{secure}");
    let mut response = Redirect::to(next).into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).expect("token is ASCII"));
    response
}

fn error_page(status: StatusCode, message: &str) -> Response {
    (status, page("Something went wrong", &format!("<p class=\"error\">{}</p><p><a href=\"/\">Back</a></p>", escape(message)))).into_response()
}

fn provider_buttons(server: &Server, next: &str, invite: Option<&str>) -> String {
    let config = server.config();
    let mut query = format!("next={}", sign_in::encode(next));
    if let Some(invite) = invite {
        query.push_str(&format!("&invite={}", sign_in::encode(invite)));
    }
    let mut buttons = String::new();
    if config.auth.github.is_some() {
        buttons.push_str(&format!("<p><a class=\"button secondary\" href=\"/auth/github/start?{query}\">Continue with GitHub</a></p>"));
    }
    if let Some(oidc) = &config.auth.oidc {
        buttons.push_str(&format!(
            "<p><a class=\"button secondary\" href=\"/auth/oidc/start?{query}\">Continue with {}</a></p>",
            escape(&oidc.name)
        ));
    }
    buttons
}

#[derive(Deserialize)]
pub(crate) struct NextQuery {
    next: Option<String>,
    error: Option<String>,
}

pub(crate) async fn home(State(server): State<Server>, headers: HeaderMap) -> Response {
    match signed_in(&server, &headers).await {
        Some(user) => page(
            "Silt",
            &format!(
                "<p>Signed in as <strong>{}</strong>.</p><p>Sign a machine in with <code>silt auth login --server {}</code>.</p><form method=\"post\" action=\"/logout\"><button class=\"secondary\">Sign out</button></form>",
                escape(&user.name),
                escape(&server.config().public_url())
            ),
        )
        .into_response(),
        None => Redirect::to("/login").into_response(),
    }
}

pub(crate) async fn login_page(State(server): State<Server>, Query(query): Query<NextQuery>) -> Response {
    let next = safe_next(query.next.as_deref());
    let mut body = String::new();
    if let Some(error) = &query.error {
        body.push_str(&format!("<p class=\"error\">{}</p>", escape(error)));
    }
    if server.config().auth.passwords {
        body.push_str(&format!(
            r#"<form method="post" action="/login">
<input type="hidden" name="next" value="{}">
<label>User <input name="user" autocomplete="username" required autofocus></label>
<label>Password <input name="password" type="password" autocomplete="current-password" required></label>
<button>Sign in</button></form>"#,
            escape(&next)
        ));
    }
    body.push_str(&provider_buttons(&server, &next, None));
    page("Sign in to Silt", &body).into_response()
}

#[derive(Deserialize)]
pub(crate) struct LoginForm {
    user: String,
    password: String,
    next: Option<String>,
}

pub(crate) async fn login(State(server): State<Server>, headers: HeaderMap, Form(form): Form<LoginForm>) -> Response {
    if !same_origin(&server, &headers) {
        return error_page(StatusCode::FORBIDDEN, "This form was sent from another site.");
    }
    let next = safe_next(form.next.as_deref());
    if !server.config().auth.passwords {
        return error_page(StatusCode::FORBIDDEN, "Passwords are turned off on this server.");
    }
    match accounts::check_password(server.db(), form.user.trim(), &form.password).await {
        Ok(Some(user)) => start_session(&server, user.id, &next).await,
        _ => Redirect::to(&format!(
            "/login?next={}&error={}",
            sign_in::encode(&next),
            sign_in::encode("That user name and password don't match.")
        ))
        .into_response(),
    }
}

pub(crate) async fn logout(State(server): State<Server>, headers: HeaderMap) -> Response {
    if let Some(token) = cookie_value(&headers, SESSION_COOKIE) {
        let _ = accounts::end_session(server.db(), &token).await;
    }
    let mut response = Redirect::to("/login").into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("silt_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0"),
    );
    response
}

#[derive(Deserialize)]
pub(crate) struct DeviceQuery {
    code: Option<String>,
}

pub(crate) async fn device_page(State(server): State<Server>, headers: HeaderMap, Query(query): Query<DeviceQuery>) -> Response {
    let code = query.code.unwrap_or_default();
    let here = format!("/device?code={}", sign_in::encode(&code));
    let Some(user) = signed_in(&server, &headers).await else {
        return Redirect::to(&format!("/login?next={}", sign_in::encode(&here))).into_response();
    };
    let code = accounts::normalize_user_code(&code);
    match accounts::device_request(server.db(), &code).await {
        Ok(Some(name)) => page(
            "Sign in a device",
            &format!(
                r#"<p>Sign in <strong>{name}</strong> as <strong>{user}</strong>?</p>
<p>Check that the device shows this code: <code>{code}</code></p>
<form method="post" action="/device"><input type="hidden" name="code" value="{code}">
<button name="action" value="approve">Approve</button> <button class="secondary" name="action" value="deny">Deny</button></form>"#,
                name = escape(&name),
                user = escape(&user.name),
                code = escape(&code)
            ),
        )
        .into_response(),
        _ => page(
            "Sign in a device",
            r#"<p class="error">This code is unknown or expired. Run <code>silt auth login</code> again.</p>"#,
        )
        .into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct DeviceForm {
    code: String,
    action: String,
}

pub(crate) async fn device(State(server): State<Server>, headers: HeaderMap, Form(form): Form<DeviceForm>) -> Response {
    if !same_origin(&server, &headers) {
        return error_page(StatusCode::FORBIDDEN, "This form was sent from another site.");
    }
    let Some(user) = signed_in(&server, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    if form.action != "approve" {
        let _ = accounts::deny_device(server.db(), &form.code).await;
        return page("Device denied", "<p>The device was not signed in.</p>").into_response();
    }
    match accounts::approve_device(server.db(), &form.code, user.id).await {
        Ok(true) => page("Device signed in", "<p>You can close this page; the device is signed in.</p>").into_response(),
        _ => error_page(StatusCode::BAD_REQUEST, "This code is unknown or expired."),
    }
}

#[derive(Deserialize)]
pub(crate) struct SignupQuery {
    invite: Option<String>,
    next: Option<String>,
    error: Option<String>,
}

async fn may_sign_up(server: &Server, invite: Option<&str>) -> bool {
    match invite {
        Some(invite) => accounts::invite(server.db(), invite).await.ok().flatten().is_some(),
        None => server.config().signup.policy == SignupPolicy::Open,
    }
}

pub(crate) async fn signup_page(State(server): State<Server>, Query(query): Query<SignupQuery>) -> Response {
    let next = safe_next(query.next.as_deref());
    let invite = query.invite.as_deref();
    let can_sign_up = may_sign_up(&server, invite).await;
    let buttons = provider_buttons(&server, &next, invite);
    if !can_sign_up && buttons.is_empty() {
        return error_page(StatusCode::FORBIDDEN, "You need a current invite to sign up here.");
    }
    let mut body = String::new();
    if let Some(error) = &query.error {
        body.push_str(&format!("<p class=\"error\">{}</p>", escape(error)));
    }
    if can_sign_up && server.config().auth.passwords {
        body.push_str(&format!(
            r#"<form method="post" action="/signup">
<input type="hidden" name="next" value="{next}"><input type="hidden" name="invite" value="{invite}">
<label>User name <input name="user" autocomplete="username" pattern="[a-z0-9][a-z0-9._-]*" required autofocus></label>
<label>Your name <input name="display_name" autocomplete="name"></label>
<label>Password (at least 10 characters) <input name="password" type="password" autocomplete="new-password" minlength="10" required></label>
<button>Create account</button></form>"#,
            next = escape(&next),
            invite = escape(invite.unwrap_or_default())
        ));
    }
    body.push_str(&buttons);
    page("Join Silt", &body).into_response()
}

#[derive(Deserialize)]
pub(crate) struct SignupForm {
    user: String,
    display_name: Option<String>,
    password: String,
    invite: Option<String>,
    next: Option<String>,
}

pub(crate) async fn signup(State(server): State<Server>, headers: HeaderMap, Form(form): Form<SignupForm>) -> Response {
    if !same_origin(&server, &headers) {
        return error_page(StatusCode::FORBIDDEN, "This form was sent from another site.");
    }
    let next = safe_next(form.next.as_deref());
    let invite = form.invite.filter(|invite| !invite.is_empty());
    if !server.config().auth.passwords || !may_sign_up(&server, invite.as_deref()).await {
        return error_page(StatusCode::FORBIDDEN, "You need a current invite to sign up here.");
    }
    let new = NewUser {
        name: form.user.trim(),
        display_name: form.display_name.as_deref().map(str::trim).filter(|name| !name.is_empty()),
        email: None,
        password: Some(&form.password),
        admin: false,
    };
    match accounts::create_user(server.db(), &new).await {
        Ok(user) => {
            if let Some(invite) = &invite {
                let _ = accounts::use_invite(server.db(), invite, user).await;
            }
            start_session(&server, user, &next).await
        }
        Err(error) => {
            let mut back = format!("/signup?next={}&error={}", sign_in::encode(&next), sign_in::encode(&error.to_string()));
            if let Some(invite) = &invite {
                back.push_str(&format!("&invite={}", sign_in::encode(invite)));
            }
            Redirect::to(&back).into_response()
        }
    }
}

fn redirect_uri(server: &Server, provider: &str) -> String {
    format!("{}/auth/{provider}/callback", server.config().public_url())
}

pub(crate) async fn provider_start(
    State(server): State<Server>,
    UrlPath(provider): UrlPath<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let endpoints = match sign_in::endpoints(server.config(), server.http(), &provider).await {
        Ok(endpoints) => endpoints,
        Err(error) => return error_page(StatusCode::BAD_REQUEST, &error.to_string()),
    };
    let state = accounts::random_token();
    let saved = SignInState {
        verifier: accounts::random_token(),
        invite: query.get("invite").filter(|invite| !invite.is_empty()).cloned(),
        next: safe_next(query.get("next").map(String::as_str)),
    };
    if accounts::save_sign_in(server.db(), &provider, &state, &saved).await.is_err() {
        return error_page(StatusCode::INTERNAL_SERVER_ERROR, "Could not start signing in.");
    }
    let mut response =
        Redirect::to(&sign_in::authorize_url(&endpoints, &redirect_uri(&server, &provider), &state, &saved.verifier)).into_response();
    let cookie = format!("{SIGN_IN_COOKIE}={state}; Path=/auth; HttpOnly; SameSite=Lax; Max-Age=900{}", secure_attribute(&server));
    response
        .headers_mut()
        .insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).expect("the state is ASCII"));
    response
}

#[derive(Deserialize)]
pub(crate) struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

pub(crate) async fn provider_callback(
    State(server): State<Server>,
    UrlPath(provider): UrlPath<String>,
    headers: HeaderMap,
    Query(query): Query<CallbackQuery>,
) -> Response {
    if let Some(error) = query.error {
        return error_page(StatusCode::BAD_REQUEST, &format!("The provider refused: {error}"));
    }
    let (Some(code), Some(state)) = (query.code, query.state) else {
        return error_page(StatusCode::BAD_REQUEST, "The provider sent no code.");
    };
    if cookie_value(&headers, SIGN_IN_COOKIE).as_deref() != Some(state.as_str()) {
        return error_page(
            StatusCode::BAD_REQUEST,
            "This sign-in was started in another browser; start it again here.",
        );
    }
    let saved = match accounts::take_sign_in(server.db(), &provider, &state).await {
        Ok(Some(saved)) => saved,
        _ => return error_page(StatusCode::BAD_REQUEST, "This sign-in expired or was already used; try again."),
    };
    let identity = match sign_in::endpoints(server.config(), server.http(), &provider).await {
        Ok(endpoints) => sign_in::finish(server.http(), &endpoints, &redirect_uri(&server, &provider), &code, &saved.verifier).await,
        Err(error) => Err(error),
    };
    let identity = match identity {
        Ok(identity) => identity,
        Err(error) => return error_page(StatusCode::BAD_GATEWAY, &error.to_string()),
    };
    match account_for(&server, &identity, saved.invite.as_deref()).await {
        Ok(Some(user)) => start_session(&server, user, &saved.next).await,
        Ok(None) => error_page(
            StatusCode::FORBIDDEN,
            "There is no account for you here, and this server needs an invite to create one.",
        ),
        Err(_) => error_page(StatusCode::INTERNAL_SERVER_ERROR, "Could not sign you in."),
    }
}

/// The account an identity signs in to: the one it is linked to, else the
/// one with its verified email, else a new one when the sign-up policy or
/// an invite allows.
async fn account_for(server: &Server, identity: &sign_in::Identity, invite: Option<&str>) -> Result<Option<i64>, accounts::AccountError> {
    let db = server.db();
    if let Some(user) = accounts::identity_user(db, &identity.provider, &identity.subject).await? {
        return Ok(Some(user));
    }
    if let Some(email) = &identity.email
        && let Some(user) = accounts::user_by_email(db, email).await?
    {
        accounts::link_identity(db, &identity.provider, &identity.subject, user.id).await?;
        return Ok(Some(user.id));
    }
    let invited = match invite {
        Some(invite) => accounts::invite(db, invite).await?.is_some(),
        None => false,
    };
    let signup = &server.config().signup;
    let allowed = invited
        || signup.policy == SignupPolicy::Open
        || (signup.policy == SignupPolicy::Domains
            && identity.email.as_deref().and_then(|email| email.rsplit_once('@')).is_some_and(|(_, domain)| {
                signup.domains.iter().any(|allowed| allowed.eq_ignore_ascii_case(domain))
            }));
    if !allowed {
        return Ok(None);
    }
    let name = accounts::free_name(db, &identity.login).await?;
    let user = accounts::create_user(
        db,
        &NewUser {
            name: &name,
            display_name: identity.display_name.as_deref(),
            email: identity.email.as_deref(),
            password: None,
            admin: false,
        },
    )
    .await?;
    accounts::link_identity(db, &identity.provider, &identity.subject, user).await?;
    if let Some(invite) = invite {
        accounts::use_invite(db, invite, user).await?;
    }
    Ok(Some(user))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_paths_are_followed() {
        assert_eq!(safe_next(Some("/device?code=AB")), "/device?code=AB");
        assert_eq!(safe_next(Some("//evil.example")), "/");
        assert_eq!(safe_next(Some("https://evil.example")), "/");
        assert_eq!(safe_next(Some("/\t/evil.example")), "/");
        assert_eq!(safe_next(Some("/\r\nSet-Cookie: x")), "/");
        assert_eq!(safe_next(None), "/");
        assert_eq!(escape("<a href=\"x\">"), "&lt;a href=&quot;x&quot;&gt;");
    }
}
