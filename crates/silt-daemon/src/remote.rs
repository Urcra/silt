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

/// The server this daemon syncs with, saved by `silt auth login`.
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

pub fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join("server.json")
}

pub fn load_config(data_dir: &Path) -> Option<RemoteConfig> {
    let text = fs::read_to_string(config_path(data_dir)).ok()?;
    serde_json::from_str(&text).ok().or_else(|| serde_json::from_str::<LegacyConfig>(&text).ok().map(RemoteConfig::from))
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

/// Saves the settings, with the device's token, for this user's eyes only.
pub fn save_config(data_dir: &Path, config: &RemoteConfig) -> Result<(), RemoteError> {
    use std::io::Write as _;
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
    crate::paths::make_private(data_dir)?;
    let path = config_path(data_dir);
    let mut file = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    file.write_all(&serde_json::to_vec_pretty(config)?)?;
    Ok(())
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
        let body = RegisterThread { thread: thread.clone(), visibility, origin_url };
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
        fs::write(config_path(dir.path()), legacy).expect("writes");
        assert_eq!(load_config(dir.path()).expect("loads").device, DeviceId(device));
        save_config(dir.path(), &load_config(dir.path()).expect("loads")).expect("saves");
        assert!(fs::read_to_string(config_path(dir.path())).expect("reads").contains(&format!("\"{device:032x}\"")));
        assert_eq!(load_config(dir.path()).expect("loads").device, DeviceId(device));
    }
}
