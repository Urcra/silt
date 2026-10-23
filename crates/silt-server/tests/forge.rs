//! The forge bridge (ADR 0042) against a stand-in for GitHub's API: a
//! thread's pull request gets one comment, edited as the family changes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::routing::{get, patch};
use axum::{Json, Router};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use silt_log::{Author, ConvOp, Delta, Family, GitOp, Op};
use silt_proto::{
    LoginReply, OrgChange, PROTOCOL, PROTOCOL_HEADER, PasswordLoginRequest, PushRequest, ThreadInfo, Visibility,
    VisibilityRequest,
};
use silt_server::config::Forge;
use silt_server::forge::ForgeKind;
use silt_server::{Admin, Config, Server};
use silt_types::{ActorId, FamilyId, GitOid, ThreadId};

const FAMILY: FamilyId = FamilyId(900);
const THREAD: ThreadId = ThreadId(901);
const FORK: ThreadId = ThreadId(902);
const SECRET_FORK: ThreadId = ThreadId(903);
const REPO: &str = "github.com/acme/app";
const PASSWORD: &str = "correct horse battery";

#[derive(Default)]
struct GitHub {
    comments: Vec<Value>,
    posts: usize,
    tokens: Vec<String>,
}

type Shared = Arc<Mutex<GitHub>>;

fn fake_github(state: Shared) -> Router {
    Router::new()
        .route("/user", get(|| async { Json(json!({ "login": "silt-bot" })) }))
        .route(
            "/repos/acme/app/pulls",
            get(|Query(query): Query<HashMap<String, String>>| async move {
                let wanted = query.get("head").map(String::as_str) == Some("acme:feature");
                Json(if wanted {
                    json!([{ "number": 7, "head": { "ref": "feature", "repo": { "full_name": "acme/app" } } }])
                } else {
                    json!([])
                })
            }),
        )
        .route(
            "/repos/acme/app/issues/7/comments",
            get(|State(state): State<Shared>, headers: axum::http::HeaderMap| async move {
                let mut github = state.lock().expect("lock");
                github
                    .tokens
                    .push(headers.get("authorization").and_then(|value| value.to_str().ok()).unwrap_or_default().to_owned());
                Json(Value::Array(github.comments.clone()))
            })
            .post(|State(state): State<Shared>, Json(body): Json<Value>| async move {
                let mut github = state.lock().expect("lock");
                github.posts += 1;
                let comment = json!({ "id": 100 + github.comments.len(), "user": { "login": "silt-bot" }, "body": body["body"] });
                github.comments.push(comment.clone());
                (StatusCode::CREATED, Json(comment))
            }),
        )
        .route(
            "/repos/acme/app/issues/comments/{id}",
            patch(|State(state): State<Shared>, Path(id): Path<u64>, Json(body): Json<Value>| async move {
                let mut github = state.lock().expect("lock");
                let comment = github.comments.iter_mut().find(|comment| comment["id"] == id).expect("a known comment");
                comment["body"] = body["body"].clone();
                Json(comment.clone())
            }),
        )
        .with_state(state)
}

struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl Client {
    async fn sign_in(base: &str, user: &str) -> Self {
        let reply: LoginReply = reqwest::Client::new()
            .post(format!("{base}/v1/login/password"))
            .header(PROTOCOL_HEADER, PROTOCOL.to_string())
            .json(&PasswordLoginRequest { user: user.into(), password: PASSWORD.into(), name: "laptop".into() })
            .send()
            .await
            .expect("sends")
            .json()
            .await
            .expect("login");
        Self { http: reqwest::Client::new(), base: base.to_owned(), token: reply.token }
    }

    async fn send(&self, method: Method, path: &str, body: &impl serde::Serialize) -> StatusCode {
        self.http
            .request(method, format!("{}{path}", self.base))
            .header(PROTOCOL_HEADER, PROTOCOL.to_string())
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .expect("sends")
            .status()
    }

    async fn change(&self, change: OrgChange) -> StatusCode {
        self.send(Method::POST, "/v1/orgs/change", &change).await
    }

    async fn register(&self, thread: ThreadId) {
        let info = ThreadInfo {
            id: thread,
            family: FAMILY,
            repo: REPO.into(),
            created_ms: 1,
            lease: None,
            owner: String::new(),
            collaborators: Vec::new(),
        };
        assert_eq!(self.send(Method::POST, "/v1/threads", &info).await, StatusCode::NO_CONTENT);
    }

    async fn push(&self, deltas: Vec<Delta>) {
        let path = format!("/v1/families/{FAMILY}/deltas");
        assert_eq!(self.send(Method::POST, &path, &PushRequest { deltas }).await, StatusCode::OK);
    }
}

fn write(family: &mut Family, thread: ThreadId, op: Op) -> Delta {
    let delta = family.delta(ActorId(1), thread, Author::User, 0, family.thread_head(thread), op);
    family.apply(delta.clone(), &mut HashMap::new()).expect("applies");
    delta
}

fn wait_for(state: &Shared, what: &str, check: impl Fn(&GitHub) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if check(&state.lock().expect("lock")) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("{what}: {:?}", state.lock().expect("lock").comments);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_threads_pull_request_gets_one_comment_kept_current() {
    let github = Shared::default();
    let forge_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let forge_url = format!("http://{}", forge_listener.local_addr().expect("address"));
    let app = fake_github(Arc::clone(&github));
    tokio::spawn(async move { axum::serve(forge_listener, app).await });

    let data = tempfile::tempdir().expect("temp dir");
    let admin = Admin::open(data.path()).await.expect("opens");
    for name in ["root", "alice"] {
        admin.add_user(name, Some(PASSWORD), name == "root").await.expect("adds");
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let config = Config {
        public_url: Some("https://silt.example".into()),
        forges: vec![Forge {
            host: "github.com".into(),
            kind: ForgeKind::GitHub,
            api_url: Some(forge_url),
            token: Some("forge-token".into()),
            token_file: None,
        }],
        ..Config::default()
    };
    tokio::spawn(Server::open_with(data.path(), config).await.expect("opens").serve(listener));
    let root = Client::sign_in(&base, "root").await;
    let alice = Client::sign_in(&base, "alice").await;
    assert_eq!(root.change(OrgChange::Create { org: "acme".into() }).await, StatusCode::NO_CONTENT);
    let member = OrgChange::AddMember { org: "acme".into(), user: "alice".into(), owner: true };
    assert_eq!(root.change(member).await, StatusCode::NO_CONTENT);

    let mut family = Family::new();
    alice.register(THREAD).await;
    let genesis = write(&mut family, THREAD, Op::Git(GitOp::Genesis { commit: GitOid([1; 20]), branch: Some("feature".into()) }));
    let title = write(&mut family, THREAD, Op::Conv(ConvOp::Title("Fix the [login] redirect".into())));
    alice.push(vec![genesis.clone()]).await;
    assert_eq!(root.change(OrgChange::MoveProject { repo: REPO.into(), org: "acme".into() }).await, StatusCode::NO_CONTENT);
    let on = OrgChange::ProjectSettings {
        repo: REPO.into(),
        name: None,
        default_agent: None,
        default_preset: None,
        git_notes: None,
        forge_comments: Some(true),
    };
    assert_eq!(alice.change(on).await, StatusCode::NO_CONTENT);

    alice.push(vec![title]).await;
    wait_for(&github, "a comment on the pull request", |github| github.comments.len() == 1);
    {
        let github = github.lock().expect("lock");
        let body = github.comments[0]["body"].as_str().expect("a body");
        assert!(body.starts_with(&format!("<!-- silt:thread:{THREAD} -->")), "{body}");
        assert!(
            body.contains(&format!(r"[Fix the \[login\] redirect](https://silt.example/app/t/{THREAD}): 0 turns, 0 commits.")),
            "{body}"
        );
        assert!(github.tokens.iter().all(|token| token == "Bearer forge-token"));
    }

    alice.register(FORK).await;
    alice.register(SECRET_FORK).await;
    let private = VisibilityRequest { repo: None, thread: Some(SECRET_FORK), visibility: Visibility::Private };
    assert_eq!(alice.send(Method::PUT, "/v1/visibility", &private).await, StatusCode::NO_CONTENT);
    let fork = write(&mut family, FORK, Op::Fork { from: THREAD });
    let named = write(&mut family, FORK, Op::Conv(ConvOp::Title("Try another way".into())));
    let secret = write(&mut family, SECRET_FORK, Op::Fork { from: THREAD });
    let secret_title = write(&mut family, SECRET_FORK, Op::Conv(ConvOp::Title("Secret idea".into())));
    alice.push(vec![fork, named, secret, secret_title]).await;
    wait_for(&github, "the fork in the comment", |github| {
        github.comments[0]["body"].as_str().is_some_and(|body| body.contains("**Forks**"))
    });
    let github = github.lock().expect("lock");
    let body = github.comments[0]["body"].as_str().expect("a body");
    assert_eq!((github.comments.len(), github.posts), (1, 1), "edited, not posted again");
    assert!(body.contains(&format!("- [Try another way](https://silt.example/app/t/{FORK}): in progress")), "{body}");
    assert!(!body.contains("Secret"), "a private fork isn't named: {body}");
}
