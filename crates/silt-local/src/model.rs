//! The daemon's model gateway, as this agent reaches it: `SILT_MODEL_URL`
//! with its run's token, `SILT_MODEL_TOKEN`.

use serde_json::Value;
use tokio::sync::watch;

use crate::conversation::Call;
use crate::stream::{CallPiece, Events, Piece, Usage, piece};

/// Ends once the turn is cancelled; a channel whose sender is gone never
/// cancels.
pub async fn cancelled(mut cancel: watch::Receiver<bool>) {
    if cancel.wait_for(|cancelled| *cancelled).await.is_err() {
        std::future::pending::<()>().await;
    }
}

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
    Done { text: String, calls: Vec<Call>, usage: Option<Usage> },
    Cancelled { text: String },
}

/// Puts a call's pieces together as they come.
fn gather(calls: &mut Vec<Call>, piece: &CallPiece) {
    while calls.len() <= piece.index {
        calls.push(Call { id: String::new(), name: String::new(), arguments: String::new() });
    }
    let call = &mut calls[piece.index];
    if let Some(id) = &piece.id {
        call.id.clone_from(id);
    }
    if let Some(name) = &piece.name {
        call.name.push_str(name);
    }
    if let Some(arguments) = &piece.arguments {
        call.arguments.push_str(arguments);
    }
}

/// Calls whole enough to make: named, each with an id of its own.
fn finished(calls: Vec<Call>) -> Vec<Call> {
    calls
        .into_iter()
        .enumerate()
        .filter(|(_, call)| !call.name.is_empty())
        .map(|(index, mut call)| {
            if call.id.is_empty() {
                call.id = format!("call-{index}");
            }
            call
        })
        .collect()
}

impl Gateway {
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("SILT_MODEL_URL").ok().filter(|url| !url.is_empty())?;
        let token = std::env::var("SILT_MODEL_TOKEN").unwrap_or_default();
        // The gateway is on loopback; a proxy the environment names must never see the conversation.
        let client = reqwest::Client::builder().no_proxy().build().ok()?;
        Some(Self { url: url.trim_end_matches('/').to_owned(), token, client })
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
    pub async fn stream(&self, body: &Value, cancel: watch::Receiver<bool>, mut on: impl FnMut(&Piece)) -> Result<Ended, String> {
        let mut response = self.post(body).await?;
        let mut events = Events::default();
        let mut text = String::new();
        let mut calls = Vec::new();
        let mut usage = None;
        loop {
            let chunk = tokio::select! {
                chunk = response.chunk() => chunk.map_err(|error| error.to_string())?,
                () = cancelled(cancel.clone()) => return Ok(Ended::Cancelled { text }),
            };
            let Some(chunk) = chunk else {
                break;
            };
            for payload in events.feed(&chunk) {
                let piece = piece(&payload);
                if piece.done {
                    return Ok(Ended::Done { text, calls: finished(calls), usage });
                }
                if let Some(more) = &piece.text {
                    text.push_str(more);
                }
                for call in &piece.calls {
                    gather(&mut calls, call);
                }
                usage = piece.usage.or(usage);
                on(&piece);
            }
        }
        Ok(Ended::Done { text, calls: finished(calls), usage })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_are_put_together_from_their_pieces() {
        let mut calls = Vec::new();
        let piece = |index, id: Option<&str>, name: Option<&str>, arguments: &str| CallPiece {
            index,
            id: id.map(str::to_owned),
            name: name.map(str::to_owned),
            arguments: Some(arguments.to_owned()),
        };
        gather(&mut calls, &piece(0, Some("a"), Some("read_file"), r#"{"path":"#));
        gather(&mut calls, &piece(1, None, Some("list"), "{}"));
        gather(&mut calls, &piece(0, None, None, r#""x"}"#));
        let calls = finished(calls);
        assert_eq!(calls[0], Call { id: "a".into(), name: "read_file".into(), arguments: r#"{"path":"x"}"#.into() });
        assert_eq!(calls[1].id, "call-1", "a call the model gave no id gets one");
    }
}
