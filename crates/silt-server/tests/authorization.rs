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
    let register = silt_proto::RegisterThread { thread: info, visibility: Some(Visibility::Private), origin_url: None };
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
