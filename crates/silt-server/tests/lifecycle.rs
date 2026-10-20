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
            collaborators: Vec::new(),
        };
        let status = self.request(reqwest::Method::POST, "/v1/threads").json(&info).send().await.expect("sends").status();
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    async fn push(&self, deltas: &[Delta]) {
        assert_eq!(self.try_push(deltas).await, StatusCode::OK);
    }

    async fn try_push(&self, deltas: &[Delta]) -> StatusCode {
        self.request(reqwest::Method::POST, &format!("/v1/families/{FAMILY}/deltas"))
            .json(&PushRequest { deltas: deltas.to_vec() })
            .send()
            .await
            .expect("sends")
            .status()
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

/// A server with Alice and Bob signed in.
async fn two_people() -> (tempfile::TempDir, Client, Client) {
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
    (data, alice, bob)
}

#[tokio::test]
async fn only_the_owner_and_collaborators_write_a_thread() {
    let (_data, alice, bob) = two_people().await;
    let mut family = Family::new();
    let first = delta(&mut family, MAIN, file("a"));
    alice.register(MAIN).await;
    alice.push(std::slice::from_ref(&first)).await;
    let bobs = family.clone();
    let edit = bobs.delta(ActorId(77), MAIN, Author::User, 0, bobs.thread_head(MAIN), file("bob"));
    assert_eq!(bob.try_push(std::slice::from_ref(&edit)).await, StatusCode::FORBIDDEN);
    let comment = bobs.delta(
        ActorId(78),
        MAIN,
        Author::User,
        0,
        bobs.thread_head(MAIN),
        Op::Comment(silt_log::CommentOp::Added {
            target: silt_log::CommentTarget::Message { delta: first.span.first() },
            text: "looks good".into(),
            by: "bob".into(),
        }),
    );
    assert_eq!(bob.try_push(&[comment]).await, StatusCode::OK, "anyone who sees a thread comments on it");
    assert_eq!(bob.try_push(std::slice::from_ref(&first)).await, StatusCode::OK, "known deltas sync back freely");

    let share = |remove: bool| silt_proto::ShareRequest { user: "bob".into(), remove };
    let shared = bob.request(reqwest::Method::POST, &format!("/v1/threads/{MAIN}/collaborators")).json(&share(false)).send().await.expect("sends");
    assert_eq!(shared.status(), StatusCode::FORBIDDEN, "only the owner shares");
    let shared = alice.request(reqwest::Method::POST, &format!("/v1/threads/{MAIN}/collaborators")).json(&share(false)).send().await.expect("sends");
    assert_eq!(shared.status(), StatusCode::NO_CONTENT);
    assert_eq!(bob.listing().await.threads[0].collaborators, vec!["bob".to_owned()]);
    assert_eq!(bob.try_push(&[edit]).await, StatusCode::OK, "a collaborator writes");
}

#[tokio::test]
async fn sharing_does_not_list_the_accounts() {
    let (_data, alice, _bob) = two_people().await;
    let mut family = Family::new();
    let first = delta(&mut family, MAIN, file("a"));
    alice.register(MAIN).await;
    alice.push(&[first]).await;
    let share = |user: &str| {
        alice
            .request(reqwest::Method::POST, &format!("/v1/threads/{MAIN}/collaborators"))
            .json(&silt_proto::ShareRequest { user: user.into(), remove: true })
            .send()
    };
    for n in 0..10 {
        assert_eq!(share(&format!("guess{n}")).await.expect("sends").status(), StatusCode::BAD_REQUEST);
    }
    assert_eq!(share("bob").await.expect("sends").status(), StatusCode::TOO_MANY_REQUESTS, "known names wait too");
}

#[tokio::test]
async fn nobody_writes_as_another_persons_device() {
    let (_data, alice, bob) = two_people().await;
    let mut family = Family::new();
    let first = delta(&mut family, MAIN, file("a"));
    alice.register(MAIN).await;
    alice.push(std::slice::from_ref(&first)).await;

    // Bob takes Alice's actor and her next lamport for a comment, which
    // would make her real next delta conflict.
    let mut forged = family.clone();
    let comment = forged.delta(
        ActorId(MAIN.0),
        MAIN,
        Author::User,
        0,
        forged.thread_head(MAIN),
        Op::Comment(silt_log::CommentOp::Added {
            target: silt_log::CommentTarget::Message { delta: first.span.first() },
            text: "mine now".into(),
            by: "bob".into(),
        }),
    );
    forged.apply(comment.clone(), &mut HashMap::new()).expect("applies");
    assert_eq!(bob.try_push(&[comment]).await, StatusCode::OK);
    assert_eq!(alice.pulled().await.len(), 1, "the forged delta was not stored");

    let second = delta(&mut family, MAIN, file("b"));
    alice.push(&[second]).await;
    assert_eq!(alice.pulled().await.len(), 2);
}

#[tokio::test]
async fn nobody_builds_on_history_they_cannot_read() {
    let (_data, alice, bob) = two_people().await;
    let mut family = Family::new();
    alice.register(MAIN).await;
    alice.register(FORK).await;
    let open = delta(&mut family, MAIN, file("open"));
    alice.push(std::slice::from_ref(&open)).await;
    let private = silt_proto::VisibilityRequest {
        repo: None,
        thread: Some(FORK),
        visibility: silt_proto::Visibility::Private,
    };
    let set = alice.request(reqwest::Method::PUT, "/v1/visibility").json(&private).send().await.expect("sends");
    assert_eq!(set.status(), StatusCode::NO_CONTENT);
    let secret = delta(&mut family, FORK, file("secret"));
    alice.push(std::slice::from_ref(&secret)).await;

    let bobs_thread = ThreadId(93);
    let probe = family.delta(ActorId(93), bobs_thread, Author::User, 0, family.thread_head(FORK), file("probe"));
    assert_eq!(bob.try_push(&[probe]).await, StatusCode::CONFLICT, "the private thread's deltas count as missing");
    let honest = family.delta(ActorId(93), bobs_thread, Author::User, 0, family.thread_head(MAIN), file("honest"));
    assert_eq!(bob.try_push(&[honest]).await, StatusCode::OK);
}

#[tokio::test]
async fn a_deleted_private_thread_stays_hidden() {
    let (_data, alice, bob) = two_people().await;
    let mut family = Family::new();
    alice.register(MAIN).await;
    alice.push(&[delta(&mut family, MAIN, file("shared"))]).await;
    bob.register(FORK).await;
    let private = silt_proto::VisibilityRequest {
        repo: None,
        thread: Some(FORK),
        visibility: silt_proto::Visibility::Private,
    };
    let set = bob.request(reqwest::Method::PUT, "/v1/visibility").json(&private).send().await.expect("sends");
    assert_eq!(set.status(), StatusCode::NO_CONTENT);
    bob.push(&[delta(&mut family, FORK, file("bobs secret"))]).await;
    assert!(!alice.pulled().await.contains(&FORK));

    assert_eq!(bob.status(reqwest::Method::POST, &format!("/v1/threads/{FORK}/delete")).await, StatusCode::NO_CONTENT);
    assert!(!alice.pulled().await.contains(&FORK), "the family's owner doesn't get the deleted private thread");
}

#[tokio::test]
async fn a_repository_is_not_claimed_by_someone_without_threads_in_it() {
    let (_data, alice, bob) = two_people().await;
    let squat = silt_proto::VisibilityRequest {
        repo: Some("github.com/acme/app".into()),
        thread: None,
        visibility: silt_proto::Visibility::Private,
    };
    let refused = bob.request(reqwest::Method::PUT, "/v1/visibility").json(&squat).send().await.expect("sends");
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);

    let mut family = Family::new();
    alice.register(MAIN).await;
    alice.push(&[delta(&mut family, MAIN, file("a"))]).await;
    assert_eq!(bob.listing().await.threads.len(), 1, "the refused request changed nothing");
}

#[tokio::test]
async fn purging_keeps_a_fork_that_is_not_registered_yet() {
    let (_data, alice, _bob) = two_people().await;
    let mut family = Family::new();
    let shared = delta(&mut family, MAIN, file("shared"));
    let forked = family.delta(ActorId(FORK.0), FORK, Author::User, 0, family.thread_head(MAIN), Op::Fork { from: MAIN });
    family.apply(forked.clone(), &mut HashMap::new()).expect("applies");
    let on_fork = delta(&mut family, FORK, file("fork"));
    alice.register(MAIN).await;
    alice.push(&[shared, forked, on_fork]).await;

    assert_eq!(alice.status(reqwest::Method::POST, &format!("/v1/threads/{MAIN}/delete")).await, StatusCode::NO_CONTENT);
    assert_eq!(alice.status(reqwest::Method::POST, &format!("/v1/threads/{MAIN}/purge")).await, StatusCode::NO_CONTENT);
    assert_eq!(alice.pulled().await, vec![MAIN, FORK, FORK], "the fork's device registers it later; its work is still here");
}

#[tokio::test]
async fn purged_threads_leave_what_forks_need_and_never_come_back() {
    let (_data, alice, bob) = two_people().await;

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
