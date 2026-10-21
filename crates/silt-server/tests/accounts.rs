//! Accounts, sign-in and orgs (ADR 0013) over real HTTP, with a stand-in
//! OIDC provider and GitHub served by the test.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use reqwest::StatusCode;
use reqwest::header::{COOKIE, LOCATION, SET_COOKIE};
use serde_json::{Value, json};
use sha2::Digest;
use silt_crdt::{InodeKind, Parent};
use silt_log::{Author, Content, Family, Op};
use silt_proto::{
    DevicePollReply, DevicePollRequest, DeviceStartReply, DeviceStartRequest, Listing, LoginReply, OrgChange, OrgsReply,
    PROTOCOL, PROTOCOL_HEADER, PasswordLoginRequest, PushRequest, ThreadInfo, Visibility, VisibilityRequest,
};
use silt_server::config::{Oidc, SignupPolicy};
use silt_server::{Admin, Config, Server};
use silt_types::{ActorId, FamilyId, ThreadId};

async fn start(data: &Path, config: impl FnOnce(&str) -> Config) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let mut config = config(&base);
    config.public_url = Some(base.clone());
    let server = Server::open_with(data, config).await.expect("opens");
    tokio::spawn(server.serve(listener));
    base
}

fn browser() -> reqwest::Client {
    reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().expect("client")
}

fn api(http: &reqwest::Client, method: reqwest::Method, url: String) -> reqwest::RequestBuilder {
    http.request(method, url).header(PROTOCOL_HEADER, PROTOCOL.to_string())
}

fn session_cookie(response: &reqwest::Response) -> Option<String> {
    response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with("silt_session=") && !value.starts_with("silt_session=;"))
        .map(|value| value.split(';').next().expect("pair").to_owned())
}

async fn password_login(base: &str, user: &str, password: &str) -> reqwest::Response {
    api(&browser(), reqwest::Method::POST, format!("{base}/v1/login/password"))
        .json(&PasswordLoginRequest { user: user.into(), password: password.into(), name: "laptop".into() })
        .send()
        .await
        .expect("sends")
}

async fn token(base: &str, user: &str, password: &str) -> String {
    let reply: LoginReply = password_login(base, user, password).await.json().await.expect("login");
    reply.token
}

#[tokio::test]
async fn passwords_sign_devices_in_and_out() {
    let data = tempfile::tempdir().expect("temp dir");
    let admin = Admin::open(data.path()).await.expect("opens");
    admin.add_user("alice", Some("correct horse battery"), false).await.expect("adds");
    assert!(admin.add_user("alice", None, false).await.is_err(), "names are unique");
    assert!(admin.add_user("bob", Some("short"), false).await.is_err(), "passwords have a minimum length");
    let base = start(data.path(), |_| Config::default()).await;

    assert_eq!(password_login(&base, "alice", "wrong horse battery").await.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(password_login(&base, "nobody", "correct horse battery").await.status(), StatusCode::UNAUTHORIZED);
    let reply: LoginReply = password_login(&base, "alice", "correct horse battery").await.json().await.expect("login");
    assert_eq!(reply.user, "alice");
    let http = browser();
    let listing =
        api(&http, reqwest::Method::GET, format!("{base}/v1/listing")).bearer_auth(&reply.token).send().await.expect("sends");
    assert_eq!(listing.status(), StatusCode::OK);
    let out =
        api(&http, reqwest::Method::DELETE, format!("{base}/v1/device")).bearer_auth(&reply.token).send().await.expect("sends");
    assert_eq!(out.status(), StatusCode::NO_CONTENT);
    let after =
        api(&http, reqwest::Method::GET, format!("{base}/v1/listing")).bearer_auth(&reply.token).send().await.expect("sends");
    assert_eq!(after.status(), StatusCode::UNAUTHORIZED, "a signed-out device's token stops working");

    let closed = tempfile::tempdir().expect("temp dir");
    Admin::open(closed.path()).await.expect("opens").add_user("carol", Some("correct horse battery"), false).await.expect("adds");
    let base = start(closed.path(), |_| {
        let mut config = Config::default();
        config.auth.passwords = false;
        config
    })
    .await;
    assert_eq!(password_login(&base, "carol", "correct horse battery").await.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_name_that_keeps_failing_has_to_wait() {
    let data = tempfile::tempdir().expect("temp dir");
    Admin::open(data.path()).await.expect("opens").add_user("alice", Some("correct horse battery"), false).await.expect("adds");
    let base = start(data.path(), |_| Config::default()).await;
    for _ in 0..10 {
        assert_eq!(password_login(&base, "alice", "a wrong guess").await.status(), StatusCode::UNAUTHORIZED);
    }
    assert_eq!(password_login(&base, "alice", "another guess").await.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        password_login(&base, "alice", "correct horse battery").await.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "guessing right during the wait doesn't help"
    );
}

#[tokio::test]
async fn people_sign_devices_out_and_admins_disable_people() {
    let data = tempfile::tempdir().expect("temp dir");
    let admin = Admin::open(data.path()).await.expect("opens");
    admin.add_user("alice", Some("correct horse battery"), false).await.expect("adds");
    let base = start(data.path(), |_| Config::default()).await;
    let http = browser();
    let status = |token: String| {
        let (http, base) = (http.clone(), base.clone());
        async move {
            api(&http, reqwest::Method::GET, format!("{base}/v1/listing"))
                .bearer_auth(token)
                .send()
                .await
                .expect("sends")
                .status()
        }
    };
    let home = |cookie: String| {
        let (http, base) = (http.clone(), base.clone());
        async move { http.get(format!("{base}/")).header(COOKIE, cookie).send().await.expect("sends") }
    };
    let (first, second) =
        (token(&base, "alice", "correct horse battery").await, token(&base, "alice", "correct horse battery").await);
    let (here, elsewhere) = (
        sign_in_page(&base, "alice", "correct horse battery").await,
        sign_in_page(&base, "alice", "correct horse battery").await,
    );

    let html = home(here.clone()).await.text().await.expect("html");
    let (_, rest) = html.split_once(r#"name="device" value=""#).expect("a device is listed");
    let device = rest.split('"').next().expect("an id");
    let revoked = http
        .post(format!("{base}/devices/revoke"))
        .header(COOKIE, &here)
        .form(&[("device", device)])
        .send()
        .await
        .expect("sends");
    assert_eq!(revoked.status(), StatusCode::SEE_OTHER);
    let mut statuses = [status(first.clone()).await, status(second.clone()).await];
    statuses.sort();
    assert_eq!(statuses, [StatusCode::OK, StatusCode::UNAUTHORIZED], "one device is signed out");
    let remaining = if status(first.clone()).await == StatusCode::OK { first } else { second };

    let foreign = http
        .post(format!("{base}/logout"))
        .header(COOKIE, &here)
        .header("origin", "https://evil.example")
        .send()
        .await
        .expect("sends");
    assert_eq!(foreign.status(), StatusCode::FORBIDDEN, "other sites can't sign people out");
    let ended = http.post(format!("{base}/sessions/end-others")).header(COOKIE, &here).send().await.expect("sends");
    assert_eq!(ended.status(), StatusCode::SEE_OTHER);
    assert_eq!(home(here.clone()).await.status(), StatusCode::OK, "this browser stays signed in");
    assert_eq!(home(elsewhere).await.headers()[LOCATION], "/login", "other browsers are signed out");

    admin.set_disabled("alice", true).await.expect("disables");
    assert_eq!(status(remaining).await, StatusCode::UNAUTHORIZED);
    assert_eq!(home(here).await.headers()[LOCATION], "/login");
    assert_eq!(password_login(&base, "alice", "correct horse battery").await.status(), StatusCode::UNAUTHORIZED);
    admin.set_disabled("alice", false).await.expect("enables");
    assert_eq!(password_login(&base, "alice", "correct horse battery").await.status(), StatusCode::OK);
}

/// Signs in on the server's page and returns the session cookie.
async fn sign_in_page(base: &str, user: &str, password: &str) -> String {
    let response = browser()
        .post(format!("{base}/login"))
        .form(&[("user", user), ("password", password), ("next", "/")])
        .send()
        .await
        .expect("sends");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    session_cookie(&response).expect("a session cookie")
}

#[tokio::test]
async fn devices_sign_in_through_the_browser() {
    let data = tempfile::tempdir().expect("temp dir");
    Admin::open(data.path()).await.expect("opens").add_user("alice", Some("correct horse battery"), false).await.expect("adds");
    let base = start(data.path(), |_| Config::default()).await;
    let http = browser();
    let started: DeviceStartReply = api(&http, reqwest::Method::POST, format!("{base}/v1/device/start"))
        .json(&DeviceStartRequest { name: "home".into() })
        .send()
        .await
        .expect("sends")
        .json()
        .await
        .expect("start");
    assert!(started.verification_url.starts_with(&format!("{base}/device?code=")));
    let poll = || async {
        api(&http, reqwest::Method::POST, format!("{base}/v1/device/poll"))
            .json(&DevicePollRequest { device_code: started.device_code.clone() })
            .send()
            .await
            .expect("sends")
            .json::<DevicePollReply>()
            .await
            .expect("poll")
    };
    assert_eq!(poll().await, DevicePollReply::Pending);

    let page = http.get(&started.verification_url).send().await.expect("sends");
    assert_eq!(page.status(), StatusCode::SEE_OTHER, "signing in comes first");
    assert!(page.headers()[LOCATION].to_str().expect("ascii").starts_with("/login?next=%2Fdevice"));

    let refused = browser()
        .post(format!("{base}/login"))
        .form(&[("user", "alice"), ("password", "nope"), ("next", "/")])
        .send()
        .await
        .expect("sends");
    assert!(refused.headers()[LOCATION].to_str().expect("ascii").contains("error="), "a wrong password is reported");
    let cookie = sign_in_page(&base, "alice", "correct horse battery").await;
    let page = http.get(&started.verification_url).header(COOKIE, &cookie).send().await.expect("sends");
    let html = page.text().await.expect("html");
    assert!(html.contains("Sign in <strong>home</strong> as <strong>alice</strong>"), "{html}");
    assert!(html.contains(&started.user_code));

    let foreign = http
        .post(format!("{base}/device"))
        .header(COOKIE, &cookie)
        .header("origin", "https://evil.example")
        .form(&[("code", started.user_code.as_str()), ("action", "approve")])
        .send()
        .await
        .expect("sends");
    assert_eq!(foreign.status(), StatusCode::FORBIDDEN, "approvals posted from other sites are refused");
    let approved = http
        .post(format!("{base}/device"))
        .header(COOKIE, &cookie)
        .header("origin", &base)
        .form(&[("code", started.user_code.to_lowercase().as_str()), ("action", "approve")])
        .send()
        .await
        .expect("sends");
    assert_eq!(approved.status(), StatusCode::OK);
    let DevicePollReply::Approved { login } = poll().await else {
        panic!("the device is approved");
    };
    assert_eq!(login.user, "alice");
    assert_eq!(poll().await, DevicePollReply::Gone, "a device code works once");
    let listing =
        api(&http, reqwest::Method::GET, format!("{base}/v1/listing")).bearer_auth(&login.token).send().await.expect("sends");
    assert_eq!(listing.status(), StatusCode::OK);
}

#[tokio::test]
async fn invites_let_people_sign_up() {
    let data = tempfile::tempdir().expect("temp dir");
    let admin = Admin::open(data.path()).await.expect("opens");
    let invite = admin.invite(None, 7).await.expect("invites");
    let base = start(data.path(), |_| Config::default()).await;
    let http = browser();
    assert_eq!(http.get(format!("{base}/signup")).send().await.expect("sends").status(), StatusCode::FORBIDDEN);
    let page = http.get(format!("{base}/signup?invite={invite}")).send().await.expect("sends");
    assert_eq!(page.status(), StatusCode::OK);
    let form = [
        ("user", "dave"),
        ("display_name", "Dave"),
        ("password", "correct horse battery"),
        ("invite", invite.as_str()),
        ("next", "/"),
    ];
    let signed_up = http.post(format!("{base}/signup")).form(&form).send().await.expect("sends");
    assert_eq!(signed_up.status(), StatusCode::SEE_OTHER);
    assert!(session_cookie(&signed_up).is_some());
    let again = http
        .post(format!("{base}/signup"))
        .form(&[("user", "eve"), ("password", "correct horse battery"), ("invite", invite.as_str())])
        .send()
        .await
        .expect("sends");
    assert_eq!(again.status(), StatusCode::FORBIDDEN, "an invite works once");
    assert_eq!(token(&base, "dave", "correct horse battery").await.len(), 64);

    let racing = admin.invite(None, 7).await.expect("invites");
    let mut racers = tokio::task::JoinSet::new();
    for n in 0..8 {
        let (http, base, racing) = (browser(), base.clone(), racing.clone());
        racers.spawn(async move {
            let form = [("user", format!("racer{n}")), ("password", "correct horse battery".into()), ("invite", racing)];
            let response = http.post(format!("{base}/signup")).form(&form).send().await.expect("sends");
            session_cookie(&response).is_some()
        });
    }
    let signed_up = racers.join_all().await.into_iter().filter(|signed_up| *signed_up).count();
    assert_eq!(signed_up, 1, "one invite makes one account");

    let failing = admin.invite(None, 7).await.expect("invites");
    let taken = http
        .post(format!("{base}/signup"))
        .form(&[("user", "dave"), ("password", "correct horse battery"), ("invite", failing.as_str())])
        .send()
        .await
        .expect("sends");
    assert_eq!(taken.status(), StatusCode::SEE_OTHER, "back to the form with an error");
    let retried = http
        .post(format!("{base}/signup"))
        .form(&[("user", "frank"), ("password", "correct horse battery"), ("invite", failing.as_str())])
        .send()
        .await
        .expect("sends");
    assert!(session_cookie(&retried).is_some(), "a failed sign-up gives the invite back");
}

/// What the stand-in provider remembers between requests.
#[derive(Default)]
struct Provider {
    challenges: HashMap<String, String>,
    tokens: HashMap<String, Value>,
    next_user: Option<Value>,
}

type Shared = Arc<Mutex<Provider>>;

async fn discovery(State(base): State<(Shared, String)>) -> Json<Value> {
    let base = base.1;
    Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/authorize"),
        "token_endpoint": format!("{base}/token"),
        "userinfo_endpoint": format!("{base}/userinfo"),
    }))
}

async fn token_endpoint(
    State((provider, _)): State<(Shared, String)>,
    Form(form): Form<HashMap<String, String>>,
) -> (axum::http::StatusCode, Json<Value>) {
    let mut provider = provider.lock().expect("lock");
    let code = form.get("code").cloned().unwrap_or_default();
    let verifier = form.get("code_verifier").cloned().unwrap_or_default();
    let challenge = silt_challenge(&verifier);
    if provider.challenges.get(&code) != Some(&challenge) || form.get("client_secret").map(String::as_str) != Some("shh") {
        return (axum::http::StatusCode::BAD_REQUEST, Json(json!({"error": "invalid_grant"})));
    }
    let user = provider.next_user.clone().expect("a user to sign in");
    let access = format!("access-{code}");
    provider.tokens.insert(access.clone(), user);
    (axum::http::StatusCode::OK, Json(json!({"access_token": access, "token_type": "bearer"})))
}

fn silt_challenge(verifier: &str) -> String {
    let digest = sha2::Sha256::digest(verifier.as_bytes());
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in digest.chunks(3) {
        let mut buffer = [0u8; 3];
        buffer[..chunk.len()].copy_from_slice(chunk);
        let triple = (u32::from(buffer[0]) << 16) | (u32::from(buffer[1]) << 8) | u32::from(buffer[2]);
        for index in 0..=chunk.len() {
            out.push(char::from(ALPHABET[((triple >> (18 - 6 * index)) & 0x3f) as usize]));
        }
    }
    out
}

async fn userinfo(
    State((provider, _)): State<(Shared, String)>,
    headers: axum::http::HeaderMap,
) -> (axum::http::StatusCode, Json<Value>) {
    let token = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();
    match provider.lock().expect("lock").tokens.get(token) {
        Some(user) => (axum::http::StatusCode::OK, Json(user.clone())),
        None => (axum::http::StatusCode::UNAUTHORIZED, Json(json!({}))),
    }
}

async fn github_emails(State((provider, _)): State<(Shared, String)>, headers: axum::http::HeaderMap) -> Json<Value> {
    let (_, user) = userinfo(State((provider, String::new())), headers).await;
    Json(json!([{"email": user.0["email_for_api"], "primary": true, "verified": true}]))
}

async fn provider() -> (Shared, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let shared = Shared::default();
    let app = Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/token", post(token_endpoint))
        .route("/login/oauth/access_token", post(token_endpoint))
        .route("/userinfo", get(userinfo))
        .route("/user", get(userinfo))
        .route("/user/emails", get(github_emails))
        .with_state((shared.clone(), base.clone()));
    tokio::spawn(async move { axum::serve(listener, app).await });
    (shared, base)
}

/// Goes through a provider sign-in as the browser would; returns the
/// server's final answer.
async fn provider_sign_in(base: &str, provider: &Shared, kind: &str, query: &str) -> reqwest::Response {
    let http = browser();
    let start = http.get(format!("{base}/auth/{kind}/start?{query}")).send().await.expect("sends");
    assert_eq!(start.status(), StatusCode::SEE_OTHER);
    let authorize = reqwest::Url::parse(start.headers()[LOCATION].to_str().expect("ascii")).expect("url");
    let params: HashMap<String, String> = authorize.query_pairs().into_owned().collect();
    assert_eq!(params["code_challenge_method"], "S256");
    assert_eq!(params["redirect_uri"], format!("{base}/auth/{kind}/callback"));
    let code = format!("code-{}", params["state"]);
    provider.lock().expect("lock").challenges.insert(code.clone(), params["code_challenge"].clone());
    let cookie = start.headers()[SET_COOKIE].to_str().expect("ascii").split(';').next().expect("cookie").to_owned();
    let callback = format!("{base}/auth/{kind}/callback?code={code}&state={}", params["state"]);
    let elsewhere = http.get(&callback).send().await.expect("sends");
    assert_eq!(elsewhere.status(), StatusCode::BAD_REQUEST, "a callback from another browser is refused");
    http.get(&callback).header(COOKIE, cookie).send().await.expect("sends")
}

#[tokio::test]
async fn oidc_and_github_sign_people_in_by_policy() {
    let (provider, issuer) = provider().await;
    let data = tempfile::tempdir().expect("temp dir");
    let admin = Admin::open(data.path()).await.expect("opens");
    let invite = admin.invite(None, 1).await.expect("invites");
    let base = start(data.path(), |_| {
        let mut config = Config::default();
        config.signup.policy = SignupPolicy::Domains;
        config.signup.domains = vec!["example.com".into()];
        config.auth.oidc = Some(Oidc {
            name: "Example SSO".into(),
            issuer: issuer.clone(),
            client_id: "silt".into(),
            client_secret: Some("shh".into()),
            client_secret_file: None,
            scopes: "openid email profile".into(),
        });
        config.auth.github = Some(silt_server::config::GitHub {
            client_id: "silt-gh".into(),
            client_secret: Some("shh".into()),
            client_secret_file: None,
            web_url: issuer.clone(),
            api_url: issuer.clone(),
        });
        config
    })
    .await;
    let login_page = browser().get(format!("{base}/login")).send().await.expect("sends").text().await.expect("html");
    assert!(login_page.contains("Continue with Example SSO") && login_page.contains("Continue with GitHub"));

    provider.lock().expect("lock").next_user = Some(
        json!({"sub": "u-1", "preferred_username": "Grace", "name": "Grace H", "email": "grace@example.com", "email_verified": true}),
    );
    let signed_in = provider_sign_in(&base, &provider, "oidc", "next=%2Fdevice%3Fcode%3DX").await;
    assert_eq!(signed_in.status(), StatusCode::SEE_OTHER, "{:?}", signed_in.text().await);
    assert_eq!(signed_in.headers()[LOCATION], "/device?code=X");
    assert!(session_cookie(&signed_in).is_some());
    let users = admin.users().await.expect("users");
    assert!(users.iter().any(|user| user.name == "grace" && user.email.as_deref() == Some("grace@example.com")));

    let again = provider_sign_in(&base, &provider, "oidc", "").await;
    assert_eq!(again.status(), StatusCode::SEE_OTHER, "the same identity signs in to the same account");
    assert_eq!(admin.users().await.expect("users").len(), users.len());

    provider.lock().expect("lock").next_user =
        Some(json!({"sub": "u-2", "preferred_username": "mallory", "email": "mallory@elsewhere.org", "email_verified": true}));
    assert_eq!(
        provider_sign_in(&base, &provider, "oidc", "").await.status(),
        StatusCode::FORBIDDEN,
        "other domains need an invite"
    );
    provider.lock().expect("lock").next_user =
        Some(json!({"sub": "u-3", "preferred_username": "ivan", "email": "ivan@example.com", "email_verified": false}));
    assert_eq!(
        provider_sign_in(&base, &provider, "oidc", "").await.status(),
        StatusCode::FORBIDDEN,
        "unverified emails don't count"
    );
    let invited = provider_sign_in(&base, &provider, "oidc", &format!("invite={invite}")).await;
    assert_eq!(invited.status(), StatusCode::SEE_OTHER, "an invite lets anyone in");

    provider.lock().expect("lock").next_user =
        Some(json!({"id": 4242, "login": "Grace-GH", "name": "Grace H", "email_for_api": "grace@example.com"}));
    let github = provider_sign_in(&base, &provider, "github", "").await;
    assert_eq!(github.status(), StatusCode::SEE_OTHER, "{:?}", github.text().await);
    assert_eq!(
        admin.users().await.expect("users").iter().filter(|user| user.name.starts_with("grace")).count(),
        1,
        "a verified email links the GitHub identity to the existing account"
    );
}

const FAMILY: FamilyId = FamilyId(77);
const THREAD: ThreadId = ThreadId(78);

async fn change(base: &str, token: &str, change: OrgChange) -> StatusCode {
    api(&browser(), reqwest::Method::POST, format!("{base}/v1/orgs/change"))
        .bearer_auth(token)
        .json(&change)
        .send()
        .await
        .expect("sends")
        .status()
}

async fn listing(base: &str, token: &str) -> Listing {
    api(&browser(), reqwest::Method::GET, format!("{base}/v1/listing"))
        .bearer_auth(token)
        .send()
        .await
        .expect("sends")
        .json()
        .await
        .expect("listing")
}

#[tokio::test]
async fn orgs_and_teams_decide_who_sees_a_project() {
    let data = tempfile::tempdir().expect("temp dir");
    let admin = Admin::open(data.path()).await.expect("opens");
    for (name, is_admin) in [("root", true), ("alice", false), ("bob", false), ("carol", false)] {
        admin.add_user(name, Some("correct horse battery"), is_admin).await.expect("adds");
    }
    let base = start(data.path(), |_| Config::default()).await;
    let [root, alice, bob, carol] = [
        token(&base, "root", "correct horse battery").await,
        token(&base, "alice", "correct horse battery").await,
        token(&base, "bob", "correct horse battery").await,
        token(&base, "carol", "correct horse battery").await,
    ];
    let acme = || "acme".to_owned();
    assert_eq!(change(&base, &alice, OrgChange::Create { org: acme() }).await, StatusCode::FORBIDDEN);
    assert_eq!(change(&base, &root, OrgChange::Create { org: acme() }).await, StatusCode::NO_CONTENT);
    for user in ["alice", "bob"] {
        let add = OrgChange::AddMember { org: acme(), user: user.into(), owner: user == "alice" };
        assert_eq!(change(&base, &root, add).await, StatusCode::NO_CONTENT);
    }
    assert_eq!(change(&base, &alice, OrgChange::CreateTeam { org: acme(), team: "core".into() }).await, StatusCode::NO_CONTENT);
    let into_core = OrgChange::AddToTeam { org: acme(), team: "core".into(), user: "alice".into() };
    assert_eq!(change(&base, &bob, into_core.clone()).await, StatusCode::FORBIDDEN, "members don't manage the org");
    assert_eq!(change(&base, &alice, into_core).await, StatusCode::NO_CONTENT);
    let outsider = OrgChange::AddToTeam { org: acme(), team: "core".into(), user: "carol".into() };
    assert_eq!(change(&base, &alice, outsider).await, StatusCode::BAD_REQUEST, "teams hold org members only");

    let thread = ThreadInfo {
        id: THREAD,
        family: FAMILY,
        repo: "github.com/acme/app".into(),
        created_ms: 1,
        lease: None,
        owner: String::new(),
        collaborators: Vec::new(),
    };
    let http = browser();
    let registered = api(&http, reqwest::Method::POST, format!("{base}/v1/threads"))
        .bearer_auth(&alice)
        .json(&thread)
        .send()
        .await
        .expect("sends");
    assert_eq!(registered.status(), StatusCode::NO_CONTENT);
    let family = Family::new();
    let delta = family.delta(
        ActorId(1),
        THREAD,
        Author::User,
        0,
        family.thread_head(THREAD),
        Op::Create {
            kind: InodeKind::File,
            parent: Parent::Root,
            name: "a.txt".into(),
            content: Content::Empty,
            executable: false,
        },
    );
    let pushed = api(&http, reqwest::Method::POST, format!("{base}/v1/families/{FAMILY}/deltas"))
        .bearer_auth(&alice)
        .json(&PushRequest { deltas: vec![delta] })
        .send()
        .await
        .expect("sends");
    assert_eq!(pushed.status(), StatusCode::OK);

    // Alice's first org is `default`, which every user joins; move it.
    assert_eq!(listing(&base, &carol).await.threads.len(), 1, "in `default`, everyone sees it");
    let moved = OrgChange::MoveProject { repo: thread.repo.clone(), org: acme() };
    assert_eq!(change(&base, &alice, moved.clone()).await, StatusCode::FORBIDDEN, "alice doesn't own `default`");
    assert_eq!(change(&base, &root, moved).await, StatusCode::NO_CONTENT);
    assert_eq!(listing(&base, &bob).await.threads.len(), 1, "org members see it");
    assert!(listing(&base, &carol).await.threads.is_empty(), "people outside the org don't");

    let team_only = OrgChange::DefaultVisibility { org: acme(), visibility: Visibility::Team("core".into()) };
    assert_eq!(change(&base, &alice, team_only).await, StatusCode::NO_CONTENT);
    assert!(listing(&base, &bob).await.threads.is_empty(), "the org default now limits it to the team");
    assert_eq!(listing(&base, &alice).await.threads.len(), 1);
    let open_project = VisibilityRequest { repo: Some(thread.repo.clone()), thread: None, visibility: Visibility::Org };
    let set = |token: &str| {
        api(&http, reqwest::Method::PUT, format!("{base}/v1/visibility")).bearer_auth(token.to_owned()).json(&open_project)
    };
    assert_eq!(set(&bob).send().await.expect("sends").status(), StatusCode::FORBIDDEN);
    assert_eq!(set(&alice).send().await.expect("sends").status(), StatusCode::NO_CONTENT);
    assert_eq!(listing(&base, &bob).await.threads.len(), 1, "the project's setting wins over the org default");

    let orgs: OrgsReply = api(&http, reqwest::Method::GET, format!("{base}/v1/orgs"))
        .bearer_auth(&bob)
        .send()
        .await
        .expect("sends")
        .json()
        .await
        .expect("orgs");
    let acme_org = orgs.orgs.iter().find(|org| org.name == "acme").expect("bob sees acme");
    assert_eq!(acme_org.role, silt_proto::OrgRole::Member);
    assert_eq!(acme_org.projects, vec!["github.com/acme/app".to_owned()]);
    assert_eq!(acme_org.teams[0].members, vec!["alice".to_owned()]);
    let carol_orgs: OrgsReply = api(&http, reqwest::Method::GET, format!("{base}/v1/orgs"))
        .bearer_auth(&carol)
        .send()
        .await
        .expect("sends")
        .json()
        .await
        .expect("orgs");
    assert!(carol_orgs.orgs.iter().all(|org| org.name != "acme"));

    let core_project =
        VisibilityRequest { repo: Some(thread.repo.clone()), thread: None, visibility: Visibility::Team("core".into()) };
    let set = api(&http, reqwest::Method::PUT, format!("{base}/v1/visibility")).bearer_auth(&alice).json(&core_project);
    assert_eq!(set.send().await.expect("sends").status(), StatusCode::NO_CONTENT);
    assert_eq!(change(&base, &alice, OrgChange::RemoveTeam { org: acme(), team: "core".into() }).await, StatusCode::NO_CONTENT);
    assert_eq!(change(&base, &alice, OrgChange::CreateTeam { org: acme(), team: "core".into() }).await, StatusCode::NO_CONTENT);
    let bob_into_core = OrgChange::AddToTeam { org: acme(), team: "core".into(), user: "bob".into() };
    assert_eq!(change(&base, &alice, bob_into_core).await, StatusCode::NO_CONTENT);
    assert!(listing(&base, &bob).await.threads.is_empty(), "a new team of the same name doesn't inherit the old one's projects");

    let demote = |user: &str| OrgChange::AddMember { org: acme(), user: user.into(), owner: false };
    assert_eq!(change(&base, &root, demote("alice")).await, StatusCode::NO_CONTENT);
    assert_eq!(change(&base, &root, demote("root")).await, StatusCode::BAD_REQUEST, "the last owner stays an owner");
    let leave = OrgChange::RemoveMember { org: acme(), user: "root".into() };
    assert_eq!(change(&base, &root, leave).await, StatusCode::BAD_REQUEST, "the last owner can't leave");
}
