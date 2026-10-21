//! Who may do what (ADR 0013, 0014, 0016): each kind of person against each
//! visibility, over real HTTP.

use std::collections::HashMap;

use reqwest::{Method, StatusCode};
use silt_crdt::{InodeKind, Parent};
use silt_log::{Author, CommentOp, CommentTarget, Content, Delta, Family, Op};
use silt_proto::{
    Listing, LoginReply, OrgChange, PROTOCOL, PROTOCOL_HEADER, PasswordLoginRequest, PresenceUpdate, PullReply, PushRequest,
    ShareRequest, ThreadInfo, Visibility, VisibilityRequest,
};
use silt_server::{Admin, Config, Server};
use silt_store::BlobStore;
use silt_types::{ActorId, FamilyId, ThreadId};

const FAMILY: FamilyId = FamilyId(300);
const THREAD: ThreadId = ThreadId(301);
const REPO: &str = "github.com/acme/app";
const PASSWORD: &str = "correct horse battery";

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

    fn request(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.base))
            .header(PROTOCOL_HEADER, PROTOCOL.to_string())
            .bearer_auth(&self.token)
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> StatusCode {
        request.send().await.expect("sends").status()
    }

    async fn push(&self, deltas: Vec<Delta>) -> StatusCode {
        self.send(self.request(Method::POST, &format!("/v1/families/{FAMILY}/deltas")).json(&PushRequest { deltas })).await
    }

    async fn change(&self, change: OrgChange) -> StatusCode {
        self.send(self.request(Method::POST, "/v1/orgs/change").json(&change)).await
    }

    async fn lists_the_thread(&self) -> bool {
        let listing: Listing =
            self.request(Method::GET, "/v1/listing").send().await.expect("sends").json().await.expect("listing");
        listing.threads.iter().any(|thread| thread.id == THREAD)
    }

    async fn pulls_the_thread(&self) -> bool {
        let response = self.request(Method::GET, &format!("/v1/families/{FAMILY}/deltas?after=0")).send().await.expect("sends");
        if response.status() != StatusCode::OK {
            return false;
        }
        let reply: PullReply = response.json().await.expect("pull");
        reply.deltas.iter().any(|(_, delta)| delta.thread == THREAD)
    }
}

/// Everyone who matters to one thread of Alice's in the `acme` org.
struct World {
    _data: tempfile::TempDir,
    alice: Client,
    people: Vec<(&'static str, Client)>,
    family: Family,
    blob: String,
}

fn file(name: &str, content: Content) -> Op {
    Op::Create { kind: InodeKind::File, parent: Parent::Root, name: name.into(), content, executable: false }
}

/// Alice owns the thread and shares it with Bob. Bob and Carol are in
/// `acme`'s team `core`, Erin is only in `acme`, and Dave is in no org of
/// the project's.
async fn world(visibility: Visibility) -> World {
    let data = tempfile::tempdir().expect("temp dir");
    let admin = Admin::open(data.path()).await.expect("opens");
    for name in ["root", "alice", "bob", "carol", "erin", "dave"] {
        admin.add_user(name, Some(PASSWORD), name == "root").await.expect("adds");
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    tokio::spawn(Server::open_with(data.path(), Config::default()).await.expect("opens").serve(listener));

    let root = Client::sign_in(&base, "root").await;
    let acme = || "acme".to_owned();
    assert_eq!(root.change(OrgChange::Create { org: acme() }).await, StatusCode::NO_CONTENT);
    for user in ["alice", "bob", "carol", "erin"] {
        let add = OrgChange::AddMember { org: acme(), user: user.into(), owner: user == "alice" };
        assert_eq!(root.change(add).await, StatusCode::NO_CONTENT);
    }
    assert_eq!(root.change(OrgChange::CreateTeam { org: acme(), team: "core".into() }).await, StatusCode::NO_CONTENT);
    for user in ["bob", "carol"] {
        let join = OrgChange::AddToTeam { org: acme(), team: "core".into(), user: user.into() };
        assert_eq!(root.change(join).await, StatusCode::NO_CONTENT);
    }

    let alice = Client::sign_in(&base, "alice").await;
    let info = ThreadInfo {
        id: THREAD,
        family: FAMILY,
        repo: REPO.into(),
        created_ms: 1,
        lease: None,
        owner: String::new(),
        collaborators: Vec::new(),
    };
    assert_eq!(alice.send(alice.request(Method::POST, "/v1/threads").json(&info)).await, StatusCode::NO_CONTENT);
    let blob = BlobStore::hash(b"the thread's file");
    let put = alice.request(Method::PUT, &format!("/v1/blobs/{blob}")).body(b"the thread's file".to_vec());
    assert_eq!(alice.send(put).await, StatusCode::NO_CONTENT);
    let mut family = Family::new();
    let first = family.delta(ActorId(1), THREAD, Author::User, 0, family.thread_head(THREAD), file("a.rs", Content::Blob(blob)));
    family.apply(first.clone(), &mut HashMap::new()).expect("applies");
    assert_eq!(alice.push(vec![first]).await, StatusCode::OK);
    assert_eq!(root.change(OrgChange::MoveProject { repo: REPO.into(), org: acme() }).await, StatusCode::NO_CONTENT);

    let share = ShareRequest { user: "bob".into(), remove: false };
    let shared = alice.request(Method::POST, &format!("/v1/threads/{THREAD}/collaborators")).json(&share);
    assert_eq!(alice.send(shared).await, StatusCode::NO_CONTENT);
    let set = VisibilityRequest { repo: None, thread: Some(THREAD), visibility };
    assert_eq!(alice.send(alice.request(Method::PUT, "/v1/visibility").json(&set)).await, StatusCode::NO_CONTENT);

    let mut people = vec![("alice", Client::sign_in(&base, "alice").await)];
    for name in ["bob", "carol", "erin", "dave"] {
        people.push((name, Client::sign_in(&base, name).await));
    }
    World { _data: data, alice, people, family, blob: blob.to_string() }
}

fn sees(person: &str, visibility: &Visibility) -> bool {
    match person {
        "alice" => true,
        "bob" | "carol" => *visibility != Visibility::Private,
        "erin" => *visibility == Visibility::Org,
        _ => false,
    }
}

fn writes(person: &str, visibility: &Visibility) -> bool {
    person == "alice" || (person == "bob" && sees(person, visibility))
}

#[tokio::test]
async fn each_person_sees_and_writes_by_visibility() {
    for visibility in [Visibility::Org, Visibility::Team("core".into()), Visibility::Private] {
        let world = world(visibility.clone()).await;
        let first = world.family.thread_head(THREAD);
        for (n, (person, client)) in world.people.iter().enumerate() {
            let case = format!("{person} with visibility {visibility}");
            let (sees, writes) = (sees(person, &visibility), writes(person, &visibility));
            let found = |status: StatusCode| if sees { status == StatusCode::OK } else { status == StatusCode::NOT_FOUND };

            assert_eq!(client.lists_the_thread().await, sees, "listing: {case}");
            assert_eq!(client.pulls_the_thread().await, sees, "pull: {case}");
            let blob = client.send(client.request(Method::GET, &format!("/v1/blobs/{}", world.blob))).await;
            assert!(found(blob), "blob: {case}: {blob}");
            let watching = client.send(client.request(Method::GET, &format!("/v1/presence?thread={THREAD}"))).await;
            assert!(found(watching), "presence list: {case}: {watching}");
            let here =
                PresenceUpdate { thread: THREAD, file: None, line: None, draft: String::new(), leaving: false, agent: None };
            let present = client.send(client.request(Method::POST, "/v1/presence").json(&here)).await;
            assert!(found(present), "presence update: {case}: {present}");

            let actor = ActorId(100 + n as u128);
            let edit =
                world.family.delta(actor, THREAD, Author::User, 0, first.clone(), file(&format!("{person}.rs"), Content::Empty));
            let pushed = client.push(vec![edit]).await;
            assert_eq!(pushed == StatusCode::OK, writes, "edit: {case}: {pushed}");
            // Without parents nothing needs to be readable, so only the
            // write check stands in the way.
            let rootless = world.family.delta(
                ActorId(300 + n as u128),
                THREAD,
                Author::User,
                0,
                silt_types::Frontier::root(),
                file(&format!("{person}-root.rs"), Content::Empty),
            );
            let pushed = client.push(vec![rootless]).await;
            assert_eq!(pushed == StatusCode::OK, writes, "edit without parents: {case}: {pushed}");
            let comment = world.family.delta(
                ActorId(200 + n as u128),
                THREAD,
                Author::User,
                0,
                first.clone(),
                Op::Comment(CommentOp::Added {
                    target: CommentTarget::Message { delta: first.ids()[0] },
                    text: "hello".into(),
                    by: (*person).into(),
                }),
            );
            let commented = client.push(vec![comment]).await;
            assert_eq!(commented == StatusCode::OK, sees, "comment: {case}: {commented}");

            let owner_only = person == &"alice";
            let unshare = ShareRequest { user: "dave".into(), remove: true };
            let shared =
                client.send(client.request(Method::POST, &format!("/v1/threads/{THREAD}/collaborators")).json(&unshare)).await;
            assert_eq!(shared == StatusCode::NO_CONTENT, owner_only, "share: {case}: {shared}");
            let set = VisibilityRequest { repo: None, thread: Some(THREAD), visibility: visibility.clone() };
            let changed = client.send(client.request(Method::PUT, "/v1/visibility").json(&set)).await;
            assert_eq!(changed == StatusCode::NO_CONTENT, owner_only, "visibility: {case}: {changed}");
            if !owner_only {
                let deleted = client.send(client.request(Method::POST, &format!("/v1/threads/{THREAD}/delete"))).await;
                assert_ne!(deleted, StatusCode::NO_CONTENT, "delete: {case}");
            }
        }
        let deleted = world.alice.send(world.alice.request(Method::POST, &format!("/v1/threads/{THREAD}/delete"))).await;
        assert_eq!(deleted, StatusCode::NO_CONTENT, "the owner deletes");
    }
}

#[tokio::test]
async fn a_collaborator_the_thread_was_hidden_from_stops_writing_it() {
    const SIBLING: ThreadId = ThreadId(302);
    let world = world(Visibility::Private).await;
    let info = ThreadInfo {
        id: SIBLING,
        family: FAMILY,
        repo: REPO.into(),
        created_ms: 2,
        lease: None,
        owner: String::new(),
        collaborators: Vec::new(),
    };
    let alice = &world.alice;
    assert_eq!(alice.send(alice.request(Method::POST, "/v1/threads").json(&info)).await, StatusCode::NO_CONTENT);
    let open =
        world.family.delta(ActorId(2), SIBLING, Author::User, 0, silt_types::Frontier::root(), file("b.rs", Content::Empty));
    assert_eq!(alice.push(vec![open]).await, StatusCode::OK);

    let bob = &world.people.iter().find(|(name, _)| *name == "bob").expect("bob").1;
    assert!(!bob.lists_the_thread().await);
    let into_hidden =
        world.family.delta(ActorId(400), THREAD, Author::User, 0, silt_types::Frontier::root(), file("c.rs", Content::Empty));
    assert_eq!(bob.push(vec![into_hidden]).await, StatusCode::FORBIDDEN, "bob still sees the family, but not this thread");
}

#[tokio::test]
async fn what_someone_sees_follows_each_change_at_once() {
    let world = world(Visibility::Team("core".into())).await;
    let person = |name: &str| &world.people.iter().find(|(person, _)| *person == name).expect("known").1;
    let (alice, carol, erin) = (&world.alice, person("carol"), person("erin"));
    assert!(carol.lists_the_thread().await && !erin.lists_the_thread().await);
    let set = |visibility| VisibilityRequest { repo: None, thread: Some(THREAD), visibility };
    assert_eq!(
        alice.send(alice.request(Method::PUT, "/v1/visibility").json(&set(Visibility::Org))).await,
        StatusCode::NO_CONTENT
    );
    assert!(erin.lists_the_thread().await, "a wider visibility shows at once");
    assert_eq!(
        alice.send(alice.request(Method::PUT, "/v1/visibility").json(&set(Visibility::Private))).await,
        StatusCode::NO_CONTENT
    );
    assert!(!carol.lists_the_thread().await && !erin.lists_the_thread().await, "and a narrower one hides at once");
    assert_eq!(
        alice.send(alice.request(Method::PUT, "/v1/visibility").json(&set(Visibility::Team("core".into())))).await,
        StatusCode::NO_CONTENT
    );
    assert!(carol.lists_the_thread().await);
    let root = Client::sign_in(&alice.base, "root").await;
    let leave = OrgChange::RemoveFromTeam { org: "acme".into(), team: "core".into(), user: "carol".into() };
    assert_eq!(root.change(leave).await, StatusCode::NO_CONTENT);
    assert!(!carol.lists_the_thread().await, "leaving the team hides it at once");
    assert!(!carol.pulls_the_thread().await);
}

#[tokio::test]
async fn a_merge_request_shares_a_private_fork_read_past_before() {
    const FORK: ThreadId = ThreadId(303);
    let mut world = world(Visibility::Org).await;
    let bob = &world.people.iter().find(|(name, _)| *name == "bob").expect("bob").1;
    let alice = &world.alice;
    let info = ThreadInfo {
        id: FORK,
        family: FAMILY,
        repo: REPO.into(),
        created_ms: 3,
        lease: None,
        owner: String::new(),
        collaborators: Vec::new(),
    };
    let register =
        silt_proto::RegisterThread { thread: info, visibility: Some(Visibility::Private), origin_url: None, recovery_of: None };
    assert_eq!(bob.send(bob.request(Method::POST, "/v1/threads").json(&register)).await, StatusCode::NO_CONTENT);
    let family = &mut world.family;
    let head = family.thread_head(THREAD);
    let forked = applied(family, family.delta(ActorId(500), FORK, Author::User, 0, head.clone(), Op::Fork { from: THREAD }));
    let work = applied(
        family,
        family.delta(ActorId(500), FORK, Author::User, 0, forked.span.last().into(), file("quiet.rs", Content::Empty)),
    );
    let comment = Op::Comment(CommentOp::Added {
        target: CommentTarget::Message { delta: head.ids()[0] },
        text: "after the fork".into(),
        by: "bob".into(),
    });
    let comment = applied(family, family.delta(ActorId(501), THREAD, Author::User, 0, head.clone(), comment));
    assert_eq!(bob.push(vec![forked, work.clone(), comment]).await, StatusCode::OK);

    let before = family_info(alice).await;
    assert!(!pulled(alice).await.iter().any(|(_, delta)| delta.thread == FORK), "the fork is private");

    let opened = Op::Mr(silt_log::MrOp::Opened { target: THREAD, title: "Quiet work".into(), summary: String::new() });
    let opened = applied(family, family.delta(ActorId(500), FORK, Author::User, 0, work.span.last().into(), opened));
    assert_eq!(bob.push(vec![opened]).await, StatusCode::OK);
    let after = family_info(alice).await;
    assert_ne!(after.view, before.view, "the request makes the fork's history readable");
    assert!(pulled(alice).await.iter().any(|(_, delta)| delta.span.first() == work.span.first()), "read again from the start");

    // Alice ticks a file of the request as seen; an older mark from another
    // device doesn't untick it, and Dave can't mark a request he can't see.
    let mr = opened_id(&world.family);
    let mark = |seen, at_ms| silt_proto::SeenMark { path: "quiet.rs".into(), hash: "h1".into(), seen, at_ms };
    let put = |client: &Client, mark: silt_proto::SeenMark| client.request(Method::PUT, &format!("/v1/me/seen/{mr}")).json(&mark);
    assert_eq!(alice.send(put(alice, mark(true, 20))).await, StatusCode::NO_CONTENT);
    assert_eq!(alice.send(put(alice, mark(false, 10))).await, StatusCode::NO_CONTENT);
    let seen: Vec<silt_proto::MrSeen> =
        alice.request(Method::GET, "/v1/me/seen").send().await.expect("sends").json().await.expect("seen");
    assert_eq!(seen, [silt_proto::MrSeen { mr, marks: vec![mark(true, 20)] }]);
    let dave = &world.people.iter().find(|(name, _)| *name == "dave").expect("dave").1;
    assert_eq!(dave.send(put(dave, mark(true, 30))).await, StatusCode::NOT_FOUND);
}

fn opened_id(family: &Family) -> silt_types::DeltaId {
    family.merge_requests()[0].id
}

fn applied(family: &mut Family, delta: Delta) -> Delta {
    family.apply(delta.clone(), &mut HashMap::new()).expect("applies");
    delta
}

async fn family_info(client: &Client) -> silt_proto::FamilyInfo {
    let listing: Listing = client.request(Method::GET, "/v1/listing").send().await.expect("sends").json().await.expect("listing");
    listing.families.into_iter().find(|family| family.id == FAMILY).expect("listed")
}

async fn pulled(client: &Client) -> Vec<(u64, Delta)> {
    let reply: PullReply = client
        .request(Method::GET, &format!("/v1/families/{FAMILY}/deltas?after=0"))
        .send()
        .await
        .expect("sends")
        .json()
        .await
        .expect("pull");
    reply.deltas
}

#[tokio::test]
async fn nothing_is_open_without_signing_in() {
    let world = world(Visibility::Org).await;
    let stranger = Client { http: reqwest::Client::new(), base: world.alice.base.clone(), token: "0".repeat(64) };
    let routes = [
        (Method::GET, "/v1/listing".to_owned()),
        (Method::GET, format!("/v1/families/{FAMILY}/deltas?after=0")),
        (Method::POST, format!("/v1/families/{FAMILY}/deltas")),
        (Method::GET, format!("/v1/blobs/{}", world.blob)),
        (Method::GET, "/v1/search?q=hello".to_owned()),
        (Method::GET, format!("/v1/presence?thread={THREAD}")),
        (Method::GET, "/v1/inbox".to_owned()),
        (Method::GET, "/v1/orgs".to_owned()),
        (Method::GET, "/v1/projects".to_owned()),
        (Method::GET, "/v1/me/threads".to_owned()),
        (Method::PATCH, format!("/v1/me/threads/{THREAD}")),
        (Method::GET, "/v1/trash".to_owned()),
        (Method::POST, format!("/v1/threads/{THREAD}/delete")),
        (Method::GET, format!("/git/{REPO}/info/refs?service=git-upload-pack")),
    ];
    for (method, path) in routes {
        assert_eq!(stranger.send(stranger.request(method.clone(), &path)).await, StatusCode::UNAUTHORIZED, "{method} {path}");
    }
}

#[tokio::test]
async fn each_person_keeps_their_own_state_of_a_thread_they_see() {
    use silt_proto::{Stamped, ThreadState};
    let world = world(Visibility::Team("core".into())).await;
    let person = |name: &str| &world.people.iter().find(|(person, _)| *person == name).expect("a person").1;
    let change = |client: &Client, state: &ThreadState| {
        client.request(Method::PATCH, &format!("/v1/me/threads/{THREAD}")).json(state).send()
    };
    let pinned_at = |value, at_ms| ThreadState { pinned: Some(Stamped { value, at_ms }), ..ThreadState::new(THREAD) };

    assert_eq!(change(&world.alice, &pinned_at(true, 20)).await.expect("sends").status(), StatusCode::OK);
    // An older change from another device doesn't undo a newer one.
    let answered: ThreadState = change(&world.alice, &pinned_at(false, 10)).await.expect("sends").json().await.expect("state");
    assert_eq!(answered.pinned, Some(Stamped { value: true, at_ms: 20 }));
    assert_eq!(world.alice.thread_states().await, [pinned_at(true, 20)]);

    // Bob shares the thread but not Alice's state of it; Erin can't see it.
    assert!(person("bob").thread_states().await.is_empty());
    assert_eq!(change(person("bob"), &pinned_at(false, 30)).await.expect("sends").status(), StatusCode::OK);
    assert_eq!(world.alice.thread_states().await, [pinned_at(true, 20)]);
    assert_eq!(change(person("erin"), &pinned_at(true, 40)).await.expect("sends").status(), StatusCode::NOT_FOUND);
    let elsewhere = ThreadState::new(ThreadId(999));
    assert_eq!(change(&world.alice, &elsewhere).await.expect("sends").status(), StatusCode::BAD_REQUEST);
}

/// A browser signed in on the server's page, reading the hosted app's API.
struct Browser {
    http: reqwest::Client,
    base: String,
    cookie: String,
}

impl Browser {
    async fn sign_in(client: &Client, user: &str) -> Self {
        let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().expect("client");
        let signed_in = http
            .post(format!("{}/login", client.base))
            .form(&[("user", user), ("password", PASSWORD), ("next", "/app/")])
            .send()
            .await
            .expect("sends");
        let cookie = signed_in
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .find(|value| value.starts_with("silt_session="))
            .and_then(|value| value.split(';').next())
            .expect("a session cookie")
            .to_owned();
        Self { http, base: client.base.clone(), cookie }
    }

    async fn get(&self, path: &str) -> (StatusCode, serde_json::Value) {
        let response = self
            .http
            .get(format!("{}/app/api{path}", self.base))
            .header(reqwest::header::COOKIE, &self.cookie)
            .send()
            .await
            .expect("sends");
        let status = response.status();
        (status, response.json().await.unwrap_or_default())
    }

    async fn post(&self, path: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
        let response = self
            .http
            .post(format!("{}/app/api{path}", self.base))
            .header(reqwest::header::COOKIE, &self.cookie)
            .header("x-silt-request", "1")
            .json(&body)
            .send()
            .await
            .expect("sends");
        let status = response.status();
        (status, response.json().await.unwrap_or_default())
    }
}

#[tokio::test]
async fn the_hosted_app_directs_an_agent_only_through_its_owners_machine() {
    use silt_proto::{AgentReport, AgentState, AgentsReport, CommandAnswer, ControlReply, RemoteCommand};
    let world = world(Visibility::Team("core".into())).await;
    let people = |name: &str| &world.people.iter().find(|(person, _)| *person == name).expect("a person").1;
    let alice = Browser::sign_in(&world.alice, "alice").await;
    let prompt = serde_json::json!({ "text": "add a test" });
    let (status, refused) = alice.post(&format!("/threads/{THREAD}/agent/prompt"), prompt.clone()).await;
    assert_eq!((status, refused["error"].as_str()), (StatusCode::CONFLICT, Some("no device runs an agent in this thread")));

    let report = AgentsReport {
        agents: vec![AgentReport {
            thread: THREAD,
            agent: "Fake agent".into(),
            state: AgentState::AwaitingPermission,
            permissions: Vec::new(),
            queued: 0,
            preset: "ask_for_everything".into(),
        }],
        startable: Vec::new(),
    };
    let machine = &world.alice;
    assert_eq!(machine.send(machine.request(Method::PUT, "/v1/control/agents").json(&report)).await, StatusCode::NO_CONTENT);
    let (_, live) = alice.get(&format!("/threads/{THREAD}/live")).await;
    assert_eq!(
        (live["agent"]["state"].as_str(), live["preset"].as_str()),
        (Some("awaiting_permission"), Some("ask_for_everything"))
    );

    let bob = Browser::sign_in(people("bob"), "bob").await;
    let (status, refused) = bob.post(&format!("/threads/{THREAD}/agent/prompt"), prompt.clone()).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "bob writes the thread, but the agent runs on alice's machine");
    assert_eq!(refused["error"], "alice's agent runs on their machine; only they can direct it");
    let to_itself = machine
        .request(Method::POST, &format!("/v1/threads/{THREAD}/relay"))
        .json(&silt_proto::RelayRequest { command: RemoteCommand::Cancel, when_back: false });
    assert_eq!(machine.send(to_itself).await, StatusCode::CONFLICT, "a device doesn't relay to itself");

    // Alice's machine takes each command and answers it as it would its own web UI.
    let daemon = async {
        for answer in [
            CommandAnswer { ok: true, message: String::new(), data: "sent".into() },
            CommandAnswer::refused("no such permission request"),
        ] {
            let reply: ControlReply = machine
                .request(Method::GET, "/v1/control?wait_ms=10000")
                .send()
                .await
                .expect("sends")
                .json()
                .await
                .expect("reply");
            let [command] = reply.commands.as_slice() else { panic!("one command at a time: {reply:?}") };
            assert_eq!(command.sender, "alice");
            assert!(command.capabilities.allow(&command.command).is_ok());
            let answered = machine.request(Method::POST, &format!("/v1/control/{}", command.id)).json(&answer);
            assert_eq!(machine.send(answered).await, StatusCode::NO_CONTENT);
            if answer.ok {
                assert_eq!(command.command, RemoteCommand::Prompt { text: "add a test".into(), plan: false });
            }
        }
    };
    let browser = async {
        let sent = alice.post(&format!("/threads/{THREAD}/agent/prompt"), prompt.clone()).await;
        let answered =
            alice.post(&format!("/threads/{THREAD}/agent/answer"), serde_json::json!({ "id": 9, "option": "allow" })).await;
        (sent, answered)
    };
    let ((), (sent, answered)) = tokio::join!(daemon, browser);
    assert_eq!(sent, (StatusCode::OK, serde_json::json!("sent")));
    assert_eq!(answered.0, StatusCode::CONFLICT);
    assert_eq!(answered.1["error"], "no such permission request");
}

#[tokio::test]
async fn people_see_their_devices_what_runs_on_each_and_choose_what_each_may_do() {
    use silt_proto::{AgentReport, AgentState, AgentsReport, Startable};
    let world = world(Visibility::Private).await;
    let machine = &world.alice;
    let report = AgentsReport {
        agents: vec![AgentReport {
            thread: THREAD,
            agent: "Fake agent".into(),
            state: AgentState::Working,
            permissions: Vec::new(),
            queued: 0,
            preset: String::new(),
        }],
        startable: vec![Startable { id: "fake".into(), name: "Fake agent".into() }],
    };
    assert_eq!(machine.send(machine.request(Method::PUT, "/v1/control/agents").json(&report)).await, StatusCode::NO_CONTENT);

    let alice = Browser::sign_in(machine, "alice").await;
    let (status, devices) = alice.get("/devices").await;
    assert_eq!(status, StatusCode::OK, "{devices}");
    let devices = devices.as_array().expect("a list").clone();
    // Two from signing in for the world, and the browser's own sign-in isn't one.
    assert_eq!(devices.len(), 2, "{devices:?}");
    let reporting = devices.iter().find(|device| device["online"] == true).expect("the reporting device is online");
    assert_eq!(reporting["agents"][0]["state"], "working");
    assert_eq!(reporting["agents"][0]["thread"], THREAD.to_string());
    assert_eq!(reporting["capabilities"]["send_prompts"], true, "a new device may do everything");
    assert_eq!(reporting["startable"], serde_json::json!([{ "id": "fake", "name": "Fake agent" }]));
    assert!(devices.iter().all(|device| device["this"] == false), "a browser is none of them");
    let id = reporting["id"].as_str().expect("an id").to_owned();

    let change = |body| {
        alice
            .http
            .patch(format!("{}/app/api/devices/{id}", alice.base))
            .header(reqwest::header::COOKIE, &alice.cookie)
            .header("x-silt-request", "1")
            .json(&body)
            .send()
    };
    let changed: serde_json::Value = change(serde_json::json!({
        "name": "workstation",
        "capabilities": { "answer_permissions": true, "send_prompts": false, "run_agents": false, "review": true }
    }))
    .await
    .expect("sends")
    .json()
    .await
    .expect("json");
    assert_eq!((changed["name"].as_str(), changed["capabilities"]["send_prompts"].as_bool()), (Some("workstation"), Some(false)));
    let refused = change(serde_json::json!({ "name": "  " })).await.expect("sends");
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    let refused: serde_json::Value = refused.json().await.expect("json");
    assert_eq!(refused["error"], "a device's name is 1 to 64 characters", "errors read as the app reads them");

    // A device lists the same, knowing itself.
    let listed: Vec<silt_proto::view::DeviceView> =
        machine.request(Method::GET, "/v1/devices").send().await.expect("sends").json().await.expect("devices");
    assert_eq!(listed.iter().filter(|device| device.this).count(), 1);
    let bob = Browser::sign_in(&world.people.iter().find(|(name, _)| *name == "bob").expect("bob").1, "bob").await;
    assert!(bob.get("/devices").await.1.as_array().expect("a list").iter().all(|device| device["id"] != id.as_str()));
    assert_eq!(
        bob.post(&format!("/devices/{id}/revoke"), serde_json::json!({})).await.0,
        StatusCode::NOT_FOUND,
        "only one's own"
    );

    assert_eq!(alice.post(&format!("/devices/{id}/revoke"), serde_json::json!({})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(alice.get("/devices").await.1.as_array().expect("a list").len(), 1);
}

#[tokio::test]
async fn a_thread_continues_only_on_an_online_device_of_ones_own_once_its_agent_stops_elsewhere() {
    use silt_proto::{AgentReport, AgentState, AgentsReport, ControlReply, RemoteCommand};
    let world = world(Visibility::Team("core".into())).await;
    let alice = Browser::sign_in(&world.alice, "alice").await;
    let devices = alice.get("/devices").await.1.as_array().expect("a list").clone();
    let (workstation, laptop) = (&world.alice, &world.people[0].1);
    let id_of = |index: usize| devices[index]["id"].as_str().expect("an id").to_owned();
    let report = |state| AgentsReport {
        agents: vec![AgentReport {
            thread: THREAD,
            agent: "Fake agent".into(),
            state,
            permissions: Vec::new(),
            queued: 0,
            preset: String::new(),
        }],
        startable: Vec::new(),
    };
    async fn continue_on(alice: &Browser, device: String) -> (StatusCode, serde_json::Value) {
        alice.post(&format!("/threads/{THREAD}/continue"), serde_json::json!({ "device": device, "agent": "fake" })).await
    }
    async fn put(client: &Client, report: AgentsReport) -> StatusCode {
        client.send(client.request(Method::PUT, "/v1/control/agents").json(&report)).await
    }

    let (status, refused) = continue_on(&alice, id_of(0)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(refused["error"].as_str().is_some_and(|error| error.ends_with("is offline")), "{refused}");
    let bob = Browser::sign_in(&world.people.iter().find(|(name, _)| *name == "bob").expect("bob").1, "bob").await;
    let bobs = bob.get("/devices").await.1[0]["id"].as_str().expect("an id").to_owned();
    assert_eq!(continue_on(&alice, bobs).await.0, StatusCode::NOT_FOUND, "only on one's own devices");

    // The workstation runs the agent; the laptop is online with nothing running.
    assert_eq!(put(workstation, report(AgentState::Working)).await, StatusCode::NO_CONTENT);
    assert_eq!(put(laptop, AgentsReport::default()).await, StatusCode::NO_CONTENT);
    let listed: Vec<silt_proto::view::DeviceView> =
        laptop.request(Method::GET, "/v1/devices").send().await.expect("sends").json().await.expect("devices");
    let laptop_id = listed.iter().find(|device| device.this).expect("itself").id.clone();
    let (status, refused) = continue_on(&alice, laptop_id.clone()).await;
    assert_eq!(
        (status, refused["error"].as_str()),
        (StatusCode::CONFLICT, Some("Fake agent runs in this thread elsewhere; stop it there first"))
    );

    assert_eq!(put(workstation, report(AgentState::Stopped)).await, StatusCode::NO_CONTENT);
    let daemon = async {
        let reply: ControlReply =
            laptop.request(Method::GET, "/v1/control?wait_ms=10000").send().await.expect("sends").json().await.expect("reply");
        assert_eq!(reply.commands[0].command, RemoteCommand::Start { agent: "fake".into(), preset: None, mode: None });
        let answer = silt_proto::CommandAnswer { ok: true, message: String::new(), data: THREAD.to_string().into() };
        let answered = laptop.request(Method::POST, &format!("/v1/control/{}", reply.commands[0].id)).json(&answer);
        assert_eq!(laptop.send(answered).await, StatusCode::NO_CONTENT);
    };
    let ((), (status, started)) = tokio::join!(daemon, continue_on(&alice, laptop_id));
    assert_eq!((status, started), (StatusCode::OK, serde_json::json!(THREAD.to_string())));
}

#[tokio::test]
async fn the_hosted_app_comments_and_reviews_as_the_person() {
    use silt_log::{ConvOp, MrStatus, SessionFile, SessionRef};
    const FORK: ThreadId = ThreadId(303);
    let mut world = world(Visibility::Team("core".into())).await;
    let alice = &world.alice;
    let family = &mut world.family;
    let said = applied(
        family,
        family.delta(
            ActorId(1),
            THREAD,
            Author::User,
            5,
            family.thread_head(THREAD),
            Op::Conv(ConvOp::Native {
                session: SessionRef { agent: "claude-code".into(), id: "s".into() },
                file: SessionFile::Main,
                line: r#"{"type":"user","message":{"role":"user","content":"Tidy the parser"}}"#.into(),
            }),
        ),
    );
    assert_eq!(alice.push(vec![said]).await, StatusCode::OK);

    let browser_of = |name: &'static str| {
        let client = &world.people.iter().find(|(person, _)| *person == name).expect("a person").1;
        Browser::sign_in(client, name)
    };
    let (bob, carol, erin) = (browser_of("bob").await, browser_of("carol").await, browser_of("erin").await);
    let comments = format!("/threads/{THREAD}/comments");
    let entry = serde_json::json!({ "on": { "on": "entry", "index": 0 }, "text": "which parser?" });
    let (status, comment) = bob.post(&comments, entry.clone()).await;
    assert_eq!(status, StatusCode::OK, "{comment}");
    assert_eq!((comment["by"].as_str(), comment["location"]["at"].as_str()), (Some("bob"), Some("entry")));
    let id = comment["id"].as_str().expect("an id").to_owned();
    assert_eq!(carol.post(&comments, entry.clone()).await.0, StatusCode::OK, "whoever sees a thread comments on it");
    assert_eq!(erin.post(&comments, entry).await.0, StatusCode::NOT_FOUND);
    let (status, refused) = bob
        .post(&comments, serde_json::json!({ "on": { "on": "lines", "path": "a.rs", "start": 1, "end": 1 }, "text": "here" }))
        .await;
    assert_eq!(
        (status, refused["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("comment on code from a machine with the thread checked out"))
    );

    let me = Browser::sign_in(alice, "alice").await;
    let (_, replied) = me.post(&format!("{comments}/{id}/replies"), serde_json::json!({ "text": "the toml one" })).await;
    assert_eq!(
        (replied["replies"][0]["by"].as_str(), replied["replies"][0]["text"].as_str()),
        (Some("alice"), Some("the toml one"))
    );
    let (_, resolved) = me.post(&format!("{comments}/{id}/resolve"), serde_json::json!({})).await;
    assert_eq!(resolved["resolved"], true);

    // Bob offers a fork back; Alice asks for changes, then merges it.
    let bobs = &world.people.iter().find(|(name, _)| *name == "bob").expect("bob").1;
    let info = ThreadInfo {
        id: FORK,
        family: FAMILY,
        repo: REPO.into(),
        created_ms: 3,
        lease: None,
        owner: String::new(),
        collaborators: Vec::new(),
    };
    assert_eq!(bobs.send(bobs.request(Method::POST, "/v1/threads").json(&info)).await, StatusCode::NO_CONTENT);
    let pulled_now: Vec<Delta> = pulled(alice).await.into_iter().map(|(_, delta)| delta).collect();
    let mut replica = Family::new();
    for delta in &pulled_now {
        replica.apply(delta.clone(), &mut HashMap::new()).expect("what the server wrote applies");
    }
    let replica = &mut replica;
    let head = replica.thread_head(THREAD);
    let forked = applied(replica, replica.delta(ActorId(500), FORK, Author::User, 0, head, Op::Fork { from: THREAD }));
    let work = applied(
        replica,
        replica.delta(ActorId(500), FORK, Author::User, 0, forked.span.last().into(), file("quiet.rs", Content::Empty)),
    );
    let opened = Op::Mr(silt_log::MrOp::Opened { target: THREAD, title: "Quiet work".into(), summary: "a quiet file".into() });
    let opened = applied(replica, replica.delta(ActorId(500), FORK, Author::User, 0, work.span.last().into(), opened));
    assert_eq!(bobs.push(vec![forked, work, opened]).await, StatusCode::OK);
    let mr = opened_id(replica).to_string();

    let (status, asked) = me.post(&format!("/mrs/{mr}/request-changes"), serde_json::json!({ "reason": "add a test" })).await;
    assert_eq!((status, asked["changes_requested"].as_str()), (StatusCode::OK, Some("add a test")), "{asked}");
    let (status, approved) = me.post(&format!("/mrs/{mr}/approve"), serde_json::json!({})).await;
    assert_eq!((status, approved["status"].as_str()), (StatusCode::OK, Some("approved")), "{approved}");
    assert_eq!(me.post(&format!("/mrs/{mr}/decline"), serde_json::json!({})).await.0, StatusCode::BAD_REQUEST, "once merged");

    let mut replayed = Family::new();
    for (_, delta) in pulled(alice).await {
        replayed.apply(delta, &mut HashMap::new()).expect("the merge applies as a daemon's would");
    }
    assert_eq!(replayed.merge_requests()[0].status, MrStatus::Approved);
    let comment = &replayed.comments(THREAD)[0];
    assert!(comment.resolved && comment.replies.len() == 1);
    let merged = replayed.log().iter().any(|delta| matches!(delta.op, Op::Merge { from: FORK, .. }) && delta.thread == THREAD);
    assert!(merged, "the fork's work is merged into the thread");
}

#[tokio::test]
async fn what_a_device_reports_reaches_its_persons_inbox_with_what_to_do() {
    use silt_proto::{AgentReport, AgentState, AgentsReport, AskedPermission};
    let world = world(Visibility::Private).await;
    let machine = &world.alice;
    let report = |state, asked: Option<u64>| AgentsReport {
        agents: vec![AgentReport {
            thread: THREAD,
            agent: "Fake agent".into(),
            state,
            permissions: asked
                .map(|id| AskedPermission { id, title: "Edit a.rs".into(), options: vec![("allow".into(), "Allow".into())] })
                .into_iter()
                .collect(),
            queued: 0,
            preset: String::new(),
        }],
        startable: Vec::new(),
    };
    async fn put(client: &Client, report: AgentsReport) {
        assert_eq!(client.send(client.request(Method::PUT, "/v1/control/agents").json(&report)).await, StatusCode::NO_CONTENT);
    }
    let alice = Browser::sign_in(machine, "alice").await;
    let inbox = || async { alice.get("/inbox").await.1.as_array().expect("a list").clone() };

    // What runs already when the server first hears of the device is old news.
    put(machine, report(AgentState::AwaitingPermission, Some(1))).await;
    assert!(inbox().await.is_empty(), "{:?}", inbox().await);

    put(machine, report(AgentState::Working, None)).await;
    put(machine, report(AgentState::AwaitingPermission, Some(2))).await;
    put(machine, report(AgentState::AwaitingPermission, Some(3))).await;
    let notes = inbox().await;
    assert_eq!(notes.len(), 1, "an unread one is brought up to date: {notes:?}");
    assert_eq!(notes[0]["kind"], "permission");
    assert!(
        notes[0]["text"]
            .as_str()
            .is_some_and(|text| text.starts_with("Fake agent on ") && text.ends_with("asks to run: Edit a.rs"))
    );
    assert_eq!(
        notes[0]["action"],
        serde_json::json!({ "do": "permission", "id": 3, "title": "Edit a.rs", "options": [["allow", "Allow"]] })
    );

    let id = notes[0]["id"].as_i64().expect("an id");
    assert_eq!(alice.post("/inbox/read", serde_json::json!({ "ids": [id] })).await.0, StatusCode::OK);
    put(machine, report(AgentState::Idle, None)).await;
    let notes = inbox().await;
    assert_eq!(notes.len(), 1);
    assert_eq!((notes[0]["kind"].as_str(), &notes[0]["action"]), (Some("finished"), &serde_json::Value::Null));
    let all = alice.get("/inbox?all=true").await.1;
    let read = all.as_array().expect("a list").iter().find(|note| note["kind"] == "permission").expect("the read one");
    assert_eq!(read["action"], serde_json::Value::Null, "a read notification offers nothing to do any more");

    let other = &world.people.iter().find(|(name, _)| *name == "bob").expect("bob").1;
    let bob = Browser::sign_in(other, "bob").await;
    assert!(bob.get("/inbox").await.1.as_array().expect("a list").is_empty(), "only the device's person hears");
}

#[tokio::test]
async fn each_person_chooses_which_notifications_notify_them() {
    let world = world(Visibility::Org).await;
    let alice = &world.alice;
    let settings: Vec<silt_proto::KindSetting> =
        alice.request(Method::GET, "/v1/me/notifications").send().await.expect("sends").json().await.expect("settings");
    let notifies = |settings: &[silt_proto::KindSetting], kind: &str| {
        settings.iter().find(|setting| setting.kind == kind).map(|setting| setting.notify)
    };
    assert_eq!((notifies(&settings, "permission"), notifies(&settings, "comment")), (Some(true), Some(false)), "the defaults");

    let change = |kind: &str, notify: bool| {
        alice.request(Method::PUT, "/v1/me/notifications").json(&silt_proto::KindSetting { kind: kind.into(), notify })
    };
    let changed: Vec<silt_proto::KindSetting> =
        change("comment", true).send().await.expect("sends").json().await.expect("settings");
    assert_eq!(notifies(&changed, "comment"), Some(true));
    assert_eq!(alice.send(change("gossip", true)).await, StatusCode::BAD_REQUEST);

    // Bob's comment on Alice's thread now notifies her; Bob's own settings stay his.
    let bob = &world.people.iter().find(|(name, _)| *name == "bob").expect("bob").1;
    let head = world.family.thread_head(THREAD);
    let comment = Op::Comment(CommentOp::Added {
        target: CommentTarget::Message { delta: head.ids()[0] },
        text: "nice".into(),
        by: "bob".into(),
    });
    assert_eq!(bob.push(vec![world.family.delta(ActorId(600), THREAD, Author::User, 0, head, comment)]).await, StatusCode::OK);
    let inbox: silt_proto::InboxReply =
        alice.request(Method::GET, "/v1/inbox").send().await.expect("sends").json().await.expect("inbox");
    assert!(inbox.notifications.iter().any(|note| note.kind == "comment" && note.notify), "{inbox:?}");
    let bobs: Vec<silt_proto::KindSetting> =
        bob.request(Method::GET, "/v1/me/notifications").send().await.expect("sends").json().await.expect("settings");
    assert_eq!(notifies(&bobs, "comment"), Some(false));
}

#[tokio::test]
async fn notifying_notifications_are_pushed_encrypted_to_the_persons_browsers() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use silt_proto::{AgentReport, AgentState, AgentsReport, AskedPermission};
    use web_push_native::p256::SecretKey;

    // A push service standing in for a browser vendor's: it takes two pushes,
    // then says the browser went away.
    let (pushed, mut received) = tokio::sync::mpsc::unbounded_channel::<(axum::http::HeaderMap, Vec<u8>)>();
    let count = Arc::new(AtomicUsize::new(0));
    let service = axum::Router::new().route(
        "/push/browser",
        axum::routing::post(move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
            let (pushed, count) = (pushed.clone(), count.clone());
            async move {
                let _ = pushed.send((headers, body.to_vec()));
                if count.fetch_add(1, Ordering::SeqCst) == 0 { StatusCode::CREATED } else { StatusCode::GONE }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let endpoint = format!("http://{}/push/browser", listener.local_addr().expect("address"));
    tokio::spawn(async move { axum::serve(listener, service).await });

    let world = world(Visibility::Private).await;
    let machine = &world.alice;
    let alice = Browser::sign_in(machine, "alice").await;
    let (_, key) = alice.get("/push/key").await;
    assert_eq!(URL_SAFE_NO_PAD.decode(key.as_str().expect("a key")).expect("base64url").len(), 65, "an uncompressed P-256 key");

    let browser_secret = SecretKey::from_slice(&[7; 32]).expect("a key");
    let auth = [1u8; 16];
    let subscription = serde_json::json!({
        "endpoint": endpoint,
        "keys": {
            "p256dh": URL_SAFE_NO_PAD.encode(browser_secret.public_key().to_sec1_bytes()),
            "auth": URL_SAFE_NO_PAD.encode(auth),
        }
    });
    assert_eq!(alice.post("/push/subscriptions", subscription).await.0, StatusCode::NO_CONTENT);

    let report = |state, asked: Option<u64>| AgentsReport {
        agents: vec![AgentReport {
            thread: THREAD,
            agent: "Fake agent".into(),
            state,
            permissions: asked
                .map(|id| AskedPermission { id, title: "Edit a.rs".into(), options: vec![("allow".into(), "Allow".into())] })
                .into_iter()
                .collect(),
            queued: 0,
            preset: String::new(),
        }],
        startable: Vec::new(),
    };
    let put = |report: AgentsReport| machine.send(machine.request(Method::PUT, "/v1/control/agents").json(&report));
    assert_eq!(put(report(AgentState::Working, None)).await, StatusCode::NO_CONTENT);
    assert_eq!(put(report(AgentState::AwaitingPermission, Some(1))).await, StatusCode::NO_CONTENT);
    let (headers, body) =
        tokio::time::timeout(std::time::Duration::from_secs(10), received.recv()).await.expect("pushed").expect("a push");
    assert_eq!(headers["content-encoding"], "aes128gcm");
    assert!(headers["authorization"].to_str().expect("ascii").starts_with("vapid t="), "{headers:?}");
    let payload = web_push_native::decrypt(body, &browser_secret, &auth.into()).expect("only the browser reads it");
    let payload: serde_json::Value = serde_json::from_slice(&payload).expect("json");
    assert_eq!(payload["title"], "An agent asks for a permission");
    assert!(payload["body"].as_str().is_some_and(|body| body.ends_with("asks to run: Edit a.rs")));
    assert_eq!(payload["url"], format!("t/{THREAD}"));

    // A kind that only lists isn't pushed; one that notifies is, until the
    // push service says the browser is gone.
    assert_eq!(put(report(AgentState::Idle, None)).await, StatusCode::NO_CONTENT);
    assert_eq!(put(report(AgentState::Error, None)).await, StatusCode::NO_CONTENT);
    let (_, body) =
        tokio::time::timeout(std::time::Duration::from_secs(10), received.recv()).await.expect("pushed").expect("a push");
    let payload = web_push_native::decrypt(body, &browser_secret, &auth.into()).expect("decrypts");
    assert!(String::from_utf8_lossy(&payload).contains("An agent stopped with an error"), "finishing didn't push");
    // The server drops the subscription once it reads the answer, in the
    // background, so a push or two may still go out meanwhile.
    let mut dropped = false;
    for asked in 2..12 {
        assert_eq!(put(report(AgentState::Working, None)).await, StatusCode::NO_CONTENT);
        assert_eq!(put(report(AgentState::AwaitingPermission, Some(asked))).await, StatusCode::NO_CONTENT);
        if tokio::time::timeout(std::time::Duration::from_millis(500), received.recv()).await.is_err() {
            dropped = true;
            break;
        }
    }
    assert!(dropped, "the gone browser's subscription was dropped");
}

#[tokio::test]
async fn changes_from_the_hosted_app_need_its_header() {
    let world = world(Visibility::Org).await;
    let alice = Browser::sign_in(&world.alice, "alice").await;
    let unmarked = alice
        .http
        .post(format!("{}/app/api/threads/{THREAD}/agent/stop", alice.base))
        .header(reqwest::header::COOKIE, &alice.cookie)
        .send()
        .await
        .expect("sends");
    assert_eq!(unmarked.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_hosted_app_reads_what_each_person_sees() {
    use silt_log::{ConvOp, SessionFile, SessionRef};
    let world = world(Visibility::Team("core".into())).await;
    let alice = &world.alice;
    let mut family = world.family.clone();
    let mut add = |op| {
        let delta = family.delta(ActorId(1), THREAD, Author::User, 5, family.thread_head(THREAD), op);
        family.apply(delta.clone(), &mut HashMap::new()).expect("applies");
        delta
    };
    let said = add(Op::Conv(ConvOp::Native {
        session: SessionRef { agent: "claude-code".into(), id: "s".into() },
        file: SessionFile::Main,
        line: r#"{"type":"user","message":{"role":"user","content":"Fix the tokenizer"}}"#.into(),
    }));
    let titled = add(Op::Conv(ConvOp::Title("Tokenizer".into())));
    let commented = add(Op::Comment(CommentOp::Added {
        target: CommentTarget::Message { delta: said.span.first() },
        text: "which tokenizer?".into(),
        by: "bob".into(),
    }));
    assert_eq!(alice.push(vec![said, titled, commented]).await, StatusCode::OK);

    let people = |name: &str| &world.people.iter().find(|(person, _)| *person == name).expect("a person").1;
    let bob = Browser::sign_in(people("bob"), "bob").await;
    let (status, threads) = bob.get("/threads").await;
    assert_eq!(status, StatusCode::OK, "{threads}");
    let thread = &threads[0];
    assert_eq!(thread["id"], THREAD.to_string());
    assert_eq!(thread["title"], "Tokenizer");
    assert_eq!(thread["owner"], "alice");
    assert_eq!(thread["messages"], 1);
    assert_eq!(thread["writable"], true, "bob collaborates");
    assert_eq!(thread["location"], "remote");

    let (_, history) = bob.get(&format!("/threads/{THREAD}/history")).await;
    assert_eq!(history["total"], 1);
    assert_eq!(history["entries"][0]["text"], "Fix the tokenizer");
    let (_, comments) = bob.get(&format!("/threads/{THREAD}/comments")).await;
    assert_eq!(comments[0]["text"], "which tokenizer?");
    assert_eq!(comments[0]["location"], serde_json::json!({ "at": "entry", "index": 0, "excerpt": "Fix the tokenizer" }));

    let erin = Browser::sign_in(people("erin"), "erin").await;
    let (_, threads) = erin.get("/threads").await;
    assert_eq!(threads, serde_json::json!([]), "erin isn't in the team");
    assert_eq!(erin.get(&format!("/threads/{THREAD}/history")).await.0, StatusCode::NOT_FOUND);

    let pin = |browser: &Browser| {
        browser
            .http
            .patch(format!("{}/app/api/threads/{THREAD}/state", browser.base))
            .header(reqwest::header::COOKIE, &browser.cookie)
            .header("x-silt-request", "1")
            .json(&serde_json::json!({ "pinned": true }))
            .send()
    };
    let mine: serde_json::Value = pin(&bob).await.expect("sends").json().await.expect("json");
    assert_eq!(mine["pinned"], true);
    assert_eq!(bob.get("/threads").await.1[0]["mine"]["pinned"], true);
    assert_eq!(people("bob").thread_states().await.len(), 1, "the same state bob's daemons sync");
    assert_eq!(pin(&erin).await.expect("sends").status(), StatusCode::NOT_FOUND);
}

impl Client {
    async fn thread_states(&self) -> Vec<silt_proto::ThreadState> {
        self.request(Method::GET, "/v1/me/threads").send().await.expect("sends").json().await.expect("states")
    }

    async fn projects(&self) -> Vec<silt_proto::ProjectInfo> {
        let reply: silt_proto::ProjectsReply =
            self.request(Method::GET, "/v1/projects").send().await.expect("sends").json().await.expect("projects");
        reply.projects
    }
}

#[tokio::test]
async fn org_members_list_their_projects_and_owners_set_them_up() {
    let world = world(Visibility::Private).await;
    let person = |name: &str| &world.people.iter().find(|(person, _)| *person == name).expect("a person").1;
    let info = ThreadInfo {
        id: THREAD,
        family: FAMILY,
        repo: REPO.into(),
        created_ms: 1,
        lease: None,
        owner: String::new(),
        collaborators: Vec::new(),
    };
    let register = silt_proto::RegisterThread {
        thread: info,
        visibility: None,
        origin_url: Some("https://alice:ghp_secret@github.com/acme/app.git".into()),
        recovery_of: None,
    };
    assert_eq!(world.alice.send(world.alice.request(Method::POST, "/v1/threads").json(&register)).await, StatusCode::NO_CONTENT);

    let listed = world.alice.projects().await;
    assert_eq!(listed.len(), 1);
    let project = &listed[0];
    assert_eq!((project.repo.as_str(), project.name.as_str(), project.org.as_deref()), (REPO, "app", Some("acme")));
    assert_eq!(project.origin_url.as_deref(), Some("https://github.com/acme/app.git"));
    assert_eq!(project.threads, 1);
    // Erin is in the org but can't see Alice's private thread; Dave isn't in it.
    assert_eq!(person("erin").projects().await[0].threads, 0);
    assert!(person("dave").projects().await.is_empty());

    let settings = |name: &str, preset: &str| OrgChange::ProjectSettings {
        repo: REPO.into(),
        name: Some(name.into()),
        default_agent: Some("claude-acp".into()),
        default_preset: Some(preset.into()),
    };
    assert_eq!(person("erin").change(settings("Mine now", "full_access")).await, StatusCode::FORBIDDEN);
    assert_eq!(world.alice.change(settings("App", "no way!")).await, StatusCode::BAD_REQUEST);
    assert_eq!(world.alice.change(settings("App", "ask_for_everything")).await, StatusCode::NO_CONTENT);
    let project = &person("erin").projects().await[0];
    assert_eq!(project.name, "App");
    assert_eq!(
        (project.default_agent.as_deref(), project.default_preset.as_deref()),
        (Some("claude-acp"), Some("ask_for_everything"))
    );

    let cleared =
        OrgChange::ProjectSettings { repo: REPO.into(), name: Some(String::new()), default_agent: None, default_preset: None };
    assert_eq!(world.alice.change(cleared).await, StatusCode::NO_CONTENT);
    let project = &world.alice.projects().await[0];
    assert_eq!((project.name.as_str(), project.default_agent.as_deref()), ("app", Some("claude-acp")));
    let elsewhere = OrgChange::ProjectSettings {
        repo: "github.com/nobody/else".into(),
        name: None,
        default_agent: None,
        default_preset: None,
    };
    assert_eq!(world.alice.change(elsewhere).await, StatusCode::NOT_FOUND);
}
