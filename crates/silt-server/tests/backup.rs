//! Backups taken while the server runs restore into a working server.

use std::collections::HashMap;

use reqwest::StatusCode;
use silt_crdt::{InodeKind, Parent};
use silt_log::{Author, Content, Family, Op};
use silt_proto::{LoginReply, PROTOCOL, PROTOCOL_HEADER, PasswordLoginRequest, PullReply, PushRequest, ThreadInfo};
use silt_server::backup::{backup, restore};
use silt_server::{Admin, Config, Server};
use silt_store::BlobStore;
use silt_types::{ActorId, FamilyId, ThreadId};

const FAMILY: FamilyId = FamilyId(70);
const THREAD: ThreadId = ThreadId(71);

async fn serve(data: &std::path::Path) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    tokio::spawn(Server::open_with(data, Config::default()).await.expect("opens").serve(listener));
    base
}

async fn sign_in(base: &str) -> String {
    let reply: LoginReply = reqwest::Client::new()
        .post(format!("{base}/v1/login/password"))
        .header(PROTOCOL_HEADER, PROTOCOL.to_string())
        .json(&PasswordLoginRequest { user: "alice".into(), password: "correct horse battery".into(), name: "laptop".into() })
        .send()
        .await
        .expect("sends")
        .json()
        .await
        .expect("login");
    reply.token
}

#[tokio::test]
async fn a_backup_taken_while_running_restores() {
    let data = tempfile::tempdir().expect("temp dir");
    Admin::open(data.path()).await.expect("opens").add_user("alice", Some("correct horse battery"), true).await.expect("adds");
    let base = serve(data.path()).await;
    let token = sign_in(&base).await;
    let http = reqwest::Client::new();
    let request = |method: reqwest::Method, path: String| {
        http.request(method, format!("{base}{path}")).header(PROTOCOL_HEADER, PROTOCOL.to_string()).bearer_auth(&token)
    };
    let blob = b"attached bytes".to_vec();
    let hash = BlobStore::hash(&blob);
    assert_eq!(
        request(reqwest::Method::PUT, format!("/v1/blobs/{hash}")).body(blob.clone()).send().await.expect("sends").status(),
        StatusCode::NO_CONTENT
    );
    let info = ThreadInfo {
        id: THREAD,
        family: FAMILY,
        repo: "github.com/acme/app".into(),
        created_ms: 1,
        lease: None,
        owner: String::new(),
        collaborators: Vec::new(),
    };
    request(reqwest::Method::POST, "/v1/threads".into()).json(&info).send().await.expect("sends");
    let mut family = Family::new();
    let delta = family.delta(
        ActorId(1),
        THREAD,
        Author::User,
        0,
        family.thread_head(THREAD),
        Op::Create {
            kind: InodeKind::File,
            parent: Parent::Root,
            name: "big.bin".into(),
            content: Content::Blob(hash),
            executable: false,
        },
    );
    family.apply(delta.clone(), &mut HashMap::new()).expect("applies");
    let pushed = request(reqwest::Method::POST, format!("/v1/families/{FAMILY}/deltas"))
        .json(&PushRequest { deltas: vec![delta] })
        .send()
        .await
        .expect("sends");
    assert_eq!(pushed.status(), StatusCode::OK);

    let saved = tempfile::tempdir().expect("temp dir");
    let target = saved.path().join("backup");
    backup(data.path(), &target).await.expect("backs up while the server runs");
    assert!(backup(data.path(), &target).await.is_err(), "an existing backup isn't overwritten");

    let restored = tempfile::tempdir().expect("temp dir");
    restore(&target, restored.path(), false).expect("restores");
    assert!(restore(&target, restored.path(), false).is_err(), "existing data needs --replace");
    let key = |dir: &std::path::Path| std::fs::read_to_string(dir.join("vapid.key")).expect("the push key");
    assert_eq!(key(restored.path()), key(data.path()), "browsers subscribed to the server's key go on receiving");
    let base = serve(restored.path()).await;
    let token = sign_in(&base).await;
    let pulled: PullReply = http
        .get(format!("{base}/v1/families/{FAMILY}/deltas?after=0"))
        .header(PROTOCOL_HEADER, PROTOCOL.to_string())
        .bearer_auth(&token)
        .send()
        .await
        .expect("sends")
        .json()
        .await
        .expect("pull");
    assert_eq!(pulled.deltas.len(), 1);
    let fetched = http
        .get(format!("{base}/v1/blobs/{hash}"))
        .header(PROTOCOL_HEADER, PROTOCOL.to_string())
        .bearer_auth(&token)
        .send()
        .await
        .expect("sends")
        .bytes()
        .await
        .expect("blob");
    assert_eq!(fetched.to_vec(), blob);
}
