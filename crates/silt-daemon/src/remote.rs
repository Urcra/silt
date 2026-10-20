use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use silt_log::Delta;
use silt_proto::{
    ErrorReply, GENERATION_HEADER, Lease, Listing, LoginReply, LoginRequest, PROTOCOL, PROTOCOL_HEADER,
    InFlightQuery, InFlightReply, PullReply, PushRequest, SearchQuery, SearchReply, ThreadInfo, Visibility,
    VisibilityRequest,
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
    #[error("the server refused: {0:?}")]
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
    serde_json::from_str(&fs::read_to_string(config_path(data_dir)).ok()?).ok()
}

pub fn save_config(data_dir: &Path, config: &RemoteConfig) -> Result<(), RemoteError> {
    fs::create_dir_all(data_dir)?;
    fs::write(config_path(data_dir), serde_json::to_vec_pretty(config)?)?;
    Ok(())
}

/// Exchanges a pairing code for a device token.
pub async fn login(url: &str, code: &str, name: &str) -> Result<RemoteConfig, RemoteError> {
    let url = url.trim_end_matches('/').to_owned();
    let response = reqwest::Client::new()
        .post(format!("{url}/v1/login"))
        .header(PROTOCOL_HEADER, PROTOCOL.to_string())
        .json(&LoginRequest {
            code: code.to_owned(),
            name: name.to_owned(),
        })
        .send()
        .await?;
    let reply: LoginReply = checked(response).await?.json().await?;
    Ok(RemoteConfig {
        url,
        token: reply.token,
        device: reply.device,
        name: name.to_owned(),
        user: reply.user,
    })
}

#[derive(Debug, Clone)]
pub struct Remote {
    pub config: RemoteConfig,
    http: reqwest::Client,
}

impl Remote {
    pub fn new(config: RemoteConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
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
            vec![
                format!("Authorization: Bearer {}", self.config.token),
                format!("{PROTOCOL_HEADER}: {PROTOCOL}"),
            ],
        )
    }

    pub async fn listing(&self) -> Result<Listing, RemoteError> {
        let response = self.request(reqwest::Method::GET, "/v1/listing").send().await?;
        Ok(checked(response).await?.json().await?)
    }

    pub async fn register_thread(&self, thread: &ThreadInfo) -> Result<(), RemoteError> {
        let response = self.request(reqwest::Method::POST, "/v1/threads").json(thread).send().await?;
        checked(response).await?;
        Ok(())
    }

    pub async fn take_lease(&self, thread: ThreadId) -> Result<Lease, RemoteError> {
        let response = self
            .request(reqwest::Method::POST, &format!("/v1/threads/{thread}/lease"))
            .send()
            .await?;
        Ok(checked(response).await?.json().await?)
    }

    pub async fn set_visibility(
        &self,
        repo: Option<String>,
        thread: Option<ThreadId>,
        visibility: Visibility,
    ) -> Result<(), RemoteError> {
        let response = self
            .request(reqwest::Method::PUT, "/v1/visibility")
            .json(&VisibilityRequest {
                repo,
                thread,
                visibility,
            })
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
        let response = self
            .request(reqwest::Method::PUT, &format!("/v1/blobs/{hash}"))
            .body(bytes)
            .send()
            .await?;
        checked(response).await?;
        Ok(())
    }

    pub async fn get_blob(&self, hash: BlobHash) -> Result<Vec<u8>, RemoteError> {
        let response = self.request(reqwest::Method::GET, &format!("/v1/blobs/{hash}")).send().await?;
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
    response
        .headers()
        .get(GENERATION_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
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
    let reply: ErrorReply = response.json().await.unwrap_or(ErrorReply::Invalid {
        message: "unreadable error".into(),
    });
    Err(match reply {
        ErrorReply::ProtocolMismatch { supported } => RemoteError::ProtocolMismatch { supported },
        other => RemoteError::Refused(other),
    })
}
