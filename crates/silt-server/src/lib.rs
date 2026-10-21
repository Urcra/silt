//! The self-hostable sync server (ADR 0009): one SQLite database, a blob
//! store and a [`Room`] per family, behind a small HTTP API.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Extension, Path as UrlPath, Query, Request, State};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use silt_log::Delta;
use silt_proto::{
    DevicePollReply, DevicePollRequest, DeviceStartReply, DeviceStartRequest, ErrorReply, FamilyInfo, GENERATION_HEADER,
    InFlightQuery, InFlightReply, InFlightThread, Lease, Listing, LoginReply, LoginRequest, PROTOCOL, PROTOCOL_HEADER,
    PasswordLoginRequest, PullReply, PushReply, PushRequest, RegisterThread, SearchHit, SearchQuery, SearchReply, ThreadInfo,
    VisibilityRequest,
};
use silt_store::BlobStore;
use silt_sync::{PullError, PushError, Room};
use silt_types::{BlobHash, DeltaId, DeviceId, FamilyId, ThreadId};
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify, OwnedMutexGuard, RwLock};

pub mod accounts;
pub mod backup;
mod collab;
pub mod config;
mod git;
mod inbox;
mod index;
mod lifecycle;
mod orgs;
mod pages;
mod presence;
mod sign_in;
mod visibility;

pub use accounts::{AccountError, Admin, User};
pub use config::{Config, ConfigError};

use index::Index;
use visibility::Viewer;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Longest a pull waits for new deltas.
const MAX_WAIT: Duration = Duration::from_secs(30);
const PULL_LIMIT: usize = 1000;
const PAIRING_TTL_MS: i64 = 15 * 60 * 1000;
/// Threads written to within this long are `active`.
const ACTIVE_MS: i64 = 24 * 60 * 60 * 1000;
const SEARCH_LIMIT: u32 = 50;
/// Largest request body anywhere but the routes that carry work: sign-in
/// pages and the like are reachable without an account (ADR 0016).
const SMALL_BODY: usize = 1 << 20;
/// Largest batch of deltas one push may send.
const DELTAS_BODY: usize = 256 << 20;
/// Largest blob or git pack one request may send.
const LARGE_BODY: usize = 512 << 20;

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("database error")]
    Database(#[from] sqlx::Error),
    #[error("a delta could not be encoded")]
    Encode(#[from] postcard::Error),
    #[error("I/O error")]
    Io(#[from] std::io::Error),
    #[error("could not write JSON")]
    Json(#[from] serde_json::Error),
    #[error("the deltas kept after a purge no longer fit together")]
    Kept(#[from] PushError),
}

/// Why the server's data could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("could not create the data directory")]
    DataDir(#[from] std::io::Error),
    #[error("could not open the database")]
    Database(#[from] sqlx::Error),
    #[error("could not migrate the database")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Stored(#[from] silt_log::StoredError),
    #[error("could not set up the HTTP client for sign-in providers")]
    HttpClient(#[from] reqwest::Error),
}

/// One family's room, locked on its own so families sync independently.
pub(crate) struct RoomSlot {
    pub(crate) room: Room,
    /// Set by a purge that dropped the family: whoever looked the room up
    /// before then finds out once they hold its lock.
    pub(crate) retired: bool,
    /// Listing views by viewer serial, with the `seq` they were worked
    /// out at: clients list every few seconds, and a view reads every delta.
    pub(crate) views: HashMap<u64, (u64, u64)>,
}

/// How far everything visibility is worked out from has changed: the
/// database's visibility epoch and the fork map's version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VisibilityAt {
    epoch: i64,
    forks: u64,
}

/// Views kept per room before they are dropped together.
const VIEWS_KEPT: usize = 64;

impl RoomSlot {
    fn new(room: Room) -> Self {
        Self { room, retired: false, views: HashMap::new() }
    }

    fn view(&mut self, family: FamilyId, viewer: &Viewer) -> u64 {
        let seq = self.room.seq();
        if let Some((at, view)) = self.views.get(&viewer.serial())
            && *at == seq
        {
            return *view;
        }
        let view = viewer.view_of(family.0, &self.room);
        if self.views.len() >= VIEWS_KEPT {
            self.views.clear();
        }
        self.views.insert(viewer.serial(), (seq, view));
        view
    }
}

pub(crate) type SharedRoom = Arc<Mutex<RoomSlot>>;

struct Shared {
    db: SqlitePool,
    blobs: BlobStore,
    generation: u64,
    /// Held only to look rooms up, except by a purge, which holds it
    /// throughout so no room appears meanwhile. Room locks come after it,
    /// then `purged`, then `index`.
    rooms: RwLock<HashMap<FamilyId, SharedRoom>>,
    waiters: Waiters,
    index: Mutex<Index>,
    /// Each user's latest visibility snapshot, with when it was taken.
    viewers: Mutex<HashMap<i64, (VisibilityAt, Arc<Viewer>)>>,
    viewer_serial: std::sync::atomic::AtomicU64,
    data_dir: std::path::PathBuf,
    config: Config,
    http: reqwest::Client,
    /// Threads purged for good; their deltas are never stored again.
    purged: Mutex<std::collections::HashSet<ThreadId>>,
    presence: Mutex<presence::Board>,
    /// Password checks: how many run at once, and recent failures by name.
    passwords: accounts::PasswordChecks,
}

#[derive(Clone)]
pub struct Server {
    shared: Arc<Shared>,
}

impl Server {
    /// Opens the server's data in `data_dir` and loads every family. Each
    /// start gets a new generation, so clients re-check what they synced.
    pub async fn open(data_dir: &Path) -> Result<Self, OpenError> {
        let config = Config::load(data_dir)?;
        Self::open_with(data_dir, config).await
    }

    pub async fn open_with(data_dir: &Path, config: Config) -> Result<Self, OpenError> {
        tokio::fs::create_dir_all(data_dir).await?;
        let db = open_db(data_dir).await?;
        let generation = u64::from_le_bytes(uuid::Uuid::new_v4().as_bytes()[..8].try_into().expect("8 bytes"));
        let purged = lifecycle::load_purged(&db).await?;
        let mut stored: BTreeMap<FamilyId, Vec<Delta>> = BTreeMap::new();
        let rows = sqlx::query("SELECT family_id, format, data FROM deltas ORDER BY family_id, seq").fetch_all(&db).await?;
        for row in rows {
            let family = FamilyId(id_from(row.get("family_id")));
            let delta = silt_log::decode_stored(row.get("format"), row.get::<&[u8], _>("data"))?;
            stored.entry(family).or_default().push(delta);
        }
        let mut rooms: HashMap<FamilyId, Room> = HashMap::new();
        let mut index = Index::default();
        let mut index_rows = Vec::new();
        for (family, deltas) in stored {
            // Purged threads' deltas stay only where a kept one builds on
            // them, as when they were purged.
            let kept = silt_log::kept_after_purge(&deltas, &purged);
            let room = rooms.entry(family).or_insert_with(|| Room::new(generation));
            for (delta, kept) in deltas.into_iter().zip(kept) {
                if !kept {
                    continue;
                }
                index_rows.extend(index.observe(family, &delta));
                if let Err(error) = room.push(vec![delta]) {
                    tracing::warn!(%error, "skipping a stored delta that no longer applies");
                }
            }
        }
        let mut tx = db.begin().await?;
        index::clear(&mut tx).await?;
        index::write(&mut tx, &index_rows).await?;
        tx.commit().await?;
        let http = reqwest::Client::builder().timeout(Duration::from_secs(20)).build()?;
        Ok(Self {
            shared: Arc::new(Shared {
                db,
                blobs: BlobStore::new(data_dir.join("blobs")),
                generation,
                rooms: RwLock::new(
                    rooms.into_iter().map(|(family, room)| (family, Arc::new(Mutex::new(RoomSlot::new(room))))).collect(),
                ),
                waiters: Waiters::default(),
                index: Mutex::new(index),
                viewers: Mutex::new(HashMap::new()),
                viewer_serial: std::sync::atomic::AtomicU64::new(0),
                data_dir: data_dir.to_owned(),
                purged: Mutex::new(purged),
                presence: Mutex::new(presence::Board::default()),
                passwords: accounts::PasswordChecks::default(),
                config,
                http,
            }),
        })
    }

    pub fn config(&self) -> &Config {
        &self.shared.config
    }

    pub(crate) fn db(&self) -> &SqlitePool {
        &self.shared.db
    }

    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.shared.http
    }

    pub fn router(&self) -> Router {
        let authed = Router::new()
            .route("/v1/listing", get(listing))
            .route("/v1/threads", post(register_thread))
            .route("/v1/threads/{id}/lease", post(take_lease))
            .route("/v1/families/{id}/deltas", post(push).get(pull).layer(DefaultBodyLimit::max(DELTAS_BODY)))
            .route("/v1/blobs/{hash}", put(put_blob).get(get_blob).layer(DefaultBodyLimit::max(LARGE_BODY)))
            .route("/v1/visibility", put(set_visibility))
            .route("/v1/search", get(search))
            .route("/v1/in-flight", get(in_flight))
            .route("/v1/device", delete(logout_device))
            .route("/v1/threads/{id}/collaborators", post(collab::share))
            .route("/v1/threads/{id}/delete", post(lifecycle::delete))
            .route("/v1/threads/{id}/restore", post(lifecycle::restore))
            .route("/v1/threads/{id}/purge", post(lifecycle::purge))
            .route("/v1/trash", get(lifecycle::trash))
            .route("/v1/presence", get(presence::list).post(presence::update))
            .route("/v1/inbox", get(inbox::list))
            .route("/v1/inbox/read", post(inbox::read))
            .route("/v1/orgs", get(orgs::list))
            .route("/v1/orgs/change", post(orgs::change))
            .route("/git/{repo}/{*rest}", get(git::serve).post(git::serve).layer(DefaultBodyLimit::max(LARGE_BODY)))
            .layer(middleware::from_fn_with_state(self.clone(), authenticate));
        let api = Router::new()
            .route("/v1/login", post(login))
            .route("/v1/login/password", post(password_login))
            .route("/v1/device/start", post(device_start))
            .route("/v1/device/poll", post(device_poll))
            .merge(authed)
            .layer(middleware::from_fn_with_state(self.clone(), protocol));
        let pages = Router::new()
            .route("/", get(pages::home))
            .route("/login", get(pages::login_page).post(pages::login))
            .route("/logout", post(pages::logout))
            .route("/devices/revoke", post(pages::revoke_device))
            .route("/sessions/end-others", post(pages::end_other_sessions))
            .route("/device", get(pages::device_page).post(pages::device))
            .route("/signup", get(pages::signup_page).post(pages::signup))
            .route("/auth/{provider}/start", get(pages::provider_start))
            .route("/auth/{provider}/callback", get(pages::provider_callback));
        Router::new()
            .merge(api)
            .merge(pages)
            // Bundles and git pushes are bigger than axum's default limit.
            .layer(DefaultBodyLimit::max(SMALL_BODY))
            .with_state(self.clone())
    }

    pub async fn serve(self, listener: TcpListener) -> std::io::Result<()> {
        if let Err(error) = self.purge_expired().await {
            tracing::warn!(%error, "could not purge the trash");
        }
        self.keep_trash_tidy();
        axum::serve(listener, self.router()).await
    }
}

/// Creates a one-time code that lets a device of `user` log in, valid for
/// 15 minutes. The user is created if it doesn't exist.
pub async fn create_pairing_code(data_dir: &Path, user: &str) -> Result<String, AccountError> {
    tokio::fs::create_dir_all(data_dir).await.map_err(OpenError::from)?;
    let db = open_db(data_dir).await?;
    if accounts::user_by_name(&db, user).await?.is_none() {
        accounts::create_user(
            &db,
            &accounts::NewUser { name: user, display_name: None, email: None, password: None, admin: false },
        )
        .await?;
    }
    let code: String = uuid::Uuid::new_v4().simple().to_string()[..10].to_owned();
    sqlx::query("INSERT INTO pairing_codes (code_hash, expires_ms, user_id) SELECT ?, ?, id FROM users WHERE name = ?")
        .bind(hash(&code).to_vec())
        .bind(now_ms() + PAIRING_TTL_MS)
        .bind(user)
        .execute(&db)
        .await?;
    Ok(code)
}

pub(crate) async fn open_db(data_dir: &Path) -> Result<SqlitePool, OpenError> {
    let options = SqliteConnectOptions::from_str("sqlite://")?
        .filename(data_dir.join("server.sqlite"))
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);
    let db = SqlitePoolOptions::new().max_connections(4).connect_with(options).await?;
    MIGRATOR.run(&db).await?;
    Ok(db)
}

pub(crate) struct ApiError(StatusCode, ErrorReply);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(self.1)).into_response()
    }
}

impl From<ServerError> for ApiError {
    fn from(error: ServerError) -> Self {
        tracing::error!(%error, "request failed");
        Self(StatusCode::INTERNAL_SERVER_ERROR, ErrorReply::Invalid { message: "internal error".into() })
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        ServerError::from(error).into()
    }
}

pub(crate) fn bad_request(message: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, ErrorReply::Invalid { message: message.into() })
}

pub(crate) fn forbidden(message: impl Into<String>) -> ApiError {
    ApiError(StatusCode::FORBIDDEN, ErrorReply::Forbidden { message: message.into() })
}

pub(crate) fn not_found() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, ErrorReply::NotFound)
}

/// The authenticated device and the user it belongs to.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Caller {
    device: DeviceId,
    user: i64,
}

pub(crate) enum FamilyAccess {
    /// No device has registered or pushed it yet.
    Unknown,
    Visible,
    Hidden,
}

pub(crate) async fn family_access(server: &Server, family: FamilyId, user: i64) -> Result<FamilyAccess, ApiError> {
    let row = sqlx::query("SELECT owner_id FROM families WHERE id = ?")
        .bind(id_bytes(family.0))
        .fetch_optional(&server.shared.db)
        .await?;
    if row.is_none() {
        return Ok(FamilyAccess::Unknown);
    }
    Ok(if server.viewer(user).await?.sees_family(family.0) { FamilyAccess::Visible } else { FamilyAccess::Hidden })
}

/// Lets `caller` read or write the family, claiming it when it's new.
pub(crate) async fn claim_family(server: &Server, family: FamilyId, caller: Caller) -> Result<(), ApiError> {
    match family_access(server, family, caller.user).await? {
        FamilyAccess::Visible => Ok(()),
        FamilyAccess::Hidden => Err(not_found()),
        FamilyAccess::Unknown => {
            sqlx::query("INSERT OR IGNORE INTO families (id, owner_id) VALUES (?, ?)")
                .bind(id_bytes(family.0))
                .bind(caller.user)
                .execute(&server.shared.db)
                .await?;
            Ok(())
        }
    }
}

/// Rejects other protocol versions and stamps every response with the
/// server's generation.
async fn protocol(State(server): State<Server>, request: Request, next: Next) -> Response {
    let version =
        request.headers().get(PROTOCOL_HEADER).and_then(|value| value.to_str().ok()).and_then(|value| value.parse::<u32>().ok());
    let mut response = if version == Some(PROTOCOL) {
        next.run(request).await
    } else {
        ApiError(StatusCode::UPGRADE_REQUIRED, ErrorReply::ProtocolMismatch { supported: vec![PROTOCOL] }).into_response()
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
    let caller = match token {
        Some(token) => accounts::device_caller(server.db(), &token).await,
        None => Ok(None),
    };
    match caller {
        Ok(Some((device, user))) => {
            request.extensions_mut().insert(Caller { device, user });
            next.run(request).await
        }
        Ok(None) => ApiError(StatusCode::UNAUTHORIZED, ErrorReply::Unauthorized).into_response(),
        Err(error) => ApiError::from(error).into_response(),
    }
}

async fn login(State(server): State<Server>, Json(body): Json<LoginRequest>) -> Result<Json<LoginReply>, ApiError> {
    let db = &server.shared.db;
    let code = sqlx::query("DELETE FROM pairing_codes WHERE code_hash = ? AND expires_ms > ? RETURNING user_id")
        .bind(hash(&body.code).to_vec())
        .bind(now_ms())
        .fetch_optional(db)
        .await?;
    let Some(code) = code else {
        return Err(ApiError(StatusCode::UNAUTHORIZED, ErrorReply::Unauthorized));
    };
    let user: i64 = code.get("user_id");
    Ok(Json(accounts::issue_device(db, user, &body.name).await?))
}

async fn password_login(
    State(server): State<Server>,
    Json(body): Json<PasswordLoginRequest>,
) -> Result<Json<LoginReply>, ApiError> {
    if !server.config().auth.passwords {
        return Err(forbidden("passwords are turned off on this server; use `silt auth login` without --user"));
    }
    let user = match accounts::check_password(&server, body.user.trim(), &body.password).await {
        Ok(user) => user,
        Err(accounts::PasswordError::Database(error)) => return Err(error.into()),
        Err(accounts::PasswordError::TooManyFailures) => {
            return Err(ApiError(StatusCode::TOO_MANY_REQUESTS, ErrorReply::Unauthorized));
        }
    }
    .ok_or(ApiError(StatusCode::UNAUTHORIZED, ErrorReply::Unauthorized))?;
    Ok(Json(accounts::issue_device(server.db(), user.id, &body.name).await?))
}

async fn device_start(
    State(server): State<Server>,
    Json(body): Json<DeviceStartRequest>,
) -> Result<Json<DeviceStartReply>, ApiError> {
    let (device_code, user_code) = accounts::start_device(server.db(), &body.name).await?;
    Ok(Json(DeviceStartReply {
        verification_url: format!("{}/device?code={user_code}", server.config().public_url()),
        device_code,
        user_code,
        interval_secs: 2,
        expires_in_secs: u64::try_from(accounts::DEVICE_REQUEST_TTL_MS / 1000).unwrap_or(900),
    }))
}

async fn device_poll(
    State(server): State<Server>,
    Json(body): Json<DevicePollRequest>,
) -> Result<Json<DevicePollReply>, ApiError> {
    Ok(Json(accounts::poll_device(server.db(), &body.device_code).await?))
}

/// Signs the calling device out: its token stops working and its leases
/// are released.
async fn logout_device(State(server): State<Server>, Extension(caller): Extension<Caller>) -> Result<StatusCode, ApiError> {
    accounts::revoke_device(server.db(), caller.user, caller.device).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn listing(State(server): State<Server>, Extension(caller): Extension<Caller>) -> Result<Json<Listing>, ApiError> {
    let viewer = server.viewer(caller.user).await?;
    let mut families = Vec::new();
    for (id, shared) in server.rooms().await {
        if !viewer.sees_family(id.0) {
            continue;
        }
        let mut slot = shared.lock().await;
        if !slot.retired {
            families.push(FamilyInfo { id, seq: slot.room.seq(), view: slot.view(id, &viewer) });
        }
    }
    let rows = sqlx::query(
        "SELECT t.id, t.family_id, t.repo, t.created_ms, l.device_id, l.epoch, u.name AS owner
         FROM threads t
         JOIN users u ON u.id = t.owner_id
         LEFT JOIN leases l ON l.thread_id = t.id
         ORDER BY t.created_ms",
    )
    .fetch_all(&server.shared.db)
    .await?;
    let mut shared = collab::collaborators(&server.shared.db).await?;
    let threads = rows
        .iter()
        .filter(|row| viewer.sees_thread(ThreadId(id_from(row.get("id")))))
        .map(|row| ThreadInfo {
            id: ThreadId(id_from(row.get("id"))),
            family: FamilyId(id_from(row.get("family_id"))),
            repo: row.get("repo"),
            created_ms: row.get("created_ms"),
            lease: row.get::<Option<&[u8]>, _>("device_id").map(|device| Lease {
                device: DeviceId(id_from(device)),
                epoch: u64::try_from(row.get::<i64, _>("epoch")).unwrap_or_default(),
            }),
            owner: row.get("owner"),
            collaborators: shared.remove(&ThreadId(id_from(row.get("id")))).unwrap_or_default(),
        })
        .collect();
    let (deleted, purged) = server.lifecycle(&viewer).await?;
    Ok(Json(Listing { families, threads, deleted, purged }))
}

async fn register_thread(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Json(RegisterThread { thread, visibility }): Json<RegisterThread>,
) -> Result<StatusCode, ApiError> {
    let db = &server.shared.db;
    claim_family(&server, thread.family, caller).await?;
    sqlx::query("UPDATE families SET root_thread = ?, repo = ? WHERE id = ? AND root_thread IS NULL")
        .bind(id_bytes(thread.id.0))
        .bind(&thread.repo)
        .bind(id_bytes(thread.family.0))
        .execute(db)
        .await?;
    sqlx::query(
        "INSERT OR IGNORE INTO threads (id, family_id, repo, created_ms, owner_id, visibility) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id_bytes(thread.id.0))
    .bind(id_bytes(thread.family.0))
    .bind(&thread.repo)
    .bind(thread.created_ms)
    .bind(caller.user)
    .bind(visibility.map(|visibility| visibility.to_string()))
    .execute(db)
    .await?;
    orgs::place_project(db, &thread.repo, caller.user).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Moves the thread's lease to the calling device. Only the owner's
/// devices may; anyone else forks the thread instead (ADR 0010).
async fn take_lease(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    UrlPath(thread): UrlPath<String>,
) -> Result<Json<Lease>, ApiError> {
    let thread = ThreadId::from_str(&thread).map_err(|_| bad_request("invalid thread id"))?;
    let row = sqlx::query("SELECT family_id, owner_id FROM threads WHERE id = ?")
        .bind(id_bytes(thread.0))
        .fetch_optional(&server.shared.db)
        .await?
        .ok_or_else(not_found)?;
    if !server.viewer(caller.user).await?.sees_thread(thread) {
        return Err(not_found());
    }
    if row.get::<i64, _>("owner_id") != caller.user {
        return Err(forbidden("this thread belongs to another user; fork it to continue the work"));
    }
    let row = sqlx::query(
        "INSERT INTO leases (thread_id, device_id, epoch) VALUES (?, ?, 1)
         ON CONFLICT (thread_id) DO UPDATE SET device_id = excluded.device_id, epoch = leases.epoch + 1
         RETURNING epoch",
    )
    .bind(id_bytes(thread.0))
    .bind(id_bytes(caller.device.0))
    .fetch_one(&server.shared.db)
    .await?;
    Ok(Json(Lease { device: caller.device, epoch: u64::try_from(row.get::<i64, _>("epoch")).unwrap_or_default() }))
}

async fn push(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    UrlPath(family): UrlPath<String>,
    Json(body): Json<PushRequest>,
) -> Result<Json<PushReply>, ApiError> {
    let family = FamilyId::from_str(&family).map_err(|_| bad_request("invalid family id"))?;
    claim_family(&server, family, caller).await?;
    let mut slot = server.room_or_new(family).await;
    // Under the room's lock, so a purge can't finish between this filter
    // and the push.
    let pushed = server.without_purged(body.deltas).await;
    let room = &mut slot.room;
    let (known, new): (Vec<&Delta>, Vec<&Delta>) = pushed.iter().partition(|delta| room.position(delta.span.first()).is_some());
    let (new, claims) = collab::admit(&server, caller.user, family, new).await?;
    // Building on history the caller can't read would let them pull it in
    // their own thread; such parents count as missing, so the answer tells
    // hidden history and none apart no better (ADR 0016).
    let readable = server.viewer(caller.user).await?.readable_history(family.0, room);
    if let Some(delta) = new
        .iter()
        .find(|delta| delta.parents.ids().iter().any(|parent| room.graph().contains(*parent) && !readable.contains(*parent)))
    {
        return Err(ApiError(StatusCode::CONFLICT, ErrorReply::MissingParents { delta: delta.span.first() }));
    }
    let pushed: Vec<Delta> = known.into_iter().cloned().chain(new).collect();
    let prepared = room.prepare(pushed).map_err(|error| match error {
        PushError::Conflict(delta) => ApiError(StatusCode::CONFLICT, ErrorReply::Conflict { delta }),
        PushError::MissingParents(delta) => ApiError(StatusCode::CONFLICT, ErrorReply::MissingParents { delta }),
        PushError::Invalid { reason, .. } => bad_request(reason),
    })?;
    let stored: Vec<Delta> = prepared.fresh().to_vec();
    let mut tx = server.shared.db.begin().await?;
    collab::claim_actors(&mut tx, caller.user, &claims).await?;
    for delta in &stored {
        // Numbered after what is stored rather than by the room, which may
        // have skipped a stored delta that no longer applies.
        sqlx::query(
            "INSERT INTO deltas (family_id, seq, format, data) SELECT ?1, COALESCE(MAX(seq), 0) + 1, ?2, ?3 FROM deltas WHERE family_id = ?1",
        )
        .bind(id_bytes(family.0))
        .bind(silt_log::DELTA_FORMAT)
        .bind(silt_log::encode_stored(delta).map_err(ServerError::from)?)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    let outcome = prepared.commit();
    let index_rows = server.observe(family, &stored).await;
    drop(slot);
    server.write_index(&index_rows).await;
    if !stored.is_empty() {
        server.wake_pulls(family);
        if let Err(error) = server.notify(&stored, caller).await {
            tracing::warn!(%error, "could not write notifications");
        }
    }
    Ok(Json(PushReply { accepted: u64::try_from(outcome.accepted).unwrap_or(u64::MAX), seq: outcome.seq }))
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
    Extension(caller): Extension<Caller>,
    UrlPath(family): UrlPath<String>,
    Query(query): Query<PullQuery>,
) -> Result<Json<PullReply>, ApiError> {
    let family = FamilyId::from_str(&family).map_err(|_| bad_request("invalid family id"))?;
    if let FamilyAccess::Hidden = family_access(&server, family, caller.user).await? {
        return Err(not_found());
    }
    let viewer = server.viewer(caller.user).await?;
    let last = match &query.last {
        Some(last) => Some(DeltaId::from_str(last).map_err(|_| bad_request("invalid cursor"))?),
        None => None,
    };
    let wait = Duration::from_secs(query.wait).min(MAX_WAIT);
    let notified = server.waiter(family);
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let waiting = notified.notified();
        tokio::pin!(waiting);
        waiting.as_mut().enable();
        let deltas = {
            match server.room(family).await {
                Some(slot) => {
                    let room = &slot.room;
                    let readable = viewer.readable(family.0, room);
                    room.after(query.after, last)
                        .map_err(|error| match error {
                            PullError::UnknownCursor { .. } => ApiError(StatusCode::CONFLICT, ErrorReply::UnknownCursor),
                        })?
                        .filter(|(seq, _)| {
                            usize::try_from(*seq - 1).is_ok_and(|index| readable.get(index).copied().unwrap_or(false))
                        })
                        .take(PULL_LIMIT)
                        .map(|(seq, delta)| (seq, delta.clone()))
                        .collect()
                }
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

async fn set_visibility(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Json(body): Json<VisibilityRequest>,
) -> Result<StatusCode, ApiError> {
    let db = &server.shared.db;
    let visibility = body.visibility.to_string();
    match (body.repo, body.thread) {
        (Some(repo), None) => {
            // A repository nobody placed yet joins the caller's org only if
            // they administer the server, or work in it and may then decide
            // its visibility; a refused request must not leave it placed
            // (ADR 0016).
            if !orgs::is_placed(db, &repo).await? {
                let works_in_it = sqlx::query("SELECT 1 FROM threads WHERE repo = ? AND owner_id = ? LIMIT 1")
                    .bind(&repo)
                    .bind(caller.user)
                    .fetch_optional(db)
                    .await?
                    .is_some();
                if !works_in_it && !orgs::is_admin(db, caller.user).await? {
                    return Err(forbidden("this repository has no threads of yours on the server yet"));
                }
                if !orgs::may_place_project(db, caller.user).await? {
                    return Err(forbidden("only the project's org owners and server admins can change its visibility"));
                }
                orgs::place_project(db, &repo, caller.user).await?;
            }
            if !orgs::may_set_project(db, &repo, caller.user).await? {
                return Err(forbidden("only the project's org owners and server admins can change its visibility"));
            }
            sqlx::query("UPDATE projects SET visibility = ? WHERE repo = ?").bind(&visibility).bind(repo).execute(db).await?;
        }
        (None, Some(thread)) => {
            if !server.viewer(caller.user).await?.sees_thread(thread) {
                return Err(not_found());
            }
            let updated = sqlx::query("UPDATE threads SET visibility = ? WHERE id = ? AND owner_id = ?")
                .bind(&visibility)
                .bind(id_bytes(thread.0))
                .bind(caller.user)
                .execute(db)
                .await?
                .rows_affected();
            if updated == 0 {
                return Err(forbidden("only the thread's owner can change its visibility"));
            }
        }
        _ => return Err(bad_request("give either a repository or a thread")),
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Full-text search over the conversations of every family the caller can
/// see, with filters (ADR 0011).
async fn search(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<SearchReply>, ApiError> {
    let terms = fts_query(&query.q);
    if terms.is_empty() {
        return Ok(Json(SearchReply { hits: Vec::new() }));
    }
    // Only the caller's threads are matched at all, and only a page of them,
    // so a common word doesn't fetch the whole index.
    let viewer = server.viewer(caller.user).await?;
    let visible: Vec<String> = viewer.visible_threads().map(|thread| hex_upper(&id_bytes(thread.0))).collect();
    let visible = serde_json::to_string(&visible).map_err(ServerError::from)?;
    let limit = i64::from(query.limit.unwrap_or(SEARCH_LIMIT).min(500));
    let rows = sqlx::query(
        "SELECT s.thread_id, s.kind, snippet(search, 0, '[', ']', '…', 12) AS snippet, CAST(s.time_ms AS INTEGER) AS time_ms,
                t.repo, u.name AS owner, a.branch,
                CASE WHEN COALESCE(a.merged, 0) = 1 THEN 'merged'
                     WHEN COALESCE(a.last_ms, 0) >= ?1 THEN 'active' ELSE 'idle' END AS status
         FROM search s
         JOIN threads t ON t.id = s.thread_id
         JOIN families f ON f.id = t.family_id
         JOIN users u ON u.id = t.owner_id
         LEFT JOIN thread_activity a ON a.thread_id = t.id
         WHERE search MATCH ?2
           AND (?3 IS NULL OR t.repo = ?3)
           AND (?4 IS NULL OR u.name = ?4)
           AND (?5 IS NULL OR a.branch = ?5)
           AND (?6 IS NULL OR CAST(s.time_ms AS INTEGER) >= ?6)
           AND (?7 IS NULL OR CAST(s.time_ms AS INTEGER) <= ?7)
           AND (?8 IS NULL OR EXISTS (SELECT 1 FROM touches th
                                      WHERE th.thread_id = t.id AND (th.path = ?8 OR th.path LIKE ?8 || '/%')))
           AND (?9 IS NULL OR status = ?9)
           AND hex(s.thread_id) IN (SELECT value FROM json_each(?10))
         ORDER BY rank
         LIMIT ?11",
    )
    .bind(now_ms() - ACTIVE_MS)
    .bind(terms)
    .bind(&query.repo)
    .bind(&query.author)
    .bind(&query.branch)
    .bind(query.since)
    .bind(query.until)
    .bind(query.file.as_deref().map(|file| file.trim_end_matches('/')))
    .bind(&query.status)
    .bind(visible)
    .bind(limit)
    .fetch_all(&server.shared.db)
    .await?;
    let hits = rows
        .iter()
        .map(|row| SearchHit {
            thread: ThreadId(id_from(row.get("thread_id"))),
            repo: row.get("repo"),
            owner: row.get("owner"),
            status: row.get("status"),
            branch: row.get("branch"),
            kind: row.get("kind"),
            snippet: row.get("snippet"),
            time_ms: row.get("time_ms"),
        })
        .collect();
    Ok(Json(SearchReply { hits }))
}

/// Visible threads of a repository that changed a path, most recently
/// active first.
async fn in_flight(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    Query(query): Query<InFlightQuery>,
) -> Result<Json<InFlightReply>, ApiError> {
    let path = query.path.trim_matches('/');
    let rows = sqlx::query(
        "SELECT t.id, u.name AS owner, th.path, th.last_ms,
                CASE WHEN COALESCE(a.merged, 0) = 1 THEN 'merged'
                     WHEN COALESCE(a.last_ms, 0) >= ?1 THEN 'active' ELSE 'idle' END AS status
         FROM touches th
         JOIN threads t ON t.id = th.thread_id
         JOIN families f ON f.id = t.family_id
         JOIN users u ON u.id = t.owner_id
         LEFT JOIN thread_activity a ON a.thread_id = t.id
         WHERE t.repo = ?2
           AND (?3 = '' OR th.path = ?3 OR th.path LIKE ?3 || '/%')
         ORDER BY th.last_ms DESC",
    )
    .bind(now_ms() - ACTIVE_MS)
    .bind(&query.repo)
    .bind(path)
    .fetch_all(&server.shared.db)
    .await?;
    let viewer = server.viewer(caller.user).await?;
    let mut threads: Vec<InFlightThread> = Vec::new();
    for row in &rows {
        let thread = ThreadId(id_from(row.get("id")));
        if !viewer.sees_thread(thread) {
            continue;
        }
        let path: String = row.get("path");
        match threads.iter_mut().find(|found| found.thread == thread) {
            Some(found) => found.paths.push(path),
            None => threads.push(InFlightThread {
                thread,
                owner: row.get("owner"),
                status: row.get("status"),
                paths: vec![path],
                last_ms: row.get("last_ms"),
            }),
        }
    }
    Ok(Json(InFlightReply { threads }))
}

/// Free text as an FTS5 query of quoted prefix terms, so input can't be
/// read as query syntax.
fn fts_query(text: &str) -> String {
    text.split_whitespace()
        .map(|word| word.replace('"', ""))
        .filter(|word| !word.is_empty())
        .map(|word| format!("\"{word}\"*"))
        .collect::<Vec<_>>()
        .join(" ")
}

async fn put_blob(State(server): State<Server>, UrlPath(hash_hex): UrlPath<String>, body: Bytes) -> Result<StatusCode, ApiError> {
    if BlobStore::hash(&body).to_string() != hash_hex {
        return Err(bad_request("blob content does not match its hash"));
    }
    server.shared.blobs.put(&body).await.map_err(ServerError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// A blob, for someone who may read a delta that refers to it. Knowing a
/// blob's hash is not enough: it could be a guess at a file's content, or
/// left over from access since taken away.
async fn get_blob(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    UrlPath(hash_hex): UrlPath<String>,
) -> Result<Vec<u8>, ApiError> {
    let hash = BlobHash::from_str(&hash_hex).map_err(|_| bad_request("invalid blob hash"))?;
    let references = server.shared.index.lock().await.blob_references(hash);
    let viewer = server.viewer(caller.user).await?;
    let mut readable = false;
    for (family, id) in references {
        if let Some(slot) = server.room(family).await
            && viewer.readable_history(family.0, &slot.room).contains(id)
        {
            readable = true;
            break;
        }
    }
    if !readable {
        return Err(ApiError(StatusCode::NOT_FOUND, ErrorReply::NotFound));
    }
    server.shared.blobs.get(hash).await.map_err(|_| ApiError(StatusCode::NOT_FOUND, ErrorReply::NotFound))
}

impl Server {
    /// The family's room, locked, unless a purge dropped it.
    pub(crate) async fn room(&self, family: FamilyId) -> Option<OwnedMutexGuard<RoomSlot>> {
        let shared = self.shared.rooms.read().await.get(&family).cloned()?;
        let slot = shared.lock_owned().await;
        (!slot.retired).then_some(slot)
    }

    /// The family's room, locked, made if there is none.
    async fn room_or_new(&self, family: FamilyId) -> OwnedMutexGuard<RoomSlot> {
        loop {
            let found = self.shared.rooms.read().await.get(&family).cloned();
            let shared = match found {
                Some(shared) => shared,
                None => Arc::clone(
                    self.shared
                        .rooms
                        .write()
                        .await
                        .entry(family)
                        .or_insert_with(|| Arc::new(Mutex::new(RoomSlot::new(Room::new(self.shared.generation))))),
                ),
            };
            let slot = shared.lock_owned().await;
            if !slot.retired {
                return slot;
            }
        }
    }

    /// Every room, unlocked, by family.
    pub(crate) async fn rooms(&self) -> Vec<(FamilyId, SharedRoom)> {
        let mut rooms: Vec<(FamilyId, SharedRoom)> =
            self.shared.rooms.read().await.iter().map(|(family, room)| (*family, Arc::clone(room))).collect();
        rooms.sort_by_key(|(family, _)| *family);
        rooms
    }

    /// What `user` can see right now: the snapshot taken since nothing it is
    /// worked out from last changed, else a new one.
    async fn viewer(&self, user: i64) -> Result<Arc<Viewer>, sqlx::Error> {
        let epoch: i64 = sqlx::query_scalar("SELECT value FROM visibility_epoch").fetch_one(&self.shared.db).await?;
        let key = VisibilityAt { epoch, forks: self.shared.index.lock().await.forks_version };
        let cached =
            self.shared.viewers.lock().await.get(&user).filter(|(at, _)| *at == key).map(|(_, viewer)| Arc::clone(viewer));
        if let Some(viewer) = cached {
            return Ok(viewer);
        }
        // Whatever changes from here on bumps the key past this one, so a
        // snapshot that already saw some of it is replaced on the next call.
        let forks = self.shared.index.lock().await.forks().clone();
        let serial = self.shared.viewer_serial.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let viewer = Arc::new(Viewer::load(&self.shared.db, user, &forks, serial).await?);
        self.shared.viewers.lock().await.insert(user, (key, Arc::clone(&viewer)));
        Ok(viewer)
    }

    /// Takes stored deltas into the index, returning the rows to write.
    async fn observe(&self, family: FamilyId, deltas: &[Delta]) -> Vec<index::Row> {
        let mut index = self.shared.index.lock().await;
        deltas.iter().flat_map(|delta| index.observe(family, delta)).collect()
    }

    /// Writes index rows. The index is rebuilt from the deltas at every
    /// start, so a failed write only leaves search short until then.
    async fn write_index(&self, rows: &[index::Row]) {
        let written = async {
            let mut tx = self.shared.db.begin().await?;
            index::write(&mut tx, rows).await?;
            tx.commit().await
        };
        if let Err(error) = written.await {
            tracing::warn!(%error, "could not write the search index; it is rebuilt at the next start");
        }
    }

    fn waiter(&self, family: FamilyId) -> Waiter<'_> {
        self.shared.waiters.subscribe(family)
    }

    fn wake_pulls(&self, family: FamilyId) {
        self.shared.waiters.wake(family);
    }
}

/// Pulls waiting for pushes, by family. Holds only families someone is
/// waiting on.
#[derive(Default)]
struct Waiters(std::sync::Mutex<HashMap<FamilyId, Arc<Notify>>>);

impl Waiters {
    fn subscribe(&self, family: FamilyId) -> Waiter<'_> {
        let notify = self.0.lock().expect("lock").entry(family).or_insert_with(|| Arc::new(Notify::new())).clone();
        Waiter { waiters: &self.0, family, notify }
    }

    fn wake(&self, family: FamilyId) {
        if let Some(notify) = self.0.lock().expect("lock").get(&family) {
            notify.notify_waiters();
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.0.lock().expect("lock").len()
    }
}

/// A pull waiting for pushes to its family. The last one to leave removes
/// the family's entry.
struct Waiter<'a> {
    waiters: &'a std::sync::Mutex<HashMap<FamilyId, Arc<Notify>>>,
    family: FamilyId,
    notify: Arc<Notify>,
}

impl std::ops::Deref for Waiter<'_> {
    type Target = Notify;

    fn deref(&self) -> &Notify {
        &self.notify
    }
}

impl Drop for Waiter<'_> {
    fn drop(&mut self) {
        let mut waiters = self.waiters.lock().expect("lock");
        // The map's copy and this one: nobody else is waiting.
        if Arc::strong_count(&self.notify) == 2 {
            waiters.remove(&self.family);
        }
    }
}

pub(crate) fn hash(text: &str) -> [u8; 32] {
    *blake3::hash(text.as_bytes()).as_bytes()
}

pub(crate) fn id_bytes(id: u128) -> Vec<u8> {
    id.to_be_bytes().to_vec()
}

/// Bytes as SQLite's `hex()` writes them.
fn hex_upper(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

/// An id stored by `id_bytes`. Ids are always 16 bytes; anything shorter
/// is padded rather than allowed to panic.
pub(crate) fn id_from(bytes: &[u8]) -> u128 {
    let mut buffer = [0u8; 16];
    let len = bytes.len().min(16);
    buffer[16 - len..].copy_from_slice(&bytes[..len]);
    u128::from_be_bytes(buffer)
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_are_forgotten_once_nobody_waits() {
        let waiters = Waiters::default();
        let first = waiters.subscribe(FamilyId(1));
        let second = waiters.subscribe(FamilyId(1));
        waiters.wake(FamilyId(2));
        assert_eq!(waiters.len(), 1, "waking a family nobody waits on remembers nothing");
        drop(first);
        assert_eq!(waiters.len(), 1, "someone still waits");
        drop(second);
        assert_eq!(waiters.len(), 0);
    }
}
