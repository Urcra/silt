//! The trash and purging (ADR 0013) over real HTTP.

use std::collections::HashMap;

use reqwest::StatusCode;
use silt_crdt::{InodeKind, Parent};
use silt_log::{Author, Content, Delta, Family, Op};
use silt_proto::{Listing, LoginReply, PROTOCOL, PROTOCOL_HEADER, PasswordLoginRequest, PullReply, PushRequest, ThreadInfo};
use silt_server::{Admin, Config, Server};
use silt_types::{ActorId, FamilyId, ThreadId};

const FAMILY: FamilyId = FamilyId(90);
const MAIN: ThreadId = ThreadId(91);
const FORK: ThreadId = ThreadId(92);

struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl Client {
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.base))
            .header(PROTOCOL_HEADER, PROTOCOL.to_string())
            .bearer_auth(&self.token)
    }

    async fn status(&self, method: reqwest::Method, path: &str) -> StatusCode {
        self.request(method, path).send().await.expect("sends").status()
    }

    async fn register(&self, thread: ThreadId) {
        let info = ThreadInfo {
            id: thread,
            family: FAMILY,
            repo: "github.com/acme/app".into(),
            created_ms: 1,
            lease: None,
            owner: String::new(),
        };
        let status = self.request(reqwest::Method::POST, "/v1/threads").json(&info).send().await.expect("sends").status();
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    async fn push(&self, deltas: &[Delta]) {
        let status = self
            .request(reqwest::Method::POST, &format!("/v1/families/{FAMILY}/deltas"))
            .json(&PushRequest { deltas: deltas.to_vec() })
            .send()
            .await
            .expect("sends")
            .status();
        assert_eq!(status, StatusCode::OK);
    }

    async fn pulled(&self) -> Vec<ThreadId> {
        let reply: PullReply = self
            .request(reqwest::Method::GET, &format!("/v1/families/{FAMILY}/deltas?after=0"))
            .send()
            .await
            .expect("sends")
            .json()
            .await
            .unwrap_or(PullReply { deltas: Vec::new() });
        reply.deltas.into_iter().map(|(_, delta)| delta.thread).collect()
    }

    async fn listing(&self) -> Listing {
        self.request(reqwest::Method::GET, "/v1/listing").send().await.expect("sends").json().await.expect("listing")
    }
}

fn delta(family: &mut Family, thread: ThreadId, op: Op) -> Delta {
    let delta = family.delta(ActorId(thread.0), thread, Author::User, 0, family.thread_head(thread), op);
    family.apply(delta.clone(), &mut HashMap::new()).expect("applies");
    delta
}

fn file(name: &str) -> Op {
    Op::Create {
        kind: InodeKind::File,
        parent: Parent::Root,
        name: name.into(),
        content: Content::Empty,
        executable: false,
    }
}

#[tokio::test]
async fn purged_threads_leave_what_forks_need_and_never_come_back() {
    let data = tempfile::tempdir().expect("temp dir");
    let admin = Admin::open(data.path()).await.expect("opens");
    admin.add_user("alice", Some("correct horse battery"), false).await.expect("adds");
    admin.add_user("bob", Some("correct horse battery"), false).await.expect("adds");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    tokio::spawn(Server::open_with(data.path(), Config::default()).await.expect("opens").serve(listener));
    let login = |user: &'static str| {
        let base = base.clone();
        async move {
            let reply: LoginReply = reqwest::Client::new()
                .post(format!("{base}/v1/login/password"))
                .header(PROTOCOL_HEADER, PROTOCOL.to_string())
                .json(&PasswordLoginRequest {
                    user: user.into(),
                    password: "correct horse battery".into(),
                    name: "laptop".into(),
                })
                .send()
                .await
                .expect("sends")
                .json()
                .await
                .expect("login");
            Client {
                http: reqwest::Client::new(),
                base,
                token: reply.token,
            }
        }
    };
    let (alice, bob) = (login("alice").await, login("bob").await);

    let mut family = Family::new();
    let shared = delta(&mut family, MAIN, file("shared"));
    let forked = family.delta(ActorId(FORK.0), FORK, Author::User, 0, family.thread_head(MAIN), Op::Fork { from: MAIN });
    family.apply(forked.clone(), &mut HashMap::new()).expect("applies");
    let on_fork = delta(&mut family, FORK, file("fork"));
    let later = delta(&mut family, MAIN, file("later"));
    alice.register(MAIN).await;
    alice.push(&[shared.clone(), forked.clone(), on_fork.clone(), later.clone()]).await;
    bob.register(FORK).await;

    assert_eq!(bob.status(reqwest::Method::POST, &format!("/v1/threads/{MAIN}/delete")).await, StatusCode::FORBIDDEN);
    assert_eq!(alice.status(reqwest::Method::POST, &format!("/v1/threads/{MAIN}/purge")).await, StatusCode::BAD_REQUEST);
    assert_eq!(alice.status(reqwest::Method::POST, &format!("/v1/threads/{MAIN}/delete")).await, StatusCode::NO_CONTENT);
    let listing = bob.listing().await;
    assert!(listing.deleted.contains(&MAIN) && listing.threads.iter().all(|thread| thread.id != MAIN));
    assert_eq!(alice.status(reqwest::Method::POST, &format!("/v1/threads/{MAIN}/purge")).await, StatusCode::NO_CONTENT);

    assert_eq!(bob.pulled().await, vec![MAIN, FORK, FORK], "only what the fork builds on is left");
    assert!(bob.listing().await.purged.contains(&MAIN));
    alice.push(std::slice::from_ref(&later)).await;
    assert_eq!(bob.pulled().await, vec![MAIN, FORK, FORK], "a purged thread's deltas aren't stored again");

    assert_eq!(bob.status(reqwest::Method::POST, &format!("/v1/threads/{FORK}/delete")).await, StatusCode::NO_CONTENT);
    assert_eq!(bob.status(reqwest::Method::POST, &format!("/v1/threads/{FORK}/purge")).await, StatusCode::NO_CONTENT);
    assert!(alice.pulled().await.is_empty(), "a family without threads is gone");
    assert!(bob.listing().await.families.iter().all(|info| info.id != FAMILY));
}
