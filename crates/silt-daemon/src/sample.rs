//! A page's `sample` capability on the daemon (ADR 0072): one question to
//! this machine's agent, with no tools and no session, the answer told to
//! the page's browser over the bus as it streams.

use std::time::{Duration, Instant};

use serde::Serialize;

use crate::core::{Core, CoreError};
use crate::protocol::BusEvent;

/// Questions each artifact may ask a minute.
const PER_MINUTE: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Sampled {
    pub(crate) text: String,
    pub(crate) truncated: bool,
}

impl Core {
    /// Asks this machine's Claude Code a page's question: for `artifact`
    /// of `thread` when a page here asks, checked against what its latest
    /// version declares, or for the person elsewhere, whose server checked
    /// it. Chunks go out as `sample` bus events named `ask`.
    pub(crate) fn sample(
        &mut self,
        thread: &str,
        artifact: Option<&str>,
        ask: Option<String>,
        input: &serde_json::Value,
    ) -> Result<impl Future<Output = Result<Sampled, CoreError>> + Send + 'static, CoreError> {
        let question = silt_proto::artifact::sample_question(input).map_err(CoreError::Invalid)?;
        let thread_id = self.find_thread(thread)?;
        if let Some(artifact) = artifact {
            let declared = self
                .history_entries(thread_id)
                .iter()
                .filter_map(|(_, entry)| entry.artifact.as_ref())
                .filter(|view| view.id == artifact)
                .max_by_key(|view| view.version)
                .and_then(|view| view.capabilities.as_ref())
                .is_some_and(|declared| declared.contains_key("sample"));
            if !declared {
                return Err(CoreError::Invalid(format!("artifact {artifact} doesn't declare sample")));
            }
        }
        let key = format!("{thread_id}/{}", artifact.unwrap_or("elsewhere"));
        let now = Instant::now();
        let recent = self.samples.entry(key).or_default();
        recent.retain(|at| now.duration_since(*at) < Duration::from_secs(60));
        if recent.len() >= PER_MINUTE {
            return Err(CoreError::Invalid(format!("rate limited: a page asks at most {PER_MINUTE} questions a minute")));
        }
        recent.push(now);
        let ask = ask.filter(|ask| (1..=64).contains(&ask.len()) && ask.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
        let cwd = self.data_dir.join("sample");
        let bus = self.bus.clone();
        Ok(async move {
            std::fs::create_dir_all(&cwd)?;
            let answer = tokio::task::spawn_blocking(move || {
                silt_agents::oneshot::ask_claude(&question, &cwd, |text| {
                    if let Some(ask) = &ask {
                        let _ = bus.send(BusEvent::Sample { ask: ask.clone(), text: text.to_owned() });
                    }
                })
            })
            .await
            .map_err(|error| CoreError::Invalid(error.to_string()))?
            .map_err(|error| CoreError::Invalid(error.to_string()))?;
            Ok(Sampled { text: answer.text, truncated: answer.truncated })
        })
    }
}
