//! The self-hostable sync server (ADR 0009): one SQLite database, a blob
//! store and a [`Room`] per family, behind a small HTTP API.

use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::{Path as UrlPath, Query, Request, State};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use silt_log::Delta;
use silt_proto::{
    ErrorReply, FamilyInfo, GENERATION_HEADER, Lease, LeaseRequest, Listing, LoginReply, LoginRequest, PROTOCOL,
    PROTOCOL_HEADER, PullReply, PushReply, PushRequest, ThreadInfo,
};
use silt_store::BlobStore;
use silt_sync::{PullError, PushError, Room};
use silt_types::{BlobHash, DeltaId, DeviceId, FamilyId, ThreadId};
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Longest a pull waits for new deltas.
const MAX_WAIT: Duration = Duration::from_secs(30);
const PULL_LIMIT: usize = 1000;
const PAIRING_TTL_MS: i64 = 15 * 60 * 1000;

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("database error")]
    Database(#[from] sqlx::Error),
    #[error("could not migrate the database")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("stored delta could not be decoded")]
    Decode(#[from] postcard::Error),
    #[error("I/O error")]
    Io(#[from] std::io::Error),
}

struct Shared {
    db: SqlitePool,
    blobs: BlobStore,
    generation: u64,
    rooms: Mutex<HashMap<FamilyId, Room>>,
    waiters: Mutex<HashMap<FamilyId, Arc<Notify>>>,
}

#[derive(Clone)]
pub struct Server {
    shared: Arc<Shared>,
}

impl Server {
    /// Opens the server's data in `data_dir` and loads every family. Each
    /// start gets a new generation, so clients re-check what they synced.
    pub async fn open(data_dir: &Path) -> Result<Self, ServerError> {
        tokio::fs::create_dir_all(data_dir).await?;
        let db = open_db(data_dir).await?;
        let generation = u64::from_le_bytes(uuid::Uuid::new_v4().as_bytes()[..8].try_into().expect("8 bytes"));
        let mut rooms: HashMap<FamilyId, Room> = HashMap::new();
        let rows = sqlx::query("SELECT family_id, data FROM deltas ORDER BY family_id, seq")
            .fetch_all(&db)
            .await?;
        for row in rows {
            let family = FamilyId(id_from(row.get("family_id")));
            let delta: Delta = postcard::from_bytes(row.get::<&[u8], _>("data"))?;
            let room = rooms.entry(family).or_insert_with(|| Room::new(generation));
            if let Err(error) = room.push(vec![delta]) {
                tracing::warn!(%error, "skipping a stored delta that no longer applies");
            }
        }
        Ok(Self {
            shared: Arc::new(Shared {
                db,
                blobs: BlobStore::new(data_dir.join("blobs")),
                generation,
                rooms: Mutex::new(rooms),
                waiters: Mutex::new(HashMap::new()),
            }),
        })
    }

    pub fn router(&self) -> Router {
        let authed = Router::new()
            .route("/v1/listing", get(listing))
            .route("/v1/threads", post(register_thread))
            .route("/v1/threads/{id}/lease", post(take_lease))
            .route("/v1/families/{id}/deltas", post(push).get(pull))
            .route("/v1/blobs/{hash}", put(put_blob).get(get_blob))
            .layer(middleware::from_fn_with_state(self.clone(), authenticate));
        Router::new()
            .route("/v1/login", post(login))
            .merge(authed)
            .layer(middleware::from_fn_with_state(self.clone(), protocol))
            .with_state(self.clone())
    }

    pub async fn serve(self, listener: TcpListener) -> std::io::Result<()> {
        axum::serve(listener, self.router()).await
    }
}

/// Creates a one-time code that lets a device log in, valid for 15 minutes.
pub async fn create_pairing_code(data_dir: &Path) -> Result<String, ServerError> {
    tokio::fs::create_dir_all(data_dir).await?;
    let db = open_db(data_dir).await?;
    let code: String = uuid::Uuid::new_v4().simple().to_string()[..10].to_owned();
    sqlx::query("INSERT INTO pairing_codes (code_hash, expires_ms) VALUES (?, ?)")
        .bind(hash(&code).to_vec())
        .bind(now_ms() + PAIRING_TTL_MS)
        .execute(&db)
        .await?;
    Ok(code)
}

async fn open_db(data_dir: &Path) -> Result<SqlitePool, ServerError> {
    let options = SqliteConnectOptions::from_str("sqlite://")?
        .filename(data_dir.join("server.sqlite"))
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);
    let db = SqlitePoolOptions::new().max_connections(4).connect_with(options).await?;
    MIGRATOR.run(&db).await?;
    Ok(db)
}

struct ApiError(StatusCode, ErrorReply);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(self.1)).into_response()
    }
}

impl From<ServerError> for ApiError {
    fn from(error: ServerError) -> Self {
        tracing::error!(%error, "request failed");
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorReply::Invalid {
                message: "internal error".into(),
            },
        )
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        ServerError::from(error).into()
    }
}

fn bad_request(message: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, ErrorReply::Invalid { message: message.into() })
}

/// Rejects other protocol versions and stamps every response with the
/// server's generation.
async fn protocol(State(server): State<Server>, request: Request, next: Next) -> Response {
    let version = request
        .headers()
        .get(PROTOCOL_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u32>().ok());
    let mut response = if version == Some(PROTOCOL) {
        next.run(request).await
    } else {
        ApiError(
            StatusCode::UPGRADE_REQUIRED,
            ErrorReply::ProtocolMismatch {
                supported: vec![PROTOCOL],
            },
        )
        .into_response()
    };
    response.headers_mut().insert(
        GENERATION_HEADER,
        HeaderValue::from_str(&server.shared.generation.to_string()).expect("digits are a valid header"),
    );
    response
}

async fn authenticate(State(server): State<Server>, mut request: Request, next: Next) -> Response {
    let token = request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::to_owned);
    let device = match token {
        Some(token) => sqlx::query("SELECT id FROM devices WHERE token_hash = ?")
            .bind(hash(&token).to_vec())
            .fetch_optional(&server.shared.db)
            .await
            .ok()
            .flatten()
            .map(|row| DeviceId(id_from(row.get("id")))),
        None => None,
    };
    match device {
        Some(device) => {
            request.extensions_mut().insert(device);
            next.run(request).await
        }
        None => ApiError(StatusCode::UNAUTHORIZED, ErrorReply::Unauthorized).into_response(),
    }
}

async fn login(State(server): State<Server>, Json(body): Json<LoginRequest>) -> Result<Json<LoginReply>, ApiError> {
    let db = &server.shared.db;
    let removed = sqlx::query("DELETE FROM pairing_codes WHERE code_hash = ? AND expires_ms > ?")
        .bind(hash(&body.code).to_vec())
        .bind(now_ms())
        .execute(db)
        .await?
        .rows_affected();
    if removed == 0 {
        return Err(ApiError(StatusCode::UNAUTHORIZED, ErrorReply::Unauthorized));
    }
    let device = DeviceId(uuid::Uuid::new_v4().as_u128());
    let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    sqlx::query("INSERT INTO devices (id, name, token_hash, created_ms) VALUES (?, ?, ?, ?)")
        .bind(id_bytes(device.0))
        .bind(&body.name)
        .bind(hash(&token).to_vec())
        .bind(now_ms())
        .execute(db)
        .await?;
    Ok(Json(LoginReply { device, token }))
}

async fn listing(State(server): State<Server>) -> Result<Json<Listing>, ApiError> {
    let families = server
        .shared
        .rooms
        .lock()
        .await
        .iter()
        .map(|(id, room)| FamilyInfo { id: *id, seq: room.seq() })
        .collect();
    let rows = sqlx::query(
        "SELECT t.id, t.family_id, t.repo, t.created_ms, l.device_id, l.epoch
         FROM threads t LEFT JOIN leases l ON l.thread_id = t.id ORDER BY t.created_ms",
    )
    .fetch_all(&server.shared.db)
    .await?;
    let threads = rows
        .iter()
        .map(|row| ThreadInfo {
            id: ThreadId(id_from(row.get("id"))),
            family: FamilyId(id_from(row.get("family_id"))),
            repo: row.get("repo"),
            created_ms: row.get("created_ms"),
            lease: row.get::<Option<&[u8]>, _>("device_id").map(|device| Lease {
                device: DeviceId(id_from(device)),
                epoch: u64::try_from(row.get::<i64, _>("epoch")).unwrap_or_default(),
            }),
        })
        .collect();
    Ok(Json(Listing { families, threads }))
}

async fn register_thread(State(server): State<Server>, Json(thread): Json<ThreadInfo>) -> Result<StatusCode, ApiError> {
    sqlx::query("INSERT OR IGNORE INTO threads (id, family_id, repo, created_ms) VALUES (?, ?, ?, ?)")
        .bind(id_bytes(thread.id.0))
        .bind(id_bytes(thread.family.0))
        .bind(&thread.repo)
        .bind(thread.created_ms)
        .execute(&server.shared.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn take_lease(
    State(server): State<Server>,
    UrlPath(thread): UrlPath<String>,
    Json(body): Json<LeaseRequest>,
) -> Result<Json<Lease>, ApiError> {
    let thread = ThreadId::from_str(&thread).map_err(|_| bad_request("invalid thread id"))?;
    let known = sqlx::query("SELECT 1 FROM threads WHERE id = ?")
        .bind(id_bytes(thread.0))
        .fetch_optional(&server.shared.db)
        .await?;
    if known.is_none() {
        return Err(ApiError(StatusCode::NOT_FOUND, ErrorReply::NotFound));
    }
    let row = sqlx::query(
        "INSERT INTO leases (thread_id, device_id, epoch) VALUES (?, ?, 1)
         ON CONFLICT (thread_id) DO UPDATE SET device_id = excluded.device_id, epoch = leases.epoch + 1
         RETURNING epoch",
    )
    .bind(id_bytes(thread.0))
    .bind(id_bytes(body.device.0))
    .fetch_one(&server.shared.db)
    .await?;
    Ok(Json(Lease {
        device: body.device,
        epoch: u64::try_from(row.get::<i64, _>("epoch")).unwrap_or_default(),
    }))
}

async fn push(
    State(server): State<Server>,
    UrlPath(family): UrlPath<String>,
    Json(body): Json<PushRequest>,
) -> Result<Json<PushReply>, ApiError> {
    let family = FamilyId::from_str(&family).map_err(|_| bad_request("invalid family id"))?;
    let mut rooms = server.shared.rooms.lock().await;
    let room = rooms
        .entry(family)
        .or_insert_with(|| Room::new(server.shared.generation));
    let before = room.seq();
    let outcome = room.push(body.deltas).map_err(|error| match error {
        PushError::Conflict(delta) => ApiError(StatusCode::CONFLICT, ErrorReply::Conflict { delta }),
        PushError::MissingParents(delta) => ApiError(StatusCode::CONFLICT, ErrorReply::MissingParents { delta }),
        PushError::Invalid { reason, .. } => bad_request(reason),
    })?;
    let stored: Vec<(u64, Delta)> = room.deltas()[usize::try_from(before).unwrap_or(usize::MAX)..]
        .iter()
        .cloned()
        .zip(before + 1..)
        .map(|(delta, seq)| (seq, delta))
        .collect();
    let mut tx = server.shared.db.begin().await?;
    for (seq, delta) in &stored {
        sqlx::query("INSERT INTO deltas (family_id, seq, data) VALUES (?, ?, ?)")
            .bind(id_bytes(family.0))
            .bind(i64::try_from(*seq).unwrap_or(i64::MAX))
            .bind(postcard::to_allocvec(delta).map_err(ServerError::from)?)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    drop(rooms);
    if !stored.is_empty() {
        server.waiter(family).await.notify_waiters();
    }
    Ok(Json(PushReply {
        accepted: outcome.accepted,
        seq: outcome.seq,
    }))
}

#[derive(Deserialize)]
struct PullQuery {
    #[serde(default)]
    after: u64,
    last: Option<String>,
    #[serde(default)]
    wait: u64,
}

async fn pull(
    State(server): State<Server>,
    UrlPath(family): UrlPath<String>,
    Query(query): Query<PullQuery>,
) -> Result<Json<PullReply>, ApiError> {
    let family = FamilyId::from_str(&family).map_err(|_| bad_request("invalid family id"))?;
    let last = match &query.last {
        Some(last) => Some(DeltaId::from_str(last).map_err(|_| bad_request("invalid cursor"))?),
        None => None,
    };
    let wait = Duration::from_secs(query.wait).min(MAX_WAIT);
    let notified = server.waiter(family).await;
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let waiting = notified.notified();
        tokio::pin!(waiting);
        waiting.as_mut().enable();
        let deltas = {
            let rooms = server.shared.rooms.lock().await;
            match rooms.get(&family) {
                Some(room) => room.pull(query.after, last, PULL_LIMIT).map_err(|error| match error {
                    PullError::UnknownCursor { .. } => ApiError(StatusCode::CONFLICT, ErrorReply::UnknownCursor),
                })?,
                None if query.after == 0 => Vec::new(),
                None => return Err(ApiError(StatusCode::CONFLICT, ErrorReply::UnknownCursor)),
            }
        };
        if !deltas.is_empty() || tokio::time::Instant::now() >= deadline {
            return Ok(Json(PullReply { deltas }));
        }
        let _ = tokio::time::timeout_at(deadline, waiting).await;
    }
}

async fn put_blob(
    State(server): State<Server>,
    UrlPath(hash_hex): UrlPath<String>,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    if BlobStore::hash(&body).to_string() != hash_hex {
        return Err(bad_request("blob content does not match its hash"));
    }
    server.shared.blobs.put(&body).await.map_err(ServerError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_blob(State(server): State<Server>, UrlPath(hash_hex): UrlPath<String>) -> Result<Vec<u8>, ApiError> {
    let hash = parse_blob_hash(&hash_hex).ok_or_else(|| bad_request("invalid blob hash"))?;
    server
        .shared
        .blobs
        .get(hash)
        .await
        .map_err(|_| ApiError(StatusCode::NOT_FOUND, ErrorReply::NotFound))
}

impl Server {
    async fn waiter(&self, family: FamilyId) -> Arc<Notify> {
        self.shared
            .waiters
            .lock()
            .await
            .entry(family)
            .or_insert_with(|| Arc::new(Notify::new()))
            .clone()
    }
}

fn parse_blob_hash(hex: &str) -> Option<BlobHash> {
    if hex.len() != 64 {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (byte, pair) in bytes.iter_mut().zip(hex.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(BlobHash(bytes))
}

fn hash(text: &str) -> [u8; 32] {
    *blake3::hash(text.as_bytes()).as_bytes()
}

fn id_bytes(id: u128) -> Vec<u8> {
    id.to_be_bytes().to_vec()
}

fn id_from(bytes: &[u8]) -> u128 {
    let mut buffer = [0u8; 16];
    buffer.copy_from_slice(&bytes[..16.min(bytes.len())]);
    u128::from_be_bytes(buffer)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
}
