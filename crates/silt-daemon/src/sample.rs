//! A page's `sample` capability on the daemon (ADR 0072): one question to
//! this machine's agent, with no tools and no session, the answer told to
//! the page's browser over the bus as it streams.

use std::time::{Duration, Instant};

use serde::Serialize;

use std::collections::HashMap;

use silt_types::ThreadId;
use tokio::sync::oneshot;

use crate::core::{Core, CoreError};
use crate::protocol::{BusEvent, SampleToolResult};

/// Questions each artifact may ask a minute.
const PER_MINUTE: usize = 4;
/// The most page questions answered at once, by every page together: each
/// is a Claude Code process.
const AT_ONCE: usize = 3;

/// One of the questions answered now, counted until it's done.
struct Answering(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl Drop for Answering {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Sampled {
    pub(crate) text: String,
    pub(crate) truncated: bool,
    /// The tier or model print mode was given, when the page asked for one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) model_applied: Option<String>,
}

/// Where `silt sample-tools` reads the tools of the question named `ask`.
fn tools_file(data_dir: &std::path::Path, ask: &str) -> std::path::PathBuf {
    data_dir.join("sample").join("tools").join(format!("{ask}.json"))
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
        asked: Question,
    ) -> Result<impl Future<Output = Result<Sampled, CoreError>> + Send + 'static, CoreError> {
        let Question { input, images, tools, model } = asked;
        let model = model.resolved().map_err(CoreError::Invalid)?;
        let question = silt_proto::artifact::sample_question(&input).map_err(CoreError::Invalid)?;
        silt_proto::artifact::sample_images(&images).map_err(CoreError::Invalid)?;
        silt_proto::artifact::sample_tools(&tools).map_err(CoreError::Invalid)?;
        let thread_id = self.find_thread(thread)?;
        if let Some(artifact) = artifact {
            let declared = self
                .history_entries(thread_id)
                .iter()
                .filter_map(|(_, entry)| entry.artifact.as_ref())
                .filter(|view| view.id == artifact)
                .max_by_key(|view| view.version)
                .and_then(|view| view.capabilities.clone());
            let Some(declared) = declared.filter(|declared| declared.contains_key("sample")) else {
                return Err(CoreError::Invalid(format!("artifact {artifact} doesn't declare sample")));
            };
            if !images.is_empty() && !silt_proto::artifact::sample_takes_images(&declared) {
                return Err(CoreError::Invalid(format!("artifact {artifact} doesn't declare sample with images")));
            }
        }
        let key = format!("{thread_id}/{}", artifact.unwrap_or("elsewhere"));
        let now = Instant::now();
        let recent = self.samples.entry(key).or_default();
        recent.retain(|at| now.duration_since(*at) < Duration::from_secs(60));
        if recent.len() >= PER_MINUTE {
            return Err(CoreError::Invalid(format!("rate limited: a page asks at most {PER_MINUTE} questions a minute")));
        }
        if self.sampling.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= AT_ONCE {
            self.sampling.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            return Err(CoreError::Invalid(format!("rate limited: this computer answers {AT_ONCE} pages' questions at once")));
        }
        let answering = Answering(self.sampling.clone());
        recent.push(now);
        let ask = ask.filter(|ask| (1..=64).contains(&ask.len()) && ask.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
        let cwd = self.data_dir.join("sample");
        let offered = match (tools.is_empty(), &ask, artifact) {
            (true, ..) => None,
            // From elsewhere, the page answers through the server, which checked it.
            (false, Some(ask), artifact) => Some(self.offer_tools(thread_id, artifact.unwrap_or_default(), ask, &tools)?),
            (false, None, _) => return Err(CoreError::Invalid("a page's tools run only for a named question".into())),
        };
        let tools_file = offered.as_ref().and(ask.as_ref()).map(|ask| tools_file(&self.data_dir, ask));
        // A named question can be stopped while it runs; those that ended are forgotten.
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.sample_stops.retain(|_, running| std::sync::Arc::strong_count(running) > 1);
        if let Some(ask) = &ask {
            self.sample_stops.insert(format!("{thread_id}/{}/{ask}", artifact.unwrap_or_default()), stop.clone());
        }
        let bus = self.bus.clone();
        let applied = model.clone();
        Ok(async move {
            std::fs::create_dir_all(&cwd)?;
            let answer = tokio::task::spawn_blocking(move || {
                let shown: Vec<_> = images
                    .iter()
                    .map(|image| silt_agents::oneshot::Image { media_type: &image.media_type, data: &image.data })
                    .collect();
                silt_agents::oneshot::ask_claude(
                    &question,
                    &shown,
                    offered.as_ref(),
                    model.as_deref(),
                    &cwd,
                    Some(stop),
                    |text| {
                        if let Some(ask) = &ask {
                            let _ = bus.send(BusEvent::Sample { ask: ask.clone(), text: text.to_owned() });
                        }
                    },
                )
            })
            .await;
            drop(answering);
            if let Some(file) = tools_file {
                let _ = std::fs::remove_file(file);
            }
            let answer = answer
                .map_err(|error| CoreError::Invalid(error.to_string()))?
                .map_err(|error| CoreError::Invalid(error.to_string()))?;
            Ok(Sampled { text: answer.text, truncated: answer.truncated, model_applied: applied })
        })
    }

    /// Readies `tools` for the question named `ask`: the file `silt
    /// sample-tools` serves them from, and what print mode needs to start it.
    fn offer_tools(
        &mut self,
        thread: ThreadId,
        artifact: &str,
        ask: &str,
        tools: &[silt_proto::artifact::SampleTool],
    ) -> Result<silt_agents::oneshot::OfferedTools, CoreError> {
        let now = Instant::now();
        self.sample_calls.retain(|_, waiting| now.duration_since(waiting.since) < ASKS_KEPT);
        let file = tools_file(&self.data_dir, ask);
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&file, serde_json::json!({ "ask": ask, "tools": tools }).to_string())?;
        let silt = std::env::current_exe()?;
        let config = serde_json::json!({"mcpServers": {"page": {
            "command": silt,
            "args": ["sample-tools", file],
            "env": {"SILT_HOME": self.data_dir},
        }}});
        self.sample_calls.insert(
            ask.to_owned(),
            WaitingTools { thread, artifact: artifact.to_owned(), since: now, next: 0, calls: HashMap::new() },
        );
        Ok(silt_agents::oneshot::OfferedTools {
            mcp_config: config.to_string(),
            allowed: tools.iter().map(|tool| format!("mcp__page__{}", tool.name)).collect(),
        })
    }

    /// The agent calls one of a page's tools: the page hears of it over the
    /// bus, and the call is answered once it answers, within a minute.
    pub(crate) fn sample_tool_call(
        &mut self,
        ask: &str,
        name: String,
        input: serde_json::Value,
    ) -> Result<impl Future<Output = Result<serde_json::Value, CoreError>> + Send + 'static, CoreError> {
        let waiting = self
            .sample_calls
            .get_mut(ask)
            .ok_or_else(|| CoreError::Invalid("no page's question by that name is waiting".into()))?;
        if waiting.calls.len() >= MAX_CALLS_WAITING {
            return Err(CoreError::Invalid("too many of the page's tools are already running".into()));
        }
        waiting.next += 1;
        let call = waiting.next;
        let (answer, answered) = oneshot::channel();
        waiting.calls.insert(call, answer);
        let _ = self.bus.send(BusEvent::SampleTool { ask: ask.to_owned(), call, name, input });
        Ok(async move {
            match tokio::time::timeout(TOOL_WAIT, answered).await {
                Ok(Ok(Ok(value))) => Ok(value),
                Ok(Ok(Err(error))) => Err(CoreError::Invalid(error)),
                Ok(Err(_)) => Err(CoreError::Invalid("the page went away".into())),
                Err(_) => Err(CoreError::Invalid(format!("the page didn't answer within {} seconds", TOOL_WAIT.as_secs()))),
            }
        })
    }

    /// Stops a page's question named `ask` while its agent runs, ending the
    /// run with what it started; a question that ended already is nothing.
    pub(crate) fn stop_sample(&mut self, thread: &str, artifact: &str, ask: &str) -> Result<(), CoreError> {
        let thread = self.find_thread(thread)?;
        if let Some(stop) = self.sample_stops.remove(&format!("{thread}/{artifact}/{ask}")) {
            stop.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(())
    }

    /// A page's answer to a call of one of its tools, for the question it
    /// asked about its own artifact.
    pub(crate) fn sample_tool_result(
        &mut self,
        thread: &str,
        artifact: &str,
        ask: &str,
        call: u32,
        result: SampleToolResult,
    ) -> Result<(), CoreError> {
        let thread = self.find_thread(thread)?;
        let waiting = self
            .sample_calls
            .get_mut(ask)
            .filter(|waiting| waiting.thread == thread && waiting.artifact == artifact)
            .ok_or_else(|| CoreError::Invalid("no question of this page's by that name is waiting".into()))?;
        let answer = waiting.calls.remove(&call).ok_or_else(|| CoreError::Invalid("no such call is waiting".into()))?;
        let _ = answer.send(if result.ok { Ok(result.value) } else { Err(result.error) });
        Ok(())
    }
}

/// What a page asks: its words, the images it shows, the tools it offers,
/// and the model it wants.
pub(crate) struct Question {
    pub(crate) input: serde_json::Value,
    pub(crate) images: Vec<silt_proto::artifact::SampleImage>,
    pub(crate) tools: Vec<silt_proto::artifact::SampleTool>,
    pub(crate) model: silt_proto::artifact::SampleModel,
}

/// How long a page's tool may take to answer a call.
const TOOL_WAIT: Duration = Duration::from_secs(60);
/// How long a question's tools are kept, past its longest run.
const ASKS_KEPT: Duration = Duration::from_secs(10 * 60);
const MAX_CALLS_WAITING: usize = 8;

/// A question whose page offered tools: the calls of them waiting for the
/// page to answer.
pub(crate) struct WaitingTools {
    thread: ThreadId,
    artifact: String,
    since: Instant,
    next: u32,
    calls: HashMap<u32, oneshot::Sender<Result<serde_json::Value, String>>>,
}
