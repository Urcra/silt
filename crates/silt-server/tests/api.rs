use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use silt_crdt::{InodeKind, Parent};
use silt_log::{Author, Content, Delta, Family, Op};
use silt_proto::{
    ErrorReply, GENERATION_HEADER, Lease, Listing, LoginReply, LoginRequest, PROTOCOL, PROTOCOL_HEADER, PullReply, PushReply,
    PushRequest, ThreadInfo, Visibility, VisibilityRequest,
};
use silt_server::{Server, create_pairing_code};
use silt_store::BlobStore;
use silt_types::{ActorId, DeviceId, FamilyId, ThreadId};

const FAMILY: FamilyId = FamilyId(5);
const THREAD: ThreadId = ThreadId(6);

async fn start(data: &Path) -> String {
    let server = Server::open(data).await.expect("opens");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let address = listener.local_addr().expect("address");
    tokio::spawn(server.serve(listener));
    format!("http://{address}")
}

struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl Client {
    async fn login(data: &Path, base: &str) -> (Self, DeviceId) {
        Self::login_as(data, base, "owner").await
    }

    async fn login_as(data: &Path, base: &str, user: &str) -> (Self, DeviceId) {
        let code = create_pairing_code(data, user).await.expect("code");
        let http = reqwest::Client::new();
        let reply: LoginReply = http
            .post(format!("{base}/v1/login"))
            .header(PROTOCOL_HEADER, PROTOCOL.to_string())
            .json(&LoginRequest { code, name: "laptop".into() })
            .send()
            .await
            .expect("sends")
            .json()
            .await
            .expect("login reply");
        assert_eq!(reply.user, user);
        (Self { http, base: base.to_owned(), token: reply.token }, reply.device)
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.base))
            .header(PROTOCOL_HEADER, PROTOCOL.to_string())
            .bearer_auth(&self.token)
    }

    async fn push(&self, deltas: Vec<Delta>) -> reqwest::Response {
        self.request(reqwest::Method::POST, &format!("/v1/families/{FAMILY}/deltas"))
            .json(&PushRequest { deltas })
            .send()
            .await
            .expect("sends")
    }

    async fn pull(&self, query: &str) -> reqwest::Response {
        self.request(reqwest::Method::GET, &format!("/v1/families/{FAMILY}/deltas?{query}")).send().await.expect("sends")
    }
}

fn deltas(count: usize) -> Vec<Delta> {
    let mut family = Family::new();
    (0..count)
        .map(|index| {
            let delta = family.delta(
                ActorId(1),
                THREAD,
                Author::User,
                0,
                family.thread_head(THREAD),
                Op::Create {
                    kind: InodeKind::File,
                    parent: Parent::Root,
                    name: format!("f{index}"),
                    content: Content::Empty,
                    executable: false,
                },
            );
            family.apply(delta.clone(), &mut HashMap::new()).expect("applies");
            delta
        })
        .collect()
}

#[tokio::test]
async fn rejects_other_protocol_versions_and_unknown_devices() {
    let data = tempfile::tempdir().expect("temp dir");
    let base = start(data.path()).await;
    let http = reqwest::Client::new();
    let response = http.get(format!("{base}/v1/listing")).send().await.expect("sends");
    assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
    assert!(response.headers().contains_key(GENERATION_HEADER));
    let reply: ErrorReply = response.json().await.expect("error body");
    assert_eq!(reply, ErrorReply::ProtocolMismatch { supported: vec![PROTOCOL] });

    let response = http
        .get(format!("{base}/v1/listing"))
        .header(PROTOCOL_HEADER, PROTOCOL.to_string())
        .bearer_auth("not-a-token")
        .send()
        .await
        .expect("sends");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = http
        .post(format!("{base}/v1/login"))
        .header(PROTOCOL_HEADER, PROTOCOL.to_string())
        .json(&LoginRequest { code: "wrong".into(), name: "x".into() })
        .send()
        .await
        .expect("sends");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn pushes_dedupe_and_pulls_follow_and_wait() {
    let data = tempfile::tempdir().expect("temp dir");
    let base = start(data.path()).await;
    let (client, _) = Client::login(data.path(), &base).await;
    let all = deltas(3);

    let reply: PushReply = client.push(all[..2].to_vec()).await.json().await.expect("push reply");
    assert_eq!((reply.accepted, reply.seq), (2, 2));
    let reply: PushReply = client.push(all.clone()).await.json().await.expect("push reply");
    assert_eq!((reply.accepted, reply.seq), (1, 3), "known deltas are skipped");

    let pulled: PullReply = client.pull("after=0").await.json().await.expect("pull reply");
    assert_eq!(pulled.deltas.len(), 3);
    let last = pulled.deltas[2].1.span.first();
    let response = client.pull(&format!("after=3&last={}", all[0].span.first())).await;
    assert_eq!(response.status(), StatusCode::CONFLICT, "a wrong fingerprint is refused");

    let waiting = {
        let client = Client { http: client.http.clone(), base: client.base.clone(), token: client.token.clone() };
        tokio::spawn(async move {
            let started = Instant::now();
            let reply: PullReply = client.pull(&format!("after=3&last={last}&wait=10")).await.json().await.expect("pull");
            (started.elapsed(), reply.deltas.len())
        })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut more = Family::new();
    for delta in &all {
        more.apply(delta.clone(), &mut HashMap::new()).expect("replays");
    }
    let next = more.delta(
        ActorId(1),
        THREAD,
        Author::User,
        0,
        more.thread_head(THREAD),
        Op::Delete { inode: silt_crdt::InodeId(all[0].span.first()) },
    );
    client.push(vec![next]).await;
    let (elapsed, count) = waiting.await.expect("joins");
    assert_eq!(count, 1);
    assert!(elapsed < Duration::from_secs(5), "the long poll woke up on the push");
}

#[tokio::test]
async fn data_survives_a_restart_under_a_new_generation() {
    let data = tempfile::tempdir().expect("temp dir");
    let base = start(data.path()).await;
    let (client, _) = Client::login(data.path(), &base).await;
    let first = client.push(deltas(2)).await;
    let generation = first.headers()[GENERATION_HEADER].clone();

    let base = start(data.path()).await;
    let client = Client { base, ..client };
    let response = client.pull("after=0").await;
    assert_ne!(response.headers()[GENERATION_HEADER], generation);
    let pulled: PullReply = response.json().await.expect("pull reply");
    assert_eq!(pulled.deltas.len(), 2);
}

#[tokio::test]
async fn a_server_opening_waits_for_another_writer_to_the_same_file() {
    let data = tempfile::tempdir().expect("temp dir");
    start(data.path()).await;
    let url = format!("sqlite://{}", data.path().join("server.sqlite").display());
    let mut other = <sqlx::SqliteConnection as sqlx::Connection>::connect(&url).await.expect("connects");
    sqlx::query("BEGIN IMMEDIATE").execute(&mut other).await.expect("takes the write lock");
    let release = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        sqlx::query("COMMIT").execute(&mut other).await.expect("commits");
    });

    Server::open(data.path()).await.expect("opens once the other writer is done");
    release.await.expect("released");
}

#[tokio::test]
async fn a_stored_delta_that_no_longer_applies_does_not_block_pushes() {
    let data = tempfile::tempdir().expect("temp dir");
    let base = start(data.path()).await;
    let (client, _) = Client::login(data.path(), &base).await;
    let chain = deltas(3);
    assert_eq!(client.push(chain[..1].to_vec()).await.status(), StatusCode::OK);

    let family = Family::new();
    let orphan = family.delta(
        ActorId(5),
        THREAD,
        Author::User,
        0,
        silt_types::Frontier::single(silt_types::DeltaId::new(ActorId(99), silt_types::Lamport(0))),
        Op::Create {
            kind: InodeKind::File,
            parent: Parent::Root,
            name: "lost".into(),
            content: Content::Empty,
            executable: false,
        },
    );
    let db =
        sqlx::SqlitePool::connect(&format!("sqlite://{}", data.path().join("server.sqlite").display())).await.expect("opens");
    sqlx::query("INSERT INTO deltas (family_id, seq, data) VALUES (?, 2, ?)")
        .bind(FAMILY.0.to_be_bytes().to_vec())
        .bind(postcard::to_allocvec(&orphan).expect("encodes"))
        .execute(&db)
        .await
        .expect("inserts");
    db.close().await;

    let client = Client { base: start(data.path()).await, ..client };
    assert_eq!(client.push(chain[1..].to_vec()).await.status(), StatusCode::OK, "new deltas are numbered after the skipped one");
    let client = Client { base: start(data.path()).await, ..client };
    let pulled: PullReply = client.pull("after=0").await.json().await.expect("pull reply");
    assert_eq!(pulled.deltas.len(), 3);
}

#[tokio::test]
async fn routes_without_an_account_take_only_small_bodies() {
    let data = tempfile::tempdir().expect("temp dir");
    let base = start(data.path()).await;
    // One byte over the server's 1 MiB: it reads the whole body before
    // refusing it, as it stops at the limit. A bigger body leaves bytes
    // unread when it closes, which Windows answers with a reset the client
    // can see before the 413.
    let huge = vec![b'x'; (1 << 20) + 1];
    let response = reqwest::Client::new()
        .post(format!("{base}/v1/login/password"))
        .header(silt_proto::PROTOCOL_HEADER, silt_proto::PROTOCOL.to_string())
        .header("content-type", "application/json")
        .body(huge)
        .send()
        .await
        .expect("sends");
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

/// A large blob goes up in chunks, in order, going on from what arrived
/// after a dropped connection, is checked whole, and is read by range
/// (ADR 0121).
#[tokio::test]
async fn a_large_blob_goes_up_in_chunks_and_is_read_by_range() {
    let data = tempfile::tempdir().expect("temp dir");
    let base = start(data.path()).await;
    let (client, _) = Client::login(data.path(), &base).await;
    let bytes: Vec<u8> = (0..30_000u32).map(|n| (n % 251) as u8).collect();
    let hash = BlobStore::hash(&bytes);
    let size = bytes.len();
    let chunk = |offset: usize, end: usize, size: usize| {
        client
            .request(reqwest::Method::PUT, &format!("/v1/blobs/{hash}/chunks?offset={offset}&size={size}"))
            .body(bytes[offset..end].to_vec())
            .send()
    };
    let state = || async {
        let answer = client.request(reqwest::Method::GET, &format!("/v1/blobs/{hash}/state")).send().await.expect("sends");
        answer.json::<silt_proto::BlobState>().await.expect("a state")
    };
    let first = chunk(0, 12_000, size).await.expect("sends");
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(state().await, silt_proto::BlobState { stored: false, received: 12_000, size: None, ..Default::default() });
    assert_eq!(chunk(0, 12_000, size).await.expect("sends").status(), StatusCode::CONFLICT, "only where it left off");
    assert_eq!(chunk(12_000, 30_000, size - 1).await.expect("sends").status(), StatusCode::BAD_REQUEST, "past its size");
    let last: silt_proto::BlobState = chunk(12_000, 30_000, size).await.expect("sends").json().await.expect("state");
    assert!(last.stored, "{last:?}");

    let family = Family::new();
    let bundle = family.delta(
        ActorId(9),
        THREAD,
        Author::User,
        0,
        family.thread_head(THREAD),
        Op::Git(silt_log::GitOp::Bundle { commits: Vec::new(), blob: hash }),
    );
    assert_eq!(client.push(vec![bundle]).await.status(), StatusCode::OK);
    assert_eq!(
        state().await,
        silt_proto::BlobState { stored: true, received: size as u64, size: Some(size as u64), ..Default::default() }
    );
    let part = client
        .request(reqwest::Method::GET, &format!("/v1/blobs/{hash}"))
        .header(reqwest::header::RANGE, "bytes=29990-")
        .send()
        .await
        .expect("sends");
    assert_eq!(part.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(part.headers()[reqwest::header::CONTENT_RANGE], "bytes 29990-29999/30000");
    assert_eq!(&part.bytes().await.expect("body")[..], &bytes[29_990..]);

    let other = BlobStore::hash(b"what was promised");
    let damaged = client
        .request(reqwest::Method::PUT, &format!("/v1/blobs/{other}/chunks?offset=0&size=14"))
        .body(b"something else".to_vec())
        .send()
        .await
        .expect("sends");
    assert_eq!(damaged.status(), StatusCode::UNPROCESSABLE_ENTITY);
    // Whoever may read it hears that it arrived damaged and comes again.
    let names = |blob| {
        let family = Family::new();
        family.delta(
            ActorId(10),
            THREAD,
            Author::User,
            0,
            family.thread_head(THREAD),
            Op::Git(silt_log::GitOp::Bundle { commits: Vec::new(), blob }),
        )
    };
    assert_eq!(client.push(vec![names(other)]).await.status(), StatusCode::OK);
    let told: silt_proto::BlobState = client
        .request(reqwest::Method::GET, &format!("/v1/blobs/{other}/state"))
        .send()
        .await
        .expect("sends")
        .json()
        .await
        .expect("a state");
    assert!(told.damaged && !told.stored, "{told:?}");
}

/// A server's owner can cap the files it keeps; it keeps any size by default.
#[tokio::test]
async fn a_server_keeps_files_up_to_its_cap() {
    let data = tempfile::tempdir().expect("temp dir");
    let config = silt_server::Config { max_file_mib: Some(1), ..silt_server::Config::default() };
    let server = Server::open_with(data.path(), config).await.expect("opens");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    tokio::spawn(server.serve(listener));
    let (client, _) = Client::login(data.path(), &base).await;
    let large = vec![1u8; (1 << 20) + 1];
    let hash = BlobStore::hash(&large);
    let whole =
        client.request(reqwest::Method::PUT, &format!("/v1/blobs/{hash}")).body(large.clone()).send().await.expect("sends");
    assert_eq!(whole.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let chunked = client
        .request(reqwest::Method::PUT, &format!("/v1/blobs/{hash}/chunks?offset=0&size={}", large.len()))
        .body(large[..1024].to_vec())
        .send()
        .await
        .expect("sends");
    assert_eq!(chunked.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(chunked.text().await.expect("body").contains("at most 1 MiB"));
    // Whoever may read it hears why it won't arrive.
    let family = Family::new();
    let names = family.delta(
        ActorId(9),
        THREAD,
        Author::User,
        0,
        family.thread_head(THREAD),
        Op::Git(silt_log::GitOp::Bundle { commits: Vec::new(), blob: hash }),
    );
    assert_eq!(client.push(vec![names]).await.status(), StatusCode::OK);
    let told: silt_proto::BlobState = client
        .request(reqwest::Method::GET, &format!("/v1/blobs/{hash}/state"))
        .send()
        .await
        .expect("sends")
        .json()
        .await
        .expect("a state");
    assert!(!told.stored && told.refused.as_deref().is_some_and(|why| why.contains("at most 1 MiB")), "{told:?}");
    let small = b"fits".to_vec();
    let fits = client
        .request(reqwest::Method::PUT, &format!("/v1/blobs/{}", BlobStore::hash(&small)))
        .body(small)
        .send()
        .await
        .expect("sends");
    assert_eq!(fits.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn blobs_threads_and_leases() {
    let data = tempfile::tempdir().expect("temp dir");
    let base = start(data.path()).await;
    let (client, device) = Client::login(data.path(), &base).await;

    let hash = BlobStore::hash(b"bundle bytes");
    let response = client
        .request(reqwest::Method::PUT, &format!("/v1/blobs/{hash}"))
        .body(b"bundle bytes".to_vec())
        .send()
        .await
        .expect("sends");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = client
        .request(reqwest::Method::PUT, &format!("/v1/blobs/{hash}"))
        .body(b"other bytes".to_vec())
        .send()
        .await
        .expect("sends");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let fetch = || client.request(reqwest::Method::GET, &format!("/v1/blobs/{hash}")).send();
    assert_eq!(fetch().await.expect("sends").status(), StatusCode::NOT_FOUND, "a blob no delta refers to is no one's to fetch");
    let family = Family::new();
    let bundle = family.delta(
        ActorId(9),
        THREAD,
        Author::User,
        0,
        family.thread_head(THREAD),
        Op::Git(silt_log::GitOp::Bundle { commits: Vec::new(), blob: hash }),
    );
    assert_eq!(client.push(vec![bundle]).await.status(), StatusCode::OK);
    let fetched = fetch().await.expect("sends").bytes().await.expect("body");
    assert_eq!(&fetched[..], b"bundle bytes");

    let thread = ThreadInfo {
        id: THREAD,
        family: FAMILY,
        repo: "github.com/u/r".into(),
        created_ms: 1,
        lease: None,
        owner: String::new(),
        collaborators: Vec::new(),
    };
    client.request(reqwest::Method::POST, "/v1/threads").json(&thread).send().await.expect("sends");
    for epoch in 1..=2 {
        let lease: Lease = client
            .request(reqwest::Method::POST, &format!("/v1/threads/{THREAD}/lease"))
            .send()
            .await
            .expect("sends")
            .json()
            .await
            .expect("lease");
        assert_eq!(lease, Lease { device, epoch });
    }
    let listing: Listing =
        client.request(reqwest::Method::GET, "/v1/listing").send().await.expect("sends").json().await.expect("listing");
    assert_eq!(listing.threads[0].lease, Some(Lease { device, epoch: 2 }));
    assert_eq!(listing.threads[0].repo, "github.com/u/r");
    assert_eq!(listing.threads[0].owner, "owner");
}

impl Client {
    async fn listing(&self) -> Listing {
        self.request(reqwest::Method::GET, "/v1/listing").send().await.expect("sends").json().await.expect("listing")
    }

    async fn register(&self, repo: &str) -> StatusCode {
        let thread = ThreadInfo {
            id: THREAD,
            family: FAMILY,
            repo: repo.into(),
            created_ms: 1,
            lease: None,
            owner: String::new(),
            collaborators: Vec::new(),
        };
        self.request(reqwest::Method::POST, "/v1/threads").json(&thread).send().await.expect("sends").status()
    }

    async fn set_visibility(&self, request: &VisibilityRequest) -> StatusCode {
        self.request(reqwest::Method::PUT, "/v1/visibility").json(request).send().await.expect("sends").status()
    }
}

#[tokio::test]
async fn only_the_owner_moves_a_lease_and_private_families_stay_hidden() {
    let data = tempfile::tempdir().expect("temp dir");
    let base = start(data.path()).await;
    let (alice, _) = Client::login_as(data.path(), &base, "alice").await;
    let (bob, _) = Client::login_as(data.path(), &base, "bob").await;
    assert_eq!(alice.register("github.com/u/r").await, StatusCode::NO_CONTENT);
    assert_eq!(alice.push(deltas(2)).await.status(), StatusCode::OK);

    let listing = bob.listing().await;
    assert_eq!(listing.threads.len(), 1, "projects are team-visible by default");
    assert_eq!(listing.threads[0].owner, "alice");
    let response = bob.request(reqwest::Method::POST, &format!("/v1/threads/{THREAD}/lease")).send().await.expect("sends");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(matches!(response.json::<ErrorReply>().await.expect("body"), ErrorReply::Forbidden { .. }));
    let pulled: PullReply = bob.pull("after=0").await.json().await.expect("pull");
    assert_eq!(pulled.deltas.len(), 2, "teammates can read, and so fork, the family");

    let private_family = VisibilityRequest { repo: None, thread: Some(THREAD), visibility: Visibility::Private };
    assert_eq!(bob.set_visibility(&private_family).await, StatusCode::FORBIDDEN);
    assert_eq!(alice.set_visibility(&private_family).await, StatusCode::NO_CONTENT);
    assert!(bob.listing().await.threads.is_empty());
    assert!(bob.listing().await.families.is_empty());
    assert_eq!(bob.pull("after=0").await.status(), StatusCode::NOT_FOUND);
    assert_eq!(bob.push(deltas(3)).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(alice.listing().await.threads.len(), 1);

    let team_family = VisibilityRequest { visibility: Visibility::Org, ..private_family };
    assert_eq!(alice.set_visibility(&team_family).await, StatusCode::NO_CONTENT);
    let private_project =
        VisibilityRequest { repo: Some("github.com/u/r".into()), thread: None, visibility: Visibility::Private };
    assert_eq!(
        bob.set_visibility(&private_project).await,
        StatusCode::FORBIDDEN,
        "only the project's org owners change its visibility"
    );
    let (admin, _) = Client::login(data.path(), &base).await;
    assert_eq!(admin.set_visibility(&private_project).await, StatusCode::NO_CONTENT);
    assert_eq!(bob.listing().await.threads.len(), 1, "the thread's own setting wins over its project's");
}

/// A thread that edits `src/lexer.rs` on branch `main` and talks about it.
fn lexer_work() -> Vec<Delta> {
    use silt_crdt::InodeId;
    use silt_log::{ConvOp, GitOp, SessionFile, SessionRef};
    let mut family = Family::new();
    let mut deltas = Vec::new();
    let mut add = |family: &mut Family, op: Op| {
        let delta = family.delta(ActorId(1), THREAD, Author::User, now(), family.thread_head(THREAD), op);
        family.apply(delta.clone(), &mut HashMap::new()).expect("applies");
        deltas.push(delta.clone());
        delta.span.first()
    };
    add(&mut family, Op::Git(GitOp::Genesis { commit: silt_types::GitOid([1; 20]), branch: Some("main".into()) }));
    let src = add(
        &mut family,
        Op::Create { kind: InodeKind::Dir, parent: Parent::Root, name: "src".into(), content: Content::Empty, executable: false },
    );
    add(
        &mut family,
        Op::Create {
            kind: InodeKind::File,
            parent: Parent::Dir(InodeId(src)),
            name: "lexer.rs".into(),
            content: Content::Empty,
            executable: false,
        },
    );
    add(
        &mut family,
        Op::Conv(ConvOp::Native {
            session: SessionRef { agent: "claude-code".into(), id: "s".into() },
            file: SessionFile::Main,
            line: r#"{"type":"user","message":{"role":"user","content":"Fix the tokenizer bug in the lexer"}}"#.into(),
        }),
    );
    deltas
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| i64::try_from(duration.as_millis()).unwrap_or_default())
}

impl Client {
    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> T {
        self.request(reqwest::Method::GET, path).send().await.expect("sends").json().await.expect("json")
    }
}

#[tokio::test]
async fn search_filters_and_in_flight_respect_visibility() {
    use silt_proto::{InFlightReply, SearchReply};
    let data = tempfile::tempdir().expect("temp dir");
    let base = start(data.path()).await;
    let (alice, _) = Client::login_as(data.path(), &base, "alice").await;
    let (bob, _) = Client::login_as(data.path(), &base, "bob").await;
    alice.register("github.com/u/r").await;
    assert_eq!(alice.push(lexer_work()).await.status(), StatusCode::OK);

    let found: SearchReply = bob.get_json("/v1/search?q=tokeniz").await;
    assert_eq!(found.hits.len(), 1, "prefix search over teammates' visible threads");
    let hit = &found.hits[0];
    assert_eq!((hit.owner.as_str(), hit.status.as_str()), ("alice", "active"));
    assert_eq!(hit.branch.as_deref(), Some("main"));
    assert!(hit.snippet.contains("[tokenizer]"), "{}", hit.snippet);
    for (filters, expected) in [
        ("&repo=github.com/u/r&author=alice&branch=main&status=active&file=src", 1),
        ("&file=src/lexer.rs", 1),
        ("&author=bob", 0),
        ("&file=docs", 0),
        ("&status=merged", 0),
        ("&since=4102444800000", 0),
    ] {
        let found: SearchReply = bob.get_json(&format!("/v1/search?q=lexer{filters}")).await;
        assert_eq!(found.hits.len(), expected, "{filters}");
    }

    let flight: InFlightReply = bob.get_json("/v1/in-flight?repo=github.com/u/r&path=src").await;
    assert_eq!(flight.threads.len(), 1);
    assert_eq!(flight.threads[0].paths, vec!["src/lexer.rs".to_owned()]);
    let flight: InFlightReply = bob.get_json("/v1/in-flight?repo=github.com/u/r&path=docs").await;
    assert!(flight.threads.is_empty());

    alice.set_visibility(&VisibilityRequest { repo: None, thread: Some(THREAD), visibility: Visibility::Private }).await;
    let found: SearchReply = bob.get_json("/v1/search?q=lexer").await;
    assert!(found.hits.is_empty());
    let flight: InFlightReply = bob.get_json("/v1/in-flight?repo=github.com/u/r").await;
    assert!(flight.threads.is_empty());
    let found: SearchReply = alice.get_json("/v1/search?q=lexer").await;
    assert_eq!(found.hits.len(), 1);
}

#[tokio::test]
async fn a_private_fork_stays_hidden_until_offered_back() {
    use silt_log::MrOp;
    let data = tempfile::tempdir().expect("temp dir");
    let base = start(data.path()).await;
    let (alice, _) = Client::login_as(data.path(), &base, "alice").await;
    let (bob, _) = Client::login_as(data.path(), &base, "bob").await;
    alice.register("github.com/u/r").await;
    let mut family = Family::new();
    let write = |family: &mut Family, actor: u128, thread: ThreadId, parents: silt_types::Frontier, op: Op| {
        let delta = family.delta(ActorId(actor), thread, Author::User, 0, parents, op);
        family.apply(delta.clone(), &mut HashMap::new()).expect("applies");
        delta
    };
    let file = |name: &str| Op::Create {
        kind: InodeKind::File,
        parent: Parent::Root,
        name: name.into(),
        content: Content::Empty,
        executable: false,
    };
    let source = write(&mut family, 1, THREAD, silt_types::Frontier::root(), file("a.rs"));
    assert_eq!(alice.push(vec![source.clone()]).await.status(), StatusCode::OK);

    const FORK: ThreadId = ThreadId(77);
    let forked = write(&mut family, 2, FORK, silt_types::Frontier::single(source.span.last()), Op::Fork { from: THREAD });
    let secret = write(&mut family, 2, FORK, silt_types::Frontier::single(forked.span.last()), file("secret.rs"));
    let fork_info = ThreadInfo {
        id: FORK,
        family: FAMILY,
        repo: "github.com/u/r".into(),
        created_ms: 2,
        lease: None,
        owner: String::new(),
        collaborators: Vec::new(),
    };
    let status = bob.request(reqwest::Method::POST, "/v1/threads").json(&fork_info).send().await.expect("sends").status();
    assert_eq!(status, StatusCode::NO_CONTENT);
    let private_fork = VisibilityRequest { repo: None, thread: Some(FORK), visibility: Visibility::Private };
    assert_eq!(bob.set_visibility(&private_fork).await, StatusCode::NO_CONTENT);
    assert_eq!(bob.push(vec![forked.clone(), secret.clone()]).await.status(), StatusCode::OK);

    let pulled: PullReply = alice.pull("after=0").await.json().await.expect("pull");
    assert_eq!(pulled.deltas.len(), 1, "the private fork is withheld from the source's owner");
    let listing = alice.listing().await;
    assert!(listing.threads.iter().all(|thread| thread.id != FORK));
    let view_before = listing.families[0].view;
    let pulled: PullReply = bob.pull("after=0").await.json().await.expect("pull");
    assert_eq!(pulled.deltas.len(), 3, "the fork's owner reads everything");

    let opened = write(
        &mut family,
        2,
        FORK,
        silt_types::Frontier::single(secret.span.last()),
        Op::Mr(MrOp::Opened { target: THREAD, title: "secret".into(), summary: String::new() }),
    );
    assert_eq!(bob.push(vec![opened]).await.status(), StatusCode::OK);
    let pulled: PullReply = alice.pull("after=0").await.json().await.expect("pull");
    assert_eq!(pulled.deltas.len(), 4, "a merge request shares the fork's work with the target");
    assert_ne!(alice.listing().await.families[0].view, view_before, "clients learn to read the family again");
}

#[tokio::test]
async fn merge_requests_notify_only_about_threads_the_asker_sees() {
    use silt_log::MrOp;
    let data = tempfile::tempdir().expect("temp dir");
    let base = start(data.path()).await;
    let (alice, _) = Client::login_as(data.path(), &base, "alice").await;
    let (bob, _) = Client::login_as(data.path(), &base, "bob").await;
    alice.register("github.com/u/r").await;
    let mut family = Family::new();
    let mut write = |actor: u128, thread: ThreadId, parents: silt_types::Frontier, op: Op| {
        let delta = family.delta(ActorId(actor), thread, Author::User, 0, parents, op);
        family.apply(delta.clone(), &mut HashMap::new()).expect("applies");
        delta
    };
    let file = Op::Create {
        kind: InodeKind::File,
        parent: Parent::Root,
        name: "a.rs".into(),
        content: Content::Empty,
        executable: false,
    };
    let source = write(1, THREAD, silt_types::Frontier::root(), file);
    assert_eq!(alice.push(vec![source.clone()]).await.status(), StatusCode::OK);
    let info = |id| ThreadInfo {
        id,
        family: FAMILY,
        repo: "github.com/u/r".into(),
        created_ms: 2,
        lease: None,
        owner: String::new(),
        collaborators: Vec::new(),
    };

    const PRIVATE: ThreadId = ThreadId(76);
    let status = alice.request(reqwest::Method::POST, "/v1/threads").json(&info(PRIVATE)).send().await.expect("sends").status();
    assert_eq!(status, StatusCode::NO_CONTENT);
    let private = VisibilityRequest { repo: None, thread: Some(PRIVATE), visibility: Visibility::Private };
    assert_eq!(alice.set_visibility(&private).await, StatusCode::NO_CONTENT);
    let hidden = write(1, PRIVATE, silt_types::Frontier::single(source.span.last()), Op::Fork { from: THREAD });
    assert_eq!(alice.push(vec![hidden]).await.status(), StatusCode::OK);

    const FORK: ThreadId = ThreadId(77);
    let status = bob.request(reqwest::Method::POST, "/v1/threads").json(&info(FORK)).send().await.expect("sends").status();
    assert_eq!(status, StatusCode::NO_CONTENT);
    let forked = write(2, FORK, silt_types::Frontier::single(source.span.last()), Op::Fork { from: THREAD });
    let mr = |target| Op::Mr(MrOp::Opened { target, title: "click here".into(), summary: String::new() });
    let into_private = write(2, FORK, silt_types::Frontier::single(forked.span.last()), mr(PRIVATE));
    let into_main = write(2, FORK, silt_types::Frontier::single(into_private.span.last()), mr(THREAD));
    assert_eq!(bob.push(vec![forked, into_private]).await.status(), StatusCode::OK);
    let inbox = || async {
        let reply: silt_proto::InboxReply =
            alice.request(reqwest::Method::GET, "/v1/inbox").send().await.expect("sends").json().await.expect("inbox");
        reply.notifications
    };
    assert!(inbox().await.is_empty(), "bob can't see the private thread, so he can't ping its owner about it");
    assert_eq!(bob.push(vec![into_main]).await.status(), StatusCode::OK);
    let notifications = inbox().await;
    assert_eq!(notifications.len(), 1);
    assert_eq!((notifications[0].kind.as_str(), notifications[0].thread), ("mr_opened", THREAD));
}
