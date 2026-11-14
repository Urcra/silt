//! The daemon's model gateway, as this agent reaches it: `SILT_MODEL_URL`
//! with its run's token, `SILT_MODEL_TOKEN`.

use serde_json::Value;
use tokio::sync::watch;

use crate::stream::{Events, Piece, Usage, piece};

/// A model the gateway serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub id: String,
    pub name: String,
    pub context: u64,
}

#[derive(Debug, Clone)]
pub struct Gateway {
    url: String,
    token: String,
    client: reqwest::Client,
}

/// How a streamed turn ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    Done { text: String, usage: Option<Usage> },
    Cancelled { text: String },
}

impl Gateway {
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("SILT_MODEL_URL").ok().filter(|url| !url.is_empty())?;
        let token = std::env::var("SILT_MODEL_TOKEN").unwrap_or_default();
        Some(Self { url: url.trim_end_matches('/').to_owned(), token, client: reqwest::Client::new() })
    }

    pub async fn models(&self) -> Result<Vec<Listed>, String> {
        let response = self
            .client
            .get(format!("{}/v1/models", self.url))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|error| error.to_string())?;
        let listed: Value = response.json().await.map_err(|error| error.to_string())?;
        Ok(listed["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|model| {
                Some(Listed {
                    id: model["id"].as_str()?.to_owned(),
                    name: model["name"].as_str().unwrap_or_default().to_owned(),
                    context: model["context"].as_u64().unwrap_or(0),
                })
            })
            .collect())
    }

    async fn post(&self, body: &Value) -> Result<reqwest::Response, String> {
        let response = self
            .client
            .post(format!("{}/v1/chat/completions", self.url))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .map_err(|error| format!("the model gateway didn't answer: {error}"))?;
        if response.status().is_success() {
            return Ok(response);
        }
        let status = response.status();
        let answer: Value = response.json().await.unwrap_or_default();
        let message = answer["error"]["message"].as_str().map_or_else(|| format!("the model answered {status}"), str::to_owned);
        Err(message)
    }

    /// Streams a turn, handing each piece on, until it ends or `cancel`
    /// says to stop.
    pub async fn stream(
        &self,
        body: &Value,
        mut cancel: watch::Receiver<bool>,
        mut on: impl FnMut(&Piece),
    ) -> Result<Ended, String> {
        let mut response = self.post(body).await?;
        let mut events = Events::default();
        let mut text = String::new();
        let mut usage = None;
        loop {
            let chunk = tokio::select! {
                chunk = response.chunk() => chunk.map_err(|error| error.to_string())?,
                _ = cancel.wait_for(|cancelled| *cancelled) => return Ok(Ended::Cancelled { text }),
            };
            let Some(chunk) = chunk else {
                break;
            };
            for payload in events.feed(&chunk) {
                let piece = piece(&payload);
                if piece.done {
                    return Ok(Ended::Done { text, usage });
                }
                if let Some(more) = &piece.text {
                    text.push_str(more);
                }
                usage = piece.usage.or(usage);
                on(&piece);
            }
        }
        Ok(Ended::Done { text, usage })
    }

    /// A whole answer at once, for a summary.
    pub async fn complete(&self, body: &Value) -> Result<String, String> {
        let answer: Value = self.post(body).await?.json().await.map_err(|error| error.to_string())?;
        answer["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| "the model gave no answer".to_owned())
    }
}
