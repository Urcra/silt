//! What the network listeners answer before a phone is paired (ADR 0112):
//! the certificate over plain HTTP, and pairing over HTTPS.

use std::sync::Arc;

use axum::Router;
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use serde::Deserialize;

use super::pages::{self, Platform};
use super::phones::{self, AskState};
use super::{Shared, now_ms};

/// The cookie a paired phone's requests carry.
pub(crate) const SESSION_COOKIE: &str = "silt_lan";
/// The cookie that ties a phone's ask to its browser while it waits.
const ASK_COOKIE: &str = "silt_lan_ask";
/// How long a phone stays signed in without opening Silt: 400 days, the
/// most browsers keep a cookie.
const SESSION_SECONDS: u32 = 400 * 24 * 60 * 60;

pub(crate) fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_owned())
}

fn user_agent(headers: &HeaderMap) -> &str {
    headers.get(header::USER_AGENT).and_then(|value| value.to_str().ok()).unwrap_or_default()
}

#[derive(Deserialize)]
struct For {
    #[serde(default, rename = "for")]
    platform: Option<String>,
}

/// The plain-HTTP port's: only the authority's certificate and its page.
pub(crate) fn trust_routes(shared: Arc<Shared>) -> Router {
    Router::new()
        .route("/", get(|| async { Redirect::to("/trust") }))
        .route("/trust", get(trust))
        .route("/silt-ca.crt", get(certificate))
        .route("/silt.mobileconfig", get(profile))
        .with_state(shared)
}

fn authority(shared: &Shared) -> Option<(Vec<u8>, String)> {
    shared.authority.read().expect("lock").clone()
}

fn off() -> Response {
    (StatusCode::SERVICE_UNAVAILABLE, "phones on this network are turned off").into_response()
}

async fn trust(State(shared): State<Arc<Shared>>, headers: HeaderMap, Query(asked): Query<For>) -> Response {
    let Some((_, fingerprint)) = authority(&shared) else { return off() };
    let platform = match asked.platform.as_deref() {
        Some("android") => Platform::Android,
        Some("apple") => Platform::Apple,
        _ => Platform::of(user_agent(&headers)),
    };
    pages::trust(&shared.machine, shared.person.as_deref(), &fingerprint, platform)
}

async fn certificate(State(shared): State<Arc<Shared>>) -> Response {
    let Some((der, _)) = authority(&shared) else { return off() };
    (
        [
            (header::CONTENT_TYPE, "application/x-x509-ca-cert"),
            (header::CONTENT_DISPOSITION, "attachment; filename=\"silt-ca.crt\""),
        ],
        der,
    )
        .into_response()
}

async fn profile(State(shared): State<Arc<Shared>>) -> Response {
    let Some((der, _)) = authority(&shared) else { return off() };
    (
        [
            (header::CONTENT_TYPE, "application/x-apple-aspen-config"),
            (header::CONTENT_DISPOSITION, "attachment; filename=\"silt.mobileconfig\""),
        ],
        pages::mobileconfig(&shared.machine, &shared.label(), &der),
    )
        .into_response()
}

#[derive(Deserialize)]
struct Typed {
    #[serde(default)]
    code: Option<String>,
}

/// Pairing, on the HTTPS listener: no phone has a session yet.
pub(crate) fn pair_routes(shared: Arc<Shared>) -> Router {
    Router::new().route("/pair", get(enter)).route("/pair/{code}", get(pair)).with_state(shared)
}

async fn enter(Query(typed): Query<Typed>) -> Response {
    match typed.code.map(|code| phones::normalized(&code)).filter(|code| !code.is_empty()) {
        Some(code) => Redirect::to(&format!("/pair/{code}")).into_response(),
        None => pages::enter(None),
    }
}

fn set_cookie(name: &str, value: &str, path: &str, seconds: u32) -> (header::HeaderName, String) {
    (header::SET_COOKIE, format!("{name}={value}; Path={path}; Max-Age={seconds}; HttpOnly; Secure; SameSite=Lax"))
}

/// A phone opened a code: it asks, then waits here, looking again every
/// couple of seconds, until the person answers on the machine.
async fn pair(
    State(shared): State<Arc<Shared>>,
    ConnectInfo(super::serve::Peer(peer)): ConnectInfo<super::serve::Peer>,
    headers: HeaderMap,
    Path(code): Path<String>,
) -> Response {
    let now = now_ms();
    let platform = Platform::of(user_agent(&headers));
    // Paired already: opening the link again, or reloading, says so.
    if let Some(caller) = cookie(&headers, SESSION_COOKIE).and_then(|token| shared.phone_by_token(&token)) {
        return pages::paired(&shared.machine, platform, &describe(caller.capabilities));
    }
    if let Some(claim) = cookie(&headers, ASK_COOKIE) {
        let mut pairing = shared.pairing.lock().expect("lock");
        if let Some(ask) = pairing.claimed(&claim, now) {
            let shown = phones::shown(&ask.code);
            match &mut ask.state {
                AskState::Waiting => return pages::waiting(&shared.machine, shared.person.as_deref(), &shown),
                AskState::Denied => return pages::denied(&shared.machine),
                AskState::Allowed { token, phone } => {
                    let (token, phone) = (token.take(), phone.clone());
                    drop(pairing);
                    let can = shared
                        .phones
                        .read()
                        .expect("lock")
                        .phones
                        .iter()
                        .find(|found| found.id == phone)
                        .map(|found| found.capabilities);
                    let mut response = pages::paired(&shared.machine, platform, &can.map(describe).unwrap_or_default());
                    if let Some(token) = token {
                        let headers = response.headers_mut();
                        for (name, value) in
                            [set_cookie(SESSION_COOKIE, &token, "/", SESSION_SECONDS), set_cookie(ASK_COOKIE, "", "/pair", 0)]
                        {
                            if let Ok(value) = value.parse() {
                                headers.append(name, value);
                            }
                        }
                    }
                    return response;
                }
            }
        }
    }
    let asked =
        shared.pairing.lock().expect("lock").ask(&code, phones::describe(user_agent(&headers)), peer.ip().to_string(), now);
    match asked {
        Ok((claim, ask)) => {
            shared.announce();
            let mut response = pages::waiting(&shared.machine, shared.person.as_deref(), &phones::shown(&ask.code));
            let (name, value) = set_cookie(ASK_COOKIE, &claim, "/pair", 600);
            if let Ok(value) = value.parse() {
                response.headers_mut().append(name, value);
            }
            response
        }
        Err(_) => pages::used_up(&shared.machine),
    }
}

/// What a phone may do, in words.
pub(crate) fn describe(capabilities: silt_proto::Capabilities) -> String {
    let mut can = Vec::new();
    if capabilities.answer_permissions {
        can.push("answer permissions");
    }
    if capabilities.send_prompts {
        can.push("send prompts");
    }
    if capabilities.run_agents {
        can.push("start and stop agents");
    }
    if capabilities.review {
        can.push("review merge requests");
    }
    if capabilities.sample {
        can.push("ask an agent a page's question");
    }
    if capabilities.page_tools {
        can.push("call your page connectors");
    }
    match can.as_slice() {
        [] => "read your threads".to_owned(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Refuses requests to any host but this machine's name and addresses, so
/// a page elsewhere can't reach the listener through DNS rebinding.
pub(crate) async fn known_host(State(shared): State<Arc<Shared>>, request: Request, next: Next) -> Response {
    let host = request.headers().get(header::HOST).and_then(|value| value.to_str().ok()).map(str::to_ascii_lowercase);
    if host.is_some_and(|host| shared.hosts().contains(&host)) {
        next.run(request).await
    } else {
        (StatusCode::FORBIDDEN, "unexpected host").into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_phone_may_do_reads_as_a_list() {
        assert_eq!(describe(silt_proto::Capabilities::PHONE), "answer permissions and review merge requests");
        assert_eq!(describe(silt_proto::Capabilities::default()), "read your threads");
    }

    #[test]
    fn cookies_are_read_by_name() {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, "a=1; silt_lan=tok; b=2".parse().unwrap());
        assert_eq!(cookie(&headers, SESSION_COOKIE).as_deref(), Some("tok"));
        assert_eq!(cookie(&headers, ASK_COOKIE), None);
    }
}
