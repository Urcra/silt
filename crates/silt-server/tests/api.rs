use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use silt_crdt::{InodeKind, Parent};
use silt_log::{Author, Content, Delta, Family, Op};
use silt_proto::{
    ErrorReply, GENERATION_HEADER, Lease, Listing, LoginReply, LoginRequest, PROTOCOL_HEADER, PullReply,
    PushReply, PushRequest, ThreadInfo, Visibility, VisibilityRequest,
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
            .header(PROTOCOL_HEADER, "1")
            .json(&LoginRequest {
                code,
                name: "laptop".into(),
            })
            .send()
            .await
            .expect("sends")
            .json()
            .await
            .expect("login reply");
        assert_eq!(reply.user, user);
        (
            Self {
                http,
                base: base.to_owned(),
                token: reply.token,
            },
            reply.device,
        )
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.base))
            .header(PROTOCOL_HEADER, "1")
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
        self.request(reqwest::Method::GET, &format!("/v1/families/{FAMILY}/deltas?{query}"))
            .send()
            .await
            .expect("sends")
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
    assert_eq!(reply, ErrorReply::ProtocolMismatch { supported: vec![1] });

    let response = http
        .get(format!("{base}/v1/listing"))
        .header(PROTOCOL_HEADER, "1")
        .bearer_auth("not-a-token")
        .send()
        .await
        .expect("sends");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = http
        .post(format!("{base}/v1/login"))
        .header(PROTOCOL_HEADER, "1")
        .json(&LoginRequest {
            code: "wrong".into(),
            name: "x".into(),
        })
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
        let client = Client {
            http: client.http.clone(),
            base: client.base.clone(),
            token: client.token.clone(),
        };
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
        Op::Delete {
            inode: silt_crdt::InodeId(all[0].span.first()),
        },
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
    let fetched = client
        .request(reqwest::Method::GET, &format!("/v1/blobs/{hash}"))
        .send()
        .await
        .expect("sends")
        .bytes()
        .await
        .expect("body");
    assert_eq!(&fetched[..], b"bundle bytes");

    let thread = ThreadInfo {
        id: THREAD,
        family: FAMILY,
        repo: "github.com/u/r".into(),
        created_ms: 1,
        lease: None,
        owner: String::new(),
    };
    client
        .request(reqwest::Method::POST, "/v1/threads")
        .json(&thread)
        .send()
        .await
        .expect("sends");
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
    let listing: Listing = client
        .request(reqwest::Method::GET, "/v1/listing")
        .send()
        .await
        .expect("sends")
        .json()
        .await
        .expect("listing");
    assert_eq!(listing.threads[0].lease, Some(Lease { device, epoch: 2 }));
    assert_eq!(listing.threads[0].repo, "github.com/u/r");
    assert_eq!(listing.threads[0].owner, "owner");
}

impl Client {
    async fn listing(&self) -> Listing {
        self.request(reqwest::Method::GET, "/v1/listing")
            .send()
            .await
            .expect("sends")
            .json()
            .await
            .expect("listing")
    }

    async fn register(&self, repo: &str) -> StatusCode {
        let thread = ThreadInfo {
            id: THREAD,
            family: FAMILY,
            repo: repo.into(),
            created_ms: 1,
            lease: None,
            owner: String::new(),
        };
        self.request(reqwest::Method::POST, "/v1/threads")
            .json(&thread)
            .send()
            .await
            .expect("sends")
            .status()
    }

    async fn set_visibility(&self, request: &VisibilityRequest) -> StatusCode {
        self.request(reqwest::Method::PUT, "/v1/visibility")
            .json(request)
            .send()
            .await
            .expect("sends")
            .status()
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
    let response = bob
        .request(reqwest::Method::POST, &format!("/v1/threads/{THREAD}/lease"))
        .send()
        .await
        .expect("sends");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(matches!(response.json::<ErrorReply>().await.expect("body"), ErrorReply::Forbidden { .. }));
    let pulled: PullReply = bob.pull("after=0").await.json().await.expect("pull");
    assert_eq!(pulled.deltas.len(), 2, "teammates can read, and so fork, the family");

    let private_family = VisibilityRequest {
        repo: None,
        family: Some(FAMILY),
        visibility: Visibility::Private,
    };
    assert_eq!(bob.set_visibility(&private_family).await, StatusCode::FORBIDDEN);
    assert_eq!(alice.set_visibility(&private_family).await, StatusCode::NO_CONTENT);
    assert!(bob.listing().await.threads.is_empty());
    assert!(bob.listing().await.families.is_empty());
    assert_eq!(bob.pull("after=0").await.status(), StatusCode::NOT_FOUND);
    assert_eq!(bob.push(deltas(3)).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(alice.listing().await.threads.len(), 1);

    let team_family = VisibilityRequest {
        visibility: Visibility::Team,
        ..private_family
    };
    assert_eq!(alice.set_visibility(&team_family).await, StatusCode::NO_CONTENT);
    let private_project = VisibilityRequest {
        repo: Some("github.com/u/r".into()),
        family: None,
        visibility: Visibility::Private,
    };
    assert_eq!(bob.set_visibility(&private_project).await, StatusCode::NO_CONTENT);
    assert_eq!(bob.listing().await.threads.len(), 1, "the family's own setting wins over its project's");
}
