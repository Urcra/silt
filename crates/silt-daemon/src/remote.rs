use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use silt_log::Delta;
use silt_proto::{
    DevicePollReply, DevicePollRequest, DeviceStartReply, DeviceStartRequest, ErrorReply, GENERATION_HEADER, InFlightQuery,
    InFlightReply, InboxRead, InboxReply, Lease, Listing, LoginReply, LoginRequest, OrgChange, OrgsReply, PROTOCOL,
    PROTOCOL_HEADER, PasswordLoginRequest, PresenceReply, PresenceUpdate, PullReply, PushRequest, RegisterThread, SearchQuery,
    SearchReply, ShareRequest, ThreadInfo, Visibility, VisibilityRequest,
};
use silt_sync::Outcome;
use silt_types::{BlobHash, DeltaId, DeviceId, FamilyId, ThreadId};

/// This machine's sign-in to one server, saved by `silt auth login`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteConfig {
    pub url: String,
    pub token: String,
    pub device: DeviceId,
    pub name: String,
    /// The user this device belongs to on the server.
    #[serde(default)]
    pub user: String,
}

#[derive(Debug, thiserror::Error)]
pub enum RemoteError {
    #[error("could not reach the Silt server")]
    Http(#[from] reqwest::Error),
    #[error("the server speaks protocol {supported:?}, this daemon speaks {PROTOCOL}; update one of them")]
    ProtocolMismatch { supported: Vec<u32> },
    #[error("the server refused: {0}")]
    Refused(ErrorReply),
    #[error("could not save the server settings")]
    Io(#[from] io::Error),
    #[error("invalid server settings")]
    Json(#[from] serde_json::Error),
}

/// Where this machine keeps its sign-ins, one per server (ADR 0032).
pub fn sign_ins_path(data_dir: &Path) -> PathBuf {
    data_dir.join("servers.json")
}

/// The one sign-in kept before there could be several.
fn single_sign_in_path(data_dir: &Path) -> PathBuf {
    data_dir.join("server.json")
}

/// This machine's sign-ins, the default first.
pub fn load_sign_ins(data_dir: &Path) -> Vec<RemoteConfig> {
    if let Ok(text) = fs::read_to_string(sign_ins_path(data_dir)) {
        return serde_json::from_str(&text).unwrap_or_default();
    }
    let Ok(text) = fs::read_to_string(single_sign_in_path(data_dir)) else {
        return Vec::new();
    };
    serde_json::from_str(&text)
        .ok()
        .or_else(|| serde_json::from_str::<LegacyConfig>(&text).ok().map(RemoteConfig::from))
        .into_iter()
        .collect()
}

/// The default sign-in.
pub fn load_config(data_dir: &Path) -> Option<RemoteConfig> {
    load_sign_ins(data_dir).into_iter().next()
}

/// Keeps `config` as the sign-in to its server, in place of one before it
/// there; a new server goes last, so the default stays the default.
pub fn keep_sign_in(data_dir: &Path, config: &RemoteConfig) -> Result<(), RemoteError> {
    let mut sign_ins = load_sign_ins(data_dir);
    match sign_ins.iter_mut().find(|kept| kept.url == config.url) {
        Some(kept) => kept.clone_from(config),
        None => sign_ins.push(config.clone()),
    }
    save_sign_ins(data_dir, &sign_ins)
}

/// Forgets the sign-in to `url`, returning it if there was one.
pub fn forget_sign_in(data_dir: &Path, url: &str) -> Result<Option<RemoteConfig>, RemoteError> {
    let mut sign_ins = load_sign_ins(data_dir);
    let Some(at) = sign_ins.iter().position(|kept| kept.url == url.trim_end_matches('/')) else {
        return Ok(None);
    };
    let forgotten = sign_ins.remove(at);
    save_sign_ins(data_dir, &sign_ins)?;
    Ok(Some(forgotten))
}

/// Settings saved before ids were written as hex, with the device as a
/// number.
#[derive(Deserialize)]
struct LegacyConfig {
    url: String,
    token: String,
    device: u128,
    name: String,
    #[serde(default)]
    user: String,
}

impl From<LegacyConfig> for RemoteConfig {
    fn from(legacy: LegacyConfig) -> Self {
        Self { url: legacy.url, token: legacy.token, device: DeviceId(legacy.device), name: legacy.name, user: legacy.user }
    }
}

/// Saves the sign-ins, with the devices' tokens, for this user's eyes only.
fn save_sign_ins(data_dir: &Path, sign_ins: &[RemoteConfig]) -> Result<(), RemoteError> {
    crate::paths::make_private(data_dir)?;
    silt_os::write_private(&sign_ins_path(data_dir), &serde_json::to_vec_pretty(sign_ins)?)?;
    match fs::remove_file(single_sign_in_path(data_dir)) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}

fn post(url: &str, path: &str) -> reqwest::RequestBuilder {
    reqwest::Client::new().post(format!("{}{path}", url.trim_end_matches('/'))).header(PROTOCOL_HEADER, PROTOCOL.to_string())
}

fn signed_in(url: &str, name: &str, reply: LoginReply) -> RemoteConfig {
    RemoteConfig {
        url: url.trim_end_matches('/').to_owned(),
        token: reply.token,
        device: reply.device,
        name: name.to_owned(),
        user: reply.user,
    }
}

/// Exchanges a pairing code for a device token.
pub async fn login(url: &str, code: &str, name: &str) -> Result<RemoteConfig, RemoteError> {
    let request = LoginRequest { code: code.to_owned(), name: name.to_owned() };
    let reply: LoginReply = checked(post(url, "/v1/login").json(&request).send().await?).await?.json().await?;
    Ok(signed_in(url, name, reply))
}

/// Signs the device in with a local account's password.
pub async fn login_password(url: &str, user: &str, password: &str, name: &str) -> Result<RemoteConfig, RemoteError> {
    let request = PasswordLoginRequest { user: user.to_owned(), password: password.to_owned(), name: name.to_owned() };
    let reply: LoginReply = checked(post(url, "/v1/login/password").json(&request).send().await?).await?.json().await?;
    Ok(signed_in(url, name, reply))
}

/// Starts signing the device in through the browser (ADR 0013).
pub async fn device_start(url: &str, name: &str) -> Result<DeviceStartReply, RemoteError> {
    let request = DeviceStartRequest { name: name.to_owned() };
    Ok(checked(post(url, "/v1/device/start").json(&request).send().await?).await?.json().await?)
}

/// Waits until the person approves the device in the browser, polling
/// every `interval`; `None` when the request expired or was denied.
pub async fn device_wait(
    url: &str,
    name: &str,
    device_code: &str,
    interval: Duration,
) -> Result<Option<RemoteConfig>, RemoteError> {
    let request = DevicePollRequest { device_code: device_code.to_owned() };
    loop {
        let reply: DevicePollReply = checked(post(url, "/v1/device/poll").json(&request).send().await?).await?.json().await?;
        match reply {
            DevicePollReply::Pending => tokio::time::sleep(interval).await,
            DevicePollReply::Approved { login } => return Ok(Some(signed_in(url, name, login))),
            DevicePollReply::Gone => return Ok(None),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Remote {
    pub config: RemoteConfig,
    http: reqwest::Client,
}

/// The longest any call to the server may take, unless it says otherwise:
/// a server that stops answering must not hold up what waits on it.
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Blobs can be large and links slow.
const BLOB_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Teammates' drafts go into an agent's prompt while the hook waits.
const DRAFTS_TIMEOUT: Duration = Duration::from_secs(3);

impl Remote {
    pub fn new(config: RemoteConfig) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(CALL_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { config, http }
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.config.url))
            .header(PROTOCOL_HEADER, PROTOCOL.to_string())
            .bearer_auth(&self.config.token)
    }

    /// The family's repository on the server's git mirror, and the headers
    /// git must send with every request to it.
    pub fn git_mirror(&self, family: FamilyId) -> (String, Vec<String>) {
        (
            format!("{}/git/{family}.git", self.config.url),
            vec![format!("Authorization: Bearer {}", self.config.token), format!("{PROTOCOL_HEADER}: {PROTOCOL}")],
        )
    }

    pub async fn listing(&self) -> Result<Listing, RemoteError> {
        let response = self.request(reqwest::Method::GET, "/v1/listing").send().await?;
        Ok(checked(response).await?.json().await?)
    }

    pub async fn register_thread(
        &self,
        thread: &ThreadInfo,
        visibility: Option<Visibility>,
        origin_url: Option<String>,
    ) -> Result<(), RemoteError> {
        self.register(RegisterThread { thread: thread.clone(), visibility, origin_url, recovery_of: None }).await
    }

    pub async fn register(&self, body: RegisterThread) -> Result<(), RemoteError> {
        let response = self.request(reqwest::Method::POST, "/v1/threads").json(&body).send().await?;
        checked(response).await?;
        Ok(())
    }

    pub async fn thread_states(&self) -> Result<Vec<silt_proto::ThreadState>, RemoteError> {
        let response = self.request(reqwest::Method::GET, "/v1/me/threads").send().await?;
        Ok(checked(response).await?.json().await?)
    }

    /// Merges `state` into the server's, answering with how it then stands.
    pub async fn change_thread_state(&self, state: &silt_proto::ThreadState) -> Result<silt_proto::ThreadState, RemoteError> {
        let path = format!("/v1/me/threads/{}", state.thread);
        let response = self.request(reqwest::Method::PATCH, &path).json(state).send().await?;
        Ok(checked(response).await?.json().await?)
    }

    /// The artifacts the person pinned on any of their machines (ADR 0075).
    pub async fn artifact_pins(&self) -> Result<Vec<silt_proto::view::ArtifactPin>, RemoteError> {
        let response = self.request(reqwest::Method::GET, "/v1/me/artifact-pins").send().await?;
        Ok(checked(response).await?.json().await?)
    }

    pub async fn pin_artifact(&self, pin: &silt_proto::view::ArtifactPin) -> Result<(), RemoteError> {
        let response = self.request(reqwest::Method::PUT, "/v1/me/artifact-pins").json(pin).send().await?;
        checked(response).await?;
        Ok(())
    }

    pub async fn seen(&self) -> Result<Vec<silt_proto::MrSeen>, RemoteError> {
        let response = self.request(reqwest::Method::GET, "/v1/me/seen").send().await?;
        Ok(checked(response).await?.json().await?)
    }

    pub async fn mark_seen(&self, mr: &str, mark: &silt_proto::SeenMark) -> Result<(), RemoteError> {
        let response = self.request(reqwest::Method::PUT, &format!("/v1/me/seen/{mr}")).json(mark).send().await?;
        checked(response).await?;
        Ok(())
    }

    pub async fn thread_seen(&self) -> Result<Vec<silt_proto::ThreadSeen>, RemoteError> {
        let response = self.request(reqwest::Method::GET, "/v1/me/thread-seen").send().await?;
        Ok(checked(response).await?.json().await?)
    }

    pub async fn mark_thread_seen(&self, thread: ThreadId, mark: &silt_proto::SeenMark) -> Result<(), RemoteError> {
        let response = self.request(reqwest::Method::PUT, &format!("/v1/me/thread-seen/{thread}")).json(mark).send().await?;
        checked(response).await?;
        Ok(())
    }

    /// This person's own design systems as the server keeps them (ADR 0071).
    pub async fn design_systems(&self) -> Result<Vec<silt_proto::DesignSystemCopy>, RemoteError> {
        let response = self.request(reqwest::Method::GET, "/v1/me/design-systems").send().await?;
        Ok(checked(response).await?.json().await?)
    }

    pub async fn put_design_system(&self, copy: &silt_proto::DesignSystemCopy) -> Result<(), RemoteError> {
        let path = format!("/v1/me/design-systems/{}", copy.name);
        let response = self.request(reqwest::Method::PUT, &path).json(copy).send().await?;
        checked(response).await?;
        Ok(())
    }

    pub async fn projects(&self) -> Result<silt_proto::ProjectsReply, RemoteError> {
        let response = self.request(reqwest::Method::GET, "/v1/projects").send().await?;
        Ok(checked(response).await?.json().await?)
    }

    pub async fn take_lease(&self, thread: ThreadId) -> Result<Lease, RemoteError> {
        let response = self.request(reqwest::Method::POST, &format!("/v1/threads/{thread}/lease")).send().await?;
        Ok(checked(response).await?.json().await?)
    }

    /// Moves the thread to the trash, out of it, or purges it for good
    /// (`delete`, `restore` or `purge`).
    pub async fn thread_lifecycle(&self, thread: ThreadId, action: &str) -> Result<(), RemoteError> {
        let response = self.request(reqwest::Method::POST, &format!("/v1/threads/{thread}/{action}")).send().await?;
        checked(response).await?;
        Ok(())
    }

    pub async fn presence(&self, update: &PresenceUpdate) -> Result<PresenceReply, RemoteError> {
        Ok(checked(self.request(reqwest::Method::POST, "/v1/presence").json(update).send().await?).await?.json().await?)
    }

    pub async fn present(&self, thread: ThreadId) -> Result<PresenceReply, RemoteError> {
        let path = format!("/v1/presence?thread={thread}");
        let request = self.request(reqwest::Method::GET, &path).timeout(DRAFTS_TIMEOUT);
        Ok(checked(request.send().await?).await?.json().await?)
    }

    pub async fn notification_settings(&self) -> Result<Vec<silt_proto::KindSetting>, RemoteError> {
        Ok(checked(self.request(reqwest::Method::GET, "/v1/me/notifications").send().await?).await?.json().await?)
    }

    pub async fn change_notification_setting(
        &self,
        setting: &silt_proto::KindSetting,
    ) -> Result<Vec<silt_proto::KindSetting>, RemoteError> {
        let response = self.request(reqwest::Method::PUT, "/v1/me/notifications").json(setting).send().await?;
        Ok(checked(response).await?.json().await?)
    }

    pub async fn pair_phone(&self) -> Result<silt_proto::view::PhonePairing, RemoteError> {
        Ok(checked(self.request(reqwest::Method::POST, "/v1/pair").send().await?).await?.json().await?)
    }

    pub async fn devices(&self) -> Result<Vec<silt_proto::view::DeviceView>, RemoteError> {
        Ok(checked(self.request(reqwest::Method::GET, "/v1/devices").send().await?).await?.json().await?)
    }

    pub async fn change_device(
        &self,
        id: &str,
        change: &silt_proto::view::DeviceChange,
    ) -> Result<silt_proto::view::DeviceView, RemoteError> {
        let response = self.request(reqwest::Method::PATCH, &format!("/v1/devices/{id}")).json(change).send().await?;
        Ok(checked(response).await?.json().await?)
    }

    pub async fn revoke_device(&self, id: &str) -> Result<(), RemoteError> {
        checked(self.request(reqwest::Method::POST, &format!("/v1/devices/{id}/revoke")).send().await?).await?;
        Ok(())
    }

    /// Has the device running the thread's agent do `relay`, and gives its answer.
    pub async fn relay(&self, thread: ThreadId, relay: &silt_proto::RelayRequest) -> Result<serde_json::Value, RemoteError> {
        let path = format!("/v1/threads/{thread}/relay");
        let request = self.request(reqwest::Method::POST, &path).json(relay).timeout(Duration::from_secs(45));
        Ok(checked(request.send().await?).await?.json().await?)
    }

    /// The commands waiting for this device, waiting up to `wait` for one.
    pub async fn control(&self, wait: Duration) -> Result<silt_proto::ControlReply, RemoteError> {
        let path = format!("/v1/control?wait_ms={}", wait.as_millis());
        let request = self.request(reqwest::Method::GET, &path).timeout(wait + Duration::from_secs(15));
        Ok(checked(request.send().await?).await?.json().await?)
    }

    pub async fn answer_command(&self, id: i64, answer: &silt_proto::CommandAnswer) -> Result<(), RemoteError> {
        checked(self.request(reqwest::Method::POST, &format!("/v1/control/{id}")).json(answer).send().await?).await?;
        Ok(())
    }

    /// What this device's agent has said so far of command `id`'s answer.
    pub async fn command_progress(&self, id: i64, text: &str) -> Result<(), RemoteError> {
        let request = self.request(reqwest::Method::POST, &format!("/v1/control/{id}/progress"));
        checked(request.json(&serde_json::json!({ "text": text })).send().await?).await?;
        Ok(())
    }

    pub async fn report_agents(&self, report: &silt_proto::AgentsReport) -> Result<(), RemoteError> {
        checked(self.request(reqwest::Method::PUT, "/v1/control/agents").json(report).send().await?).await?;
        Ok(())
    }

    pub async fn links(&self, thread: ThreadId) -> Result<Vec<silt_proto::view::ThreadLink>, RemoteError> {
        let response = self.request(reqwest::Method::GET, &format!("/v1/threads/{thread}/links")).send().await?;
        Ok(checked(response).await?.json().await?)
    }

    pub async fn make_link(
        &self,
        thread: ThreadId,
        new: &silt_proto::view::NewLink,
    ) -> Result<silt_proto::view::ThreadLink, RemoteError> {
        let response = self.request(reqwest::Method::POST, &format!("/v1/threads/{thread}/links")).json(new).send().await?;
        Ok(checked(response).await?.json().await?)
    }

    pub async fn revoke_link(&self, thread: ThreadId, link: i64) -> Result<(), RemoteError> {
        let path = format!("/v1/threads/{thread}/links/{link}/revoke");
        checked(self.request(reqwest::Method::POST, &path).send().await?).await?;
        Ok(())
    }

    pub async fn share(&self, thread: ThreadId, user: &str, remove: bool) -> Result<(), RemoteError> {
        let request = ShareRequest { user: user.to_owned(), remove };
        let response =
            self.request(reqwest::Method::POST, &format!("/v1/threads/{thread}/collaborators")).json(&request).send().await?;
        checked(response).await?;
        Ok(())
    }

    /// Signs this device out on the server.
    pub async fn logout(&self) -> Result<(), RemoteError> {
        checked(self.request(reqwest::Method::DELETE, "/v1/device").send().await?).await?;
        Ok(())
    }

    pub async fn inbox(&self, all: bool) -> Result<InboxReply, RemoteError> {
        let path = if all { "/v1/inbox?all=true" } else { "/v1/inbox" };
        Ok(checked(self.request(reqwest::Method::GET, path).send().await?).await?.json().await?)
    }

    pub async fn inbox_read(&self, read: &InboxRead) -> Result<(), RemoteError> {
        checked(self.request(reqwest::Method::POST, "/v1/inbox/read").json(read).send().await?).await?;
        Ok(())
    }

    pub async fn orgs(&self) -> Result<OrgsReply, RemoteError> {
        Ok(checked(self.request(reqwest::Method::GET, "/v1/orgs").send().await?).await?.json().await?)
    }

    pub async fn org_change(&self, change: &OrgChange) -> Result<(), RemoteError> {
        checked(self.request(reqwest::Method::POST, "/v1/orgs/change").json(change).send().await?).await?;
        Ok(())
    }

    pub async fn set_visibility(
        &self,
        repo: Option<String>,
        thread: Option<ThreadId>,
        visibility: Visibility,
    ) -> Result<(), RemoteError> {
        let response = self
            .request(reqwest::Method::PUT, "/v1/visibility")
            .json(&VisibilityRequest { repo, thread, visibility })
            .send()
            .await?;
        checked(response).await?;
        Ok(())
    }

    /// Moves a chat into the project its folder became (ADR 0059).
    pub async fn move_chat(&self, thread: ThreadId, body: &silt_proto::MoveChat) -> Result<(), RemoteError> {
        let path = format!("/v1/threads/{thread}/project");
        let response = self.request(reqwest::Method::POST, &path).json(body).send().await?;
        checked(response).await?;
        Ok(())
    }

    pub async fn search(&self, query: &SearchQuery) -> Result<SearchReply, RemoteError> {
        let response = self.request(reqwest::Method::GET, "/v1/search").query(query).send().await?;
        Ok(checked(response).await?.json().await?)
    }

    pub async fn in_flight(&self, query: &InFlightQuery) -> Result<InFlightReply, RemoteError> {
        let response = self.request(reqwest::Method::GET, "/v1/in-flight").query(query).send().await?;
        Ok(checked(response).await?.json().await?)
    }

    pub async fn put_blob(&self, hash: BlobHash, bytes: Vec<u8>) -> Result<(), RemoteError> {
        let response =
            self.request(reqwest::Method::PUT, &format!("/v1/blobs/{hash}")).timeout(BLOB_TIMEOUT).body(bytes).send().await?;
        checked(response).await?;
        Ok(())
    }

    pub async fn get_blob(&self, hash: BlobHash) -> Result<Vec<u8>, RemoteError> {
        let response = self.request(reqwest::Method::GET, &format!("/v1/blobs/{hash}")).timeout(BLOB_TIMEOUT).send().await?;
        Ok(checked(response).await?.bytes().await?.to_vec())
    }

    /// Pushes a batch; returns the server's generation and how it answered.
    pub async fn push(&self, family: FamilyId, deltas: Vec<Delta>) -> (Option<u64>, Outcome, Option<RemoteError>) {
        let response = self
            .request(reqwest::Method::POST, &format!("/v1/families/{family}/deltas"))
            .json(&PushRequest { deltas })
            .send()
            .await;
        match response {
            Ok(response) => {
                let generation = generation(&response);
                match checked(response).await {
                    Ok(_) => (generation, Outcome::Pushed, None),
                    Err(error) => (generation, outcome_of(&error), Some(error)),
                }
            }
            Err(error) => (None, Outcome::Failed, Some(error.into())),
        }
    }

    pub async fn pull(
        &self,
        family: FamilyId,
        after: u64,
        last: Option<DeltaId>,
        wait: Duration,
    ) -> (Option<u64>, Outcome, Option<RemoteError>) {
        let mut query = vec![("after", after.to_string()), ("wait", wait.as_secs().to_string())];
        if let Some(last) = last {
            query.push(("last", last.to_string()));
        }
        let response = self
            .request(reqwest::Method::GET, &format!("/v1/families/{family}/deltas"))
            .query(&query)
            .timeout(wait + Duration::from_secs(15))
            .send()
            .await;
        match response {
            Ok(response) => {
                let generation = generation(&response);
                match checked(response).await {
                    Ok(response) => match response.json::<PullReply>().await {
                        Ok(reply) => (generation, Outcome::Pulled(reply.deltas), None),
                        Err(error) => (generation, Outcome::Failed, Some(error.into())),
                    },
                    Err(error) => (generation, outcome_of(&error), Some(error)),
                }
            }
            Err(error) => (None, Outcome::Failed, Some(error.into())),
        }
    }
}

fn generation(response: &reqwest::Response) -> Option<u64> {
    response.headers().get(GENERATION_HEADER).and_then(|value| value.to_str().ok()).and_then(|value| value.parse().ok())
}

fn outcome_of(error: &RemoteError) -> Outcome {
    match error {
        RemoteError::Refused(ErrorReply::UnknownCursor) => Outcome::UnknownCursor,
        RemoteError::Refused(ErrorReply::MissingParents { .. }) => Outcome::MissingParents,
        RemoteError::Refused(ErrorReply::Conflict { delta }) => Outcome::Conflict(*delta),
        _ => Outcome::Failed,
    }
}

/// Turns error statuses into [`RemoteError`]s.
async fn checked(response: reqwest::Response) -> Result<reqwest::Response, RemoteError> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let reply: ErrorReply =
        serde_json::from_str(&body).unwrap_or_else(|_| ErrorReply::Invalid { message: format!("{status}: {}", body.trim()) });
    Err(match reply {
        ErrorReply::ProtocolMismatch { supported } => RemoteError::ProtocolMismatch { supported },
        other => RemoteError::Refused(other),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_saved_with_a_numeric_device_still_load() {
        let dir = tempfile::tempdir().expect("temp dir");
        let device = u128::MAX - 5;
        let legacy = format!(r#"{{"url":"https://silt.example","token":"t","device":{device},"name":"laptop"}}"#);
        fs::write(single_sign_in_path(dir.path()), legacy).expect("writes");
        assert_eq!(load_config(dir.path()).expect("loads").device, DeviceId(device));
        keep_sign_in(dir.path(), &load_config(dir.path()).expect("loads")).expect("saves");
        assert!(fs::read_to_string(sign_ins_path(dir.path())).expect("reads").contains(&format!("\"{device:032x}\"")));
        assert!(!single_sign_in_path(dir.path()).exists(), "the single sign-in became the list");
        assert_eq!(load_config(dir.path()).expect("loads").device, DeviceId(device));
    }

    fn sign_in(url: &str, token: &str) -> RemoteConfig {
        RemoteConfig { url: url.into(), token: token.into(), device: DeviceId(1), name: "laptop".into(), user: "ada".into() }
    }

    #[test]
    fn each_server_keeps_one_sign_in_and_the_first_stays_the_default() {
        let dir = tempfile::tempdir().expect("temp dir");
        keep_sign_in(dir.path(), &sign_in("https://work.example", "a")).expect("saves");
        keep_sign_in(dir.path(), &sign_in("https://oss.example", "b")).expect("saves");
        keep_sign_in(dir.path(), &sign_in("https://work.example", "c")).expect("saves");
        let tokens: Vec<String> = load_sign_ins(dir.path()).into_iter().map(|kept| kept.token).collect();
        assert_eq!(tokens, ["c", "b"]);
        assert_eq!(forget_sign_in(dir.path(), "https://work.example/").expect("saves").map(|gone| gone.token), Some("c".into()));
        assert_eq!(load_config(dir.path()).map(|kept| kept.url), Some("https://oss.example".into()));
        assert_eq!(forget_sign_in(dir.path(), "https://nowhere.example").expect("saves"), None);
    }
}
