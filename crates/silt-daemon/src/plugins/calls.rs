//! A run's plugin calls (ADR 0093), kept in the store: those that ran,
//! those refused, and those that wait for the person as approvals. An
//! approval is answered by the hash of the request it shows, and only that
//! request runs, once; a grant the person gives with it lets the same kind
//! of call run without asking, for the run or the thread, until a time.
//! A call the daemon stopped in the middle of reads as outcome unknown.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use silt_access::Canonical;
use silt_store::{GrantRecord, PluginCallRecord, Store};
use silt_types::ThreadId;
use tokio::sync::{broadcast, oneshot, watch};

use super::installed::{self, Installed};
use super::wasm::{CallParts, WasmRequest, asks};
use super::{Outcome, exec_of, execute, load, prepare, shown_input, tools};
use crate::CoreError;
use crate::core::{Core, now_ms, to_response};
use crate::protocol::{BusEvent, Response};

/// How long a call waits for the person before the agent is told to wait
/// on, and how long an approval stays open.
const WAIT: Duration = Duration::from_secs(45);
const OPEN_MS: i64 = 15 * 60 * 1000;
/// The calls a thread's access lists.
const LISTED: i64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    Pending,
    Approved,
    Rejected,
    Expired,
}

/// An approval as the person sees it: Silt's own copy of the request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ApprovalView {
    pub id: u64,
    pub thread: String,
    pub plugin: String,
    pub tool: String,
    pub environment: String,
    pub target: String,
    /// What runs, with what hides meaning made visible.
    pub shown: String,
    /// The hash an answer must name, so it answers what was shown.
    pub hash: String,
    pub created_ms: i64,
    pub state: ApprovalState,
    pub outcome: Option<Outcome>,
    /// It may change something, rather than only read.
    pub change: bool,
    /// Its environment allows no grant: each is approved on its own.
    pub once: bool,
    /// A WebAssembly plugin's card, which Silt draws (ADR 0100).
    pub card: Option<silt_plugin::Card>,
    /// What it will send, each checked while it runs unless opaque.
    pub effects: Vec<silt_plugin::Effect>,
    /// What decided it, such as a rule or an untrusted publisher.
    pub why: Option<String>,
}

/// What an approval may also allow from then on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case", tag = "for")]
pub enum GrantScope {
    /// The rest of this run.
    Run,
    /// The thread, until `minutes` from now.
    Thread { minutes: u32 },
    /// The workspace from now on: a folder asked for becomes one of its own
    /// (ADR 0098). Only for calls that hand over folders.
    Workspace,
}

/// A call as the thread's access lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct CallView {
    pub id: u64,
    pub plugin: String,
    pub tool: String,
    pub environment: String,
    pub target: String,
    pub shown: String,
    pub change: bool,
    /// It waited for the person.
    pub asked: bool,
    /// It ran without asking under this grant.
    pub grant: Option<u64>,
    /// `pending`, `running`, `rejected`, `expired` or `ended`.
    pub state: String,
    pub outcome: Option<Outcome>,
    pub created_ms: i64,
    pub answered_ms: Option<i64>,
    pub card: Option<silt_plugin::Card>,
    pub effects: Vec<silt_plugin::Effect>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct GrantView {
    pub id: u64,
    pub plugin: String,
    pub environment: String,
    pub change: bool,
    /// For the run that asked only, rather than the thread.
    pub run_only: bool,
    pub created_ms: i64,
    pub until_ms: Option<i64>,
    pub revoked_ms: Option<i64>,
    /// In force now.
    pub active: bool,
}

/// What reached a thread's sandbox through Silt, and what was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AccessView {
    pub calls: Vec<CallView>,
    pub grants: Vec<GrantView>,
    /// What the gateway refused, newest first.
    pub refused: Vec<NetworkRefusal>,
    /// The devices Silt holds for the thread's sandboxes.
    pub devices: Vec<RelayView>,
    /// The folders granted to its agents (ADR 0098).
    pub folders: Vec<FolderView>,
}

/// A folder granted to a thread's agents, shown from its next restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct FolderView {
    pub path: String,
    pub writable: bool,
    /// For the run that asked, rather than the thread on this machine.
    pub run_only: bool,
    pub granted_ms: i64,
    pub until_ms: Option<i64>,
}

/// A device relayed to the thread's sandboxes, its session kept across
/// the agent's restarts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct RelayView {
    /// `/dev/ttyUSB0`, or `adb:SERIAL`.
    pub device: String,
    /// `serial` or `adb`.
    pub kind: String,
    pub since_ms: i64,
}

/// A connection the gateway refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NetworkRefusal {
    pub time_ms: i64,
    pub host: String,
    pub port: u16,
    pub why: String,
}

/// Who waits on each call still open, and the tools each MCP plugin's
/// server offers, as it last listed them.
#[derive(Default)]
pub struct Approvals {
    waiting: HashMap<i64, Arc<watch::Sender<Option<Outcome>>>>,
    listed: Arc<std::sync::Mutex<BTreeMap<String, Vec<Value>>>>,
    /// WebAssembly plugins (ADR 0100).
    pub(crate) wasm: Arc<super::wasm::Wasm>,
}

impl Approvals {
    fn channel(&mut self, id: i64) -> Arc<watch::Sender<Option<Outcome>>> {
        self.waiting.entry(id).or_insert_with(|| Arc::new(watch::channel(None).0)).clone()
    }
}

fn state_of(call: &PluginCallRecord, now: i64) -> ApprovalState {
    match call.state.as_str() {
        "pending" if now - call.created_ms > OPEN_MS => ApprovalState::Expired,
        "pending" => ApprovalState::Pending,
        "rejected" => ApprovalState::Rejected,
        "expired" => ApprovalState::Expired,
        _ => ApprovalState::Approved,
    }
}

fn canonical_of(call: &PluginCallRecord) -> Canonical {
    serde_json::from_str(&call.request).unwrap_or_else(|_| Canonical {
        plugin: String::new(),
        tool: String::new(),
        environment: String::new(),
        target: String::new(),
        input: Value::Null,
    })
}

/// A call as the person sees it, whichever kind of plugin made it.
struct Described {
    plugin: String,
    tool: String,
    environment: String,
    target: String,
    shown: String,
    card: Option<silt_plugin::Card>,
    effects: Vec<silt_plugin::Effect>,
    once: Option<bool>,
    why: Option<String>,
}

fn described(call: &PluginCallRecord) -> Described {
    if call.kind == "wasm" {
        if let Ok(request) = serde_json::from_str::<WasmRequest>(&call.request) {
            let prepared = request.prepared;
            return Described {
                plugin: prepared.plugin,
                tool: prepared.tool,
                environment: prepared.environment.unwrap_or_default(),
                target: prepared.card.title.clone(),
                shown: super::wasm::card_text(&prepared.card),
                card: Some(prepared.card.shown()),
                effects: prepared.effects,
                once: Some(request.decision.once),
                why: Some(request.decision.why),
            };
        }
        // Refused before it was prepared: the tool and input it was given.
        let asked: Value = serde_json::from_str(&call.request).unwrap_or(Value::Null);
        return Described {
            plugin: asked.get("plugin").and_then(Value::as_str).unwrap_or_default().to_owned(),
            tool: asked.get("tool").and_then(Value::as_str).unwrap_or_default().to_owned(),
            environment: String::new(),
            target: String::new(),
            shown: shown_input(asked.get("input").unwrap_or(&Value::Null)),
            card: None,
            effects: Vec::new(),
            once: Some(true),
            why: None,
        };
    }
    let canonical = canonical_of(call);
    Described {
        plugin: canonical.plugin,
        tool: canonical.tool,
        environment: canonical.environment,
        target: canonical.target,
        shown: shown_input(&canonical.input),
        card: None,
        effects: Vec::new(),
        once: None,
        why: None,
    }
}

fn outcome_of(call: &PluginCallRecord) -> Option<Outcome> {
    call.outcome.as_deref().and_then(|outcome| serde_json::from_str(outcome).ok())
}

fn id_of(id: i64) -> u64 {
    u64::try_from(id).unwrap_or_default()
}

/// Ends call `id` with `outcome`: kept, told to whoever waits, announced.
async fn end(
    store: &Store,
    bus: &broadcast::Sender<BusEvent>,
    waiting: &watch::Sender<Option<Outcome>>,
    id: i64,
    outcome: Outcome,
) {
    let json = serde_json::to_string(&outcome).unwrap_or_default();
    if let Err(error) = store.set_plugin_call(id, "ended", Some(&json), None).await {
        tracing::warn!(%error, id, "a plugin call's outcome wasn't kept");
    }
    waiting.send_replace(Some(outcome));
    let _ = bus.send(BusEvent::Approvals);
}

impl Core {
    /// The thread and workspace a run's token was made for, and the token's
    /// hash.
    async fn run_of(&self, token: &str) -> Result<(ThreadId, String, Vec<u8>), CoreError> {
        let hash = blake3::hash(token.as_bytes()).as_bytes().to_vec();
        let silt_store::RunToken { thread, workspace, .. } =
            self.store.run_token(&hash).await?.ok_or_else(|| CoreError::Invalid("this run's token isn't known here".into()))?;
        // Its calls and grants are kept with its thread, so a run that has
        // none can't make any.
        if thread == ThreadId(0) {
            return Err(CoreError::Invalid(
                "this run started before its workspace had plugins; start the agent again to use them".into(),
            ));
        }
        Ok((thread, workspace, hash))
    }

    /// The tools of the plugins on in the run's workspace, an MCP plugin's
    /// as its server lists them now, which it does outside the core.
    pub(crate) async fn plugin_tools(&self, token: &str, reply: oneshot::Sender<Response>) {
        let workspace = match self.run_of(token).await {
            Ok((_, workspace, _)) => workspace,
            Err(error) => {
                let _ = reply.send(to_response::<()>(Err(error)));
                return;
            }
        };
        let (data_dir, listed, wasm) = (self.data_dir.clone(), self.approvals.listed.clone(), self.approvals.wasm.clone());
        tokio::spawn(async move {
            let plugins = load(&data_dir);
            for plugin in plugins.iter().filter(|plugin| plugin.workspaces.contains(&workspace)) {
                if let Some(offered) = super::list_mcp(&data_dir, plugin, &workspace).await {
                    listed.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(plugin.name.clone(), offered);
                }
            }
            let known = listed.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
            let mut offered = tools(&plugins, &workspace, &known);
            let wasm_offered = tokio::task::spawn_blocking(move || wasm.tools(&data_dir, &workspace)).await.unwrap_or_default();
            offered.extend(wasm_offered);
            let _ = reply.send(Response::ok(offered));
        });
    }

    /// Calls in the daemon's hands when it stopped can't be followed: they
    /// read as outcome unknown, and are never sent again.
    pub(crate) async fn recover_plugin_calls(&mut self) -> Result<(), CoreError> {
        for call in self.store.plugin_calls(None, Some("running"), i64::MAX).await? {
            let outcome = Outcome::Unknown { why: "Silt stopped while it ran".into() };
            let json = serde_json::to_string(&outcome).unwrap_or_default();
            self.store.set_plugin_call(call.id, "ended", Some(&json), None).await?;
        }
        Ok(())
    }

    /// Runs a plugin's tool for a run, or makes it an approval to wait on.
    pub(crate) async fn plugin_call(&mut self, token: &str, tool: &str, input: Value, reply: oneshot::Sender<Response>) {
        let answer = self.start_call(token, tool, input).await;
        match answer {
            Ok((id, channel)) => wait_on(channel, id, reply),
            Err(error) => {
                let _ = reply.send(to_response::<()>(Err(error)));
            }
        }
    }

    async fn start_call(
        &mut self,
        token: &str,
        tool: &str,
        input: Value,
    ) -> Result<(i64, Arc<watch::Sender<Option<Outcome>>>), CoreError> {
        let (thread, workspace, run_hash) = self.run_of(token).await?;
        let found = {
            let (wasm, data_dir, workspace, tool) =
                (self.approvals.wasm.clone(), self.data_dir.clone(), workspace.clone(), tool.to_owned());
            tokio::task::spawn_blocking(move || wasm.find(&data_dir, &workspace, &tool)).await.ok().flatten()
        };
        if let Some((installed, plugin_tool)) = found {
            return self.start_wasm_call(thread, workspace, run_hash, installed, plugin_tool.name, input).await;
        }
        let plugins = load(&self.data_dir);
        let now = now_ms();
        let mut call = PluginCallRecord {
            id: 0,
            thread,
            run_hash: run_hash.clone(),
            workspace: workspace.clone(),
            request: String::new(),
            hash: String::new(),
            change: false,
            asked: false,
            state: "running".into(),
            outcome: None,
            grant: None,
            created_ms: now,
            answered_ms: None,
            kind: "legacy".into(),
        };
        let data_dir = self.data_dir.clone();
        let schemas = move |plugin: &str, environment: &str| super::schema_of(&data_dir, plugin, environment);
        let listed = self.approvals.listed.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        let prepared = match prepare(&plugins, &schemas, &listed, &workspace, tool, &input) {
            Ok(prepared) => prepared,
            Err(why) => {
                // Kept too, so the person sees what was refused.
                let canonical = Canonical {
                    plugin: String::new(),
                    tool: tool.to_owned(),
                    environment: input.get("environment").and_then(Value::as_str).unwrap_or_default().to_owned(),
                    target: String::new(),
                    input,
                };
                call.request = serde_json::to_string(&canonical).unwrap_or_default();
                call.hash = canonical.hash();
                call.state = "ended".into();
                let outcome = Outcome::Refused { why };
                call.outcome = Some(serde_json::to_string(&outcome).unwrap_or_default());
                let id = self.store.add_plugin_call(&call).await?;
                self.announce(BusEvent::Approvals);
                let channel = Arc::new(watch::channel(Some(outcome)).0);
                return Ok((id, channel));
            }
        };
        call.request = serde_json::to_string(&prepared.canonical).unwrap_or_default();
        call.hash = prepared.canonical.hash();
        call.change = prepared.change;
        let mut ask = prepared.ask;
        if ask && !prepared.once {
            let grant = self
                .store
                .grant_for(
                    thread,
                    &run_hash,
                    &prepared.canonical.plugin,
                    &prepared.canonical.environment,
                    prepared.change,
                    None,
                    now,
                )
                .await?;
            if let Some(grant) = grant {
                ask = false;
                call.grant = Some(grant.id);
            }
        }
        call.asked = ask;
        if ask {
            call.state = "pending".into();
        }
        let id = self.store.add_plugin_call(&call).await?;
        let channel = self.approvals.channel(id);
        self.announce(BusEvent::Approvals);
        if !ask {
            self.run_call(id, &workspace, &prepared.canonical, channel.clone());
        }
        Ok((id, channel))
    }

    /// Runs call `id`'s request in the background, as it stands now.
    fn run_call(&self, id: i64, workspace: &str, canonical: &Canonical, channel: Arc<watch::Sender<Option<Outcome>>>) {
        let (store, bus) = (self.store.clone(), self.bus.clone());
        let ready = exec_of(&self.data_dir, &load(&self.data_dir), workspace, canonical);
        tokio::spawn(async move {
            let outcome = match ready {
                Ok((exec, data)) => execute(exec, data).await,
                Err(why) => Outcome::Refused { why },
            };
            end(&store, &bus, &channel, id, outcome).await;
        });
    }

    /// Waits on a call the run made earlier.
    pub(crate) async fn plugin_wait(&mut self, token: &str, id: u64, reply: oneshot::Sender<Response>) {
        let answer = async {
            let (thread, _, _) = self.run_of(token).await?;
            let call = self
                .store
                .plugin_call(i64::try_from(id).unwrap_or_default())
                .await?
                .filter(|call| call.thread == thread)
                .ok_or_else(|| CoreError::Invalid(format!("this thread has no plugin call {id}")))?;
            Ok::<_, CoreError>(call)
        }
        .await;
        let call = match answer {
            Ok(call) => call,
            Err(error) => {
                let _ = reply.send(to_response::<()>(Err(error)));
                return;
            }
        };
        let finished = match state_of(&call, now_ms()) {
            ApprovalState::Rejected => {
                Some(outcome_of(&call).unwrap_or(Outcome::Refused { why: "the person rejected it".into() }))
            }
            ApprovalState::Expired => {
                Some(Outcome::Refused { why: "its approval expired; ask again if it's still wanted".into() })
            }
            _ => outcome_of(&call),
        };
        let channel = match finished {
            Some(outcome) => Arc::new(watch::channel(Some(outcome)).0),
            None => self.approvals.channel(call.id),
        };
        wait_on(channel, call.id, reply);
    }

    /// The person's answer to approval `id`, naming the `hash` they saw,
    /// and what else it allows from then on.
    pub(crate) async fn approval_answer(
        &mut self,
        id: u64,
        approve: bool,
        hash: &str,
        grant: Option<GrantScope>,
    ) -> Result<(), CoreError> {
        let id = i64::try_from(id).map_err(|_| CoreError::Invalid(format!("no approval {id}")))?;
        let call = self
            .store
            .plugin_call(id)
            .await?
            .filter(|call| call.asked)
            .ok_or_else(|| CoreError::Invalid(format!("no approval {id}")))?;
        let now = now_ms();
        let channel = self.approvals.channel(id);
        match state_of(&call, now) {
            ApprovalState::Pending => {}
            ApprovalState::Expired if call.state == "pending" => {
                let outcome = Outcome::Refused { why: "its approval expired".into() };
                let json = serde_json::to_string(&outcome).unwrap_or_default();
                self.store.set_plugin_call(id, "expired", Some(&json), Some(now)).await?;
                channel.send_replace(Some(outcome));
                self.announce(BusEvent::Approvals);
                return Err(CoreError::Invalid(format!("approval {id} expired")));
            }
            _ => return Err(CoreError::Invalid(format!("approval {id} isn't waiting any more"))),
        }
        if call.hash != hash {
            return Err(CoreError::Invalid("that isn't the request this approval holds; look again".into()));
        }
        if !approve {
            let outcome = Outcome::Refused { why: "the person rejected it".into() };
            let json = serde_json::to_string(&outcome).unwrap_or_default();
            self.store.set_plugin_call(id, "rejected", Some(&json), Some(now)).await?;
            channel.send_replace(Some(outcome));
            self.announce(BusEvent::Approvals);
            return Ok(());
        }
        if call.kind == "wasm" {
            return self.approve_wasm_call(call, grant, channel, now).await;
        }
        let canonical = canonical_of(&call);
        if let Some(scope) = grant {
            let once = load(&self.data_dir)
                .iter()
                .find(|plugin| plugin.name == canonical.plugin)
                .and_then(|plugin| plugin.environments.get(&canonical.environment))
                .is_none_or(|environment| environment.once);
            if once {
                return Err(CoreError::Invalid(format!(
                    "{}'s {} allows no grant: each call is approved on its own",
                    canonical.plugin, canonical.environment
                )));
            }
            let (run_hash, until_ms) = match scope {
                GrantScope::Run => (Some(call.run_hash.clone()), None),
                GrantScope::Thread { minutes } => (None, Some(now + i64::from(minutes.clamp(1, 24 * 60)) * 60_000)),
                GrantScope::Workspace => {
                    return Err(CoreError::Invalid("only a folder can be granted always in its workspace".into()));
                }
            };
            self.store
                .add_grant(&GrantRecord {
                    id: 0,
                    thread: call.thread,
                    run_hash,
                    plugin: canonical.plugin.clone(),
                    environment: canonical.environment.clone(),
                    change: call.change,
                    created_ms: now,
                    until_ms,
                    revoked_ms: None,
                    from_call: id,
                    action: None,
                })
                .await?;
        }
        self.store.set_plugin_call(id, "running", None, Some(now)).await?;
        self.run_call(id, &call.workspace, &canonical, channel);
        self.announce(BusEvent::Approvals);
        Ok(())
    }

    /// Takes back the folder `path` from the thread's agents: gone from
    /// their sandbox at the end of their turn.
    pub(crate) fn folder_revoke(&mut self, thread: &str, path: &str) -> Result<(), CoreError> {
        let thread = self.find_thread(thread)?;
        if !crate::folders::revoke(&self.data_dir, &thread.to_string(), path)? {
            return Err(CoreError::Invalid(format!("{path} isn't granted to that thread")));
        }
        self.announce(BusEvent::Approvals);
        Ok(())
    }

    /// Ends grant `id`; calls it covered ask again.
    pub(crate) async fn grant_revoke(&mut self, id: u64) -> Result<(), CoreError> {
        let id = i64::try_from(id).map_err(|_| CoreError::Invalid(format!("no grant {id}")))?;
        self.store.revoke_grant(id, now_ms()).await?.ok_or_else(|| CoreError::Invalid(format!("no grant {id} in force")))?;
        self.announce(BusEvent::Approvals);
        Ok(())
    }

    /// Approvals, newest first: those waiting, a thread's alone when
    /// `thread` names it, and with `all` those answered too.
    pub(crate) async fn approvals_view(&self, thread: Option<&str>, all: bool) -> Result<Vec<ApprovalView>, CoreError> {
        let thread = thread.map(|thread| self.find_thread(thread)).transpose()?;
        let calls = self.store.plugin_calls(thread, (!all).then_some("pending"), LISTED).await?;
        let plugins = load(&self.data_dir);
        let now = now_ms();
        Ok(calls
            .into_iter()
            .filter(|call| call.asked && (all || state_of(call, now) == ApprovalState::Pending))
            .map(|call| {
                let seen = described(&call);
                let once = seen.once.unwrap_or_else(|| {
                    plugins
                        .iter()
                        .find(|plugin| plugin.name == seen.plugin)
                        .and_then(|plugin| plugin.environments.get(&seen.environment))
                        .is_none_or(|environment| environment.once)
                });
                ApprovalView {
                    id: id_of(call.id),
                    thread: call.thread.to_string(),
                    plugin: seen.plugin,
                    tool: seen.tool,
                    environment: seen.environment,
                    target: seen.target,
                    shown: seen.shown,
                    hash: call.hash.clone(),
                    created_ms: call.created_ms,
                    state: state_of(&call, now),
                    outcome: outcome_of(&call),
                    change: call.change,
                    once,
                    card: seen.card,
                    effects: seen.effects,
                    why: seen.why,
                }
            })
            .collect())
    }

    /// What reached the thread's sandboxes through Silt: plugin calls,
    /// grants, and what the gateway refused.
    pub(crate) async fn access_view(&self, thread: &str) -> Result<AccessView, CoreError> {
        let thread = self.find_thread(thread)?;
        let now = now_ms();
        let calls = self
            .store
            .plugin_calls(Some(thread), None, LISTED)
            .await?
            .into_iter()
            .map(|call| {
                let seen = described(&call);
                let state =
                    if state_of(&call, now) == ApprovalState::Expired { "expired".to_owned() } else { call.state.clone() };
                CallView {
                    id: id_of(call.id),
                    plugin: seen.plugin,
                    tool: seen.tool,
                    environment: seen.environment,
                    target: seen.target,
                    shown: seen.shown,
                    change: call.change,
                    asked: call.asked,
                    grant: call.grant.map(id_of),
                    state,
                    outcome: outcome_of(&call),
                    created_ms: call.created_ms,
                    answered_ms: call.answered_ms,
                    card: seen.card,
                    effects: seen.effects,
                }
            })
            .collect();
        let grants = self
            .store
            .grants(thread)
            .await?
            .into_iter()
            .map(|grant| GrantView {
                id: id_of(grant.id),
                active: grant.revoked_ms.is_none() && grant.until_ms.is_none_or(|until| until > now),
                plugin: grant.plugin,
                environment: grant.environment,
                change: grant.change,
                run_only: grant.run_hash.is_some(),
                created_ms: grant.created_ms,
                until_ms: grant.until_ms,
                revoked_ms: grant.revoked_ms,
            })
            .collect();
        let folders = crate::folders::grants(&self.data_dir, &thread.to_string())
            .into_iter()
            .map(|grant| {
                let (run_only, until_ms) = match grant.scope {
                    crate::folders::FolderScope::Run { .. } => (true, None),
                    crate::folders::FolderScope::Thread { until_ms } => (false, until_ms),
                };
                FolderView { path: grant.path, writable: grant.writable, run_only, granted_ms: grant.granted_ms, until_ms }
            })
            .collect();
        let refused = network_refusals(&crate::sandbox::access_log(&self.data_dir, &thread.to_string()));
        #[cfg(unix)]
        let devices = self.devices_view(thread);
        #[cfg(not(unix))]
        let devices = Vec::new();
        Ok(AccessView { calls, grants, refused, devices, folders })
    }
}

impl Core {
    /// What a WebAssembly plugin's call is given of this machine.
    fn call_parts(
        &self,
        thread: ThreadId,
        workspace: &str,
        installed: Installed,
        tool: String,
        environment: Option<String>,
    ) -> CallParts {
        let checkout = self
            .checkouts
            .values()
            .find(|checkout| checkout.record.thread == thread)
            .map(|checkout| checkout.record.path.clone());
        CallParts {
            data_dir: self.data_dir.clone(),
            installed,
            tool,
            thread: thread.to_string(),
            workspace: workspace.to_owned(),
            environment,
            checkout,
            handle: tokio::runtime::Handle::current(),
            policy: silt_workspace::WorkspaceId::new(workspace)
                .ok()
                .and_then(|id| self.workspaces().get(&id).cloned())
                .map(|workspace| self.effective_policy(&workspace)),
            folders: None,
        }
    }

    /// A WebAssembly plugin's call (ADR 0100): prepared by its plugin,
    /// decided by its rules, then run, waited on, or refused.
    async fn start_wasm_call(
        &mut self,
        thread: ThreadId,
        workspace: String,
        run_hash: Vec<u8>,
        installed: Installed,
        tool: String,
        input: Value,
    ) -> Result<(i64, Arc<watch::Sender<Option<Outcome>>>), CoreError> {
        let now = now_ms();
        let plugin = installed.id.clone();
        let parts = self.call_parts(thread, &workspace, installed, tool.clone(), None);
        let wasm = self.approvals.wasm.clone();
        let asked_input = input.clone();
        let (prepared, parts) = tokio::task::spawn_blocking(move || {
            let mut parts = parts;
            let prepared = wasm.prepare(&parts, &asked_input);
            if let Ok(request) = &prepared {
                parts.environment = request.prepared.environment.clone();
            }
            (prepared, parts)
        })
        .await
        .map_err(|error| CoreError::Invalid(error.to_string()))?;
        let mut call = PluginCallRecord {
            id: 0,
            thread,
            run_hash: run_hash.clone(),
            workspace: workspace.clone(),
            request: String::new(),
            hash: String::new(),
            change: false,
            asked: false,
            state: "running".into(),
            outcome: None,
            grant: None,
            created_ms: now,
            answered_ms: None,
            kind: "wasm".into(),
        };
        let request = match prepared {
            Ok(request) => request,
            Err(why) => {
                let asked = serde_json::json!({"plugin": plugin, "tool": tool, "input": input});
                call.request = asked.to_string();
                call.hash = blake3::hash(call.request.as_bytes()).to_hex().to_string();
                call.state = "ended".into();
                let outcome = Outcome::Refused { why };
                call.outcome = Some(serde_json::to_string(&outcome).unwrap_or_default());
                let id = self.store.add_plugin_call(&call).await?;
                self.announce(BusEvent::Approvals);
                return Ok((id, Arc::new(watch::channel(Some(outcome)).0)));
            }
        };
        let change = installed::manifest(&self.data_dir, &plugin)
            .ok()
            .and_then(|manifest| {
                manifest.action(&request.prepared.action).map(|action| action.kind == silt_plugin::ActionKind::Change)
            })
            .unwrap_or(true);
        call.request = serde_json::to_string(&request).unwrap_or_default();
        call.hash = request.prepared.hash();
        call.change = change;
        let Some(mut ask) = asks(&request.decision) else {
            call.state = "ended".into();
            let outcome = Outcome::Refused { why: format!("refused by {}", request.decision.why) };
            call.outcome = Some(serde_json::to_string(&outcome).unwrap_or_default());
            let id = self.store.add_plugin_call(&call).await?;
            self.announce(BusEvent::Approvals);
            return Ok((id, Arc::new(watch::channel(Some(outcome)).0)));
        };
        if ask && !request.decision.once {
            let environment = request.prepared.environment.clone().unwrap_or_default();
            let grant = self
                .store
                .grant_for(thread, &run_hash, &plugin, &environment, change, Some(&request.prepared.action), now)
                .await?;
            if let Some(grant) = grant {
                ask = false;
                call.grant = Some(grant.id);
            }
        }
        call.asked = ask;
        if ask {
            call.state = "pending".into();
        }
        let id = self.store.add_plugin_call(&call).await?;
        let channel = self.approvals.channel(id);
        self.announce(BusEvent::Approvals);
        if !ask {
            let mut parts = parts;
            parts.folders = Some(crate::folders::FolderScope::Run { run: crate::folders::run_id(&run_hash) });
            self.run_wasm_call(id, parts, request, channel.clone());
        }
        Ok((id, channel))
    }

    /// Makes `folders` the workspace's own, as the person chose on the card.
    fn add_workspace_folders(&mut self, workspace: &str, folders: &[(String, bool)]) -> Result<(), CoreError> {
        let id = silt_workspace::WorkspaceId::new(workspace).map_err(|error| CoreError::Invalid(error.to_string()))?;
        let mut all = self.workspaces();
        let found = all
            .workspaces
            .iter_mut()
            .find(|candidate| candidate.id == id)
            .ok_or_else(|| CoreError::Invalid(format!("no workspace {workspace}")))?;
        for (path, writable) in folders {
            *found.policy.paths.entry(path.clone()).or_insert(false) |= *writable;
        }
        self.set_workspaces(all)
    }

    /// The person approved a WebAssembly plugin's call: grant what they
    /// chose, then run it as it was prepared.
    async fn approve_wasm_call(
        &mut self,
        call: PluginCallRecord,
        grant: Option<GrantScope>,
        channel: Arc<watch::Sender<Option<Outcome>>>,
        now: i64,
    ) -> Result<(), CoreError> {
        let request: WasmRequest =
            serde_json::from_str(&call.request).map_err(|_| CoreError::Invalid("this approval's call can't be read".into()))?;
        let prepared = &request.prepared;
        let folders: Vec<(String, bool)> = prepared
            .effects
            .iter()
            .filter_map(|effect| match effect {
                silt_plugin::Effect::Reach { path, writable } => Some((path.clone(), *writable)),
                _ => None,
            })
            .collect();
        let mut folder_scope = crate::folders::FolderScope::Run { run: crate::folders::run_id(&call.run_hash) };
        if grant == Some(GrantScope::Workspace) {
            if folders.is_empty() {
                return Err(CoreError::Invalid("only a folder can be granted always in its workspace".into()));
            }
            self.add_workspace_folders(&call.workspace, &folders)?;
            folder_scope = crate::folders::FolderScope::Thread { until_ms: None };
        } else if let Some(scope) = grant {
            if request.decision.once {
                return Err(CoreError::Invalid(format!("{} allows no grant: each is approved on its own", prepared.action)));
            }
            let (run_hash, until_ms) = match scope {
                GrantScope::Run => (Some(call.run_hash.clone()), None),
                GrantScope::Thread { minutes } => (None, Some(now + i64::from(minutes.clamp(1, 24 * 60)) * 60_000)),
                GrantScope::Workspace => unreachable!("handled above"),
            };
            if let Some(until_ms) = until_ms {
                folder_scope = crate::folders::FolderScope::Thread { until_ms: Some(until_ms) };
            }
            self.store
                .add_grant(&GrantRecord {
                    id: 0,
                    thread: call.thread,
                    run_hash,
                    plugin: prepared.plugin.clone(),
                    environment: prepared.environment.clone().unwrap_or_default(),
                    change: call.change,
                    created_ms: now,
                    until_ms,
                    revoked_ms: None,
                    from_call: call.id,
                    action: Some(prepared.action.clone()),
                })
                .await?;
        }
        let installed = installed::find(&self.data_dir, &prepared.plugin)
            .ok_or_else(|| CoreError::Invalid(format!("{} isn't installed any more", prepared.plugin)))?;
        self.store.set_plugin_call(call.id, "running", None, Some(now)).await?;
        let mut parts =
            self.call_parts(call.thread, &call.workspace, installed, prepared.tool.clone(), prepared.environment.clone());
        parts.folders = Some(folder_scope);
        self.run_wasm_call(call.id, parts, request, channel);
        self.announce(BusEvent::Approvals);
        Ok(())
    }

    /// Runs call `id` on a blocking thread, as it was prepared, and ends it
    /// with how it went.
    fn run_wasm_call(&self, id: i64, parts: CallParts, request: WasmRequest, channel: Arc<watch::Sender<Option<Outcome>>>) {
        let (store, bus, wasm) = (self.store.clone(), self.bus.clone(), self.approvals.wasm.clone());
        tokio::spawn(async move {
            let ran = tokio::task::spawn_blocking(move || wasm.run(&parts, &request)).await;
            let outcome = match ran {
                Ok(Ok(ran)) if !ran.refused.is_empty() => {
                    let refused: Vec<String> = ran.refused.iter().map(ToString::to_string).collect();
                    Outcome::Refused { why: format!("Silt refused what wasn't approved: {}", refused.join("; ")) }
                }
                Ok(Ok(ran)) => match ran.output {
                    Ok(output) => Outcome::Done { output: output.text },
                    Err(error) if ran.observed.is_empty() => Outcome::Failed { error },
                    Err(error) => Outcome::Unknown { why: format!("it failed after sending something: {error}") },
                },
                Ok(Err(error)) => Outcome::Failed { error },
                Err(error) => Outcome::Unknown { why: error.to_string() },
            };
            end(&store, &bus, &channel, id, outcome).await;
        });
    }
}

/// The last refusals in a thread's gateway log, newest first.
fn network_refusals(log: &std::path::Path) -> Vec<NetworkRefusal> {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let mut refused: Vec<NetworkRefusal> = text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|line| {
            Some(NetworkRefusal {
                time_ms: line.get("time_ms")?.as_i64()?,
                host: line.get("host")?.as_str()?.to_owned(),
                port: u16::try_from(line.get("port")?.as_u64()?).ok()?,
                why: line.get("refused")?.as_str()?.to_owned(),
            })
        })
        .collect();
    refused.reverse();
    refused.truncate(50);
    refused
}

/// Answers `reply` with the call's outcome once there is one, or after
/// [`WAIT`] with its being pending.
fn wait_on(outcome: Arc<watch::Sender<Option<Outcome>>>, id: i64, reply: oneshot::Sender<Response>) {
    let mut seen = outcome.subscribe();
    tokio::spawn(async move {
        let done = tokio::time::timeout(WAIT, seen.wait_for(Option::is_some)).await;
        let answer = match done {
            Ok(Ok(outcome)) => outcome.clone().unwrap_or(Outcome::Pending { approval: id_of(id) }),
            _ => Outcome::Pending { approval: id_of(id) },
        };
        let _ = reply.send(Response::ok(answer));
    });
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;
    use silt_store::ThreadRecord;
    use silt_types::{ActorId, FamilyId};

    use super::super::{Environment, Kind, Plugin, Rule, put};
    use super::*;

    fn fleet(once: bool) -> Plugin {
        Plugin {
            name: "test fleet".into(),
            description: String::new(),
            kind: Kind::Command { argv: vec!["echo".into(), "ran on {target}".into()] },
            workspaces: vec!["work".into(), "personal".into()],
            environments: BTreeMap::from([(
                "fleet".to_owned(),
                Environment {
                    target: "car-*.fleet.vpn".into(),
                    reads: Rule::Allow,
                    changes: Rule::Ask,
                    data: Vec::new(),
                    credential: None,
                    once,
                    direct: false,
                    variable: None,
                },
            )]),
        }
    }

    async fn core(data: &std::path::Path) -> Core {
        let store = Store::open(data).await.expect("opens");
        let record = ThreadRecord {
            id: ThreadId(7),
            family: FamilyId(9),
            repo_root: std::path::PathBuf::new(),
            repo: String::new(),
            created_ms: 1,
            owner: String::new(),
        };
        store.create_thread(&record).await.expect("thread");
        store.create_thread(&ThreadRecord { id: ThreadId(8), family: FamilyId(10), ..record }).await.expect("thread");
        store.add_run_token(blake3::hash(b"work-run").as_bytes(), ThreadId(7), "work", None, 1).await.expect("token");
        store.add_run_token(blake3::hash(b"work-again").as_bytes(), ThreadId(7), "work", None, 1).await.expect("token");
        store.add_run_token(blake3::hash(b"other-thread").as_bytes(), ThreadId(8), "personal", None, 1).await.expect("token");
        let (events, _inbox) = tokio::sync::mpsc::unbounded_channel();
        Core::load(store, ActorId(1), data, events).await.expect("loads")
    }

    fn outcome(response: Response) -> Outcome {
        match response {
            Response::Ok { data } => serde_json::from_value(data).expect("an outcome"),
            Response::Error { message } => panic!("{message}"),
        }
    }

    async fn call(core: &mut Core, token: &str) -> oneshot::Receiver<Response> {
        let (reply, answer) = oneshot::channel();
        let input = json!({"environment": "fleet", "target": "car-1.fleet.vpn", "args": ["now"]});
        core.plugin_call(token, "test_fleet_run", input, reply).await;
        answer
    }

    fn probe_installed(data: &std::path::Path) {
        let manifest_text = silt_plugin_host::testing::PROBE_MANIFEST.to_owned();
        let package = super::super::install::Package {
            manifest: silt_plugin::Manifest::parse(&manifest_text).expect("the probe's manifest"),
            manifest_text,
            wasm: silt_plugin_host::testing::plugin("silt-plugin-probe"),
            readme: Some("# Probe".into()),
        };
        super::super::install::install(
            data,
            &package,
            installed::PluginSource::Dev { path: "plugins/probe".into() },
            &["work".into()],
            false,
        )
        .expect("installs");
    }

    async fn call_tool(core: &mut Core, token: &str, tool: &str) -> oneshot::Receiver<Response> {
        let (reply, answer) = oneshot::channel();
        core.plugin_call(token, tool, json!({"name": "ada"}), reply).await;
        answer
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_webassembly_plugins_call_waits_for_an_untrusted_publisher_then_runs_as_prepared() {
        let data = tempfile::tempdir().expect("temp dir");
        probe_installed(data.path());
        let mut core = core(data.path()).await;
        let (reply, tools) = oneshot::channel();
        core.plugin_tools("work-run", reply).await;
        let Response::Ok { data: offered } = tools.await.expect("tools") else { panic!("no tools") };
        let names: Vec<String> =
            offered.as_array().expect("a list").iter().filter_map(|tool| tool["name"].as_str().map(str::to_owned)).collect();
        assert!(names.contains(&"probe_files".to_owned()), "{names:?}");

        let (reply, other) = oneshot::channel();
        core.plugin_tools("other-thread", reply).await;
        let Response::Ok { data: elsewhere } = other.await.expect("tools") else { panic!("no tools") };
        assert!(!elsewhere.to_string().contains("probe_"), "not on in personal");

        let waiting = call_tool(&mut core, "work-run", "probe_files").await;
        let pending = core.approvals_view(None, false).await.expect("approvals");
        assert_eq!(pending.len(), 1, "its publisher isn't trusted, so it asks");
        let approval = &pending[0];
        assert_eq!(approval.plugin, "test.probe");
        assert!(approval.card.as_ref().is_some_and(|card| card.title == "files"));
        assert!(approval.why.as_deref().is_some_and(|why| why.contains("isn't trusted")));
        core.approval_answer(approval.id, true, &approval.hash, Some(GrantScope::Run)).await.expect("approves");
        assert_eq!(outcome(waiting.await.expect("answers")), Outcome::Done { output: "wrote /data/note; no /etc/passwd".into() });

        // The grant was for this action, for this run.
        let again = call_tool(&mut core, "work-run", "probe_files").await;
        assert!(core.approvals_view(None, false).await.expect("approvals").is_empty());
        assert!(matches!(outcome(again.await.expect("answers")), Outcome::Done { .. }));

        // Trusted, its own default lets a read go without asking.
        installed::set_trusted(data.path(), "test", true).expect("trusts");
        let trusted = call_tool(&mut core, "work-again", "probe_files").await;
        assert!(core.approvals_view(None, false).await.expect("approvals").is_empty());
        assert!(matches!(outcome(trusted.await.expect("answers")), Outcome::Done { .. }));
        let access = core.access_view(&ThreadId(7).to_string()).await.expect("access");
        assert_eq!(access.calls.len(), 3);
        assert!(access.grants.iter().any(|grant| grant.plugin == "test.probe"));
    }

    async fn ask_for(core: &mut Core, token: &str, path: &std::path::Path, writable: bool) -> oneshot::Receiver<Response> {
        let (reply, answer) = oneshot::channel();
        let input = json!({"path": path.to_string_lossy(), "writable": writable, "reason": "the app uses it"});
        core.plugin_call(token, "folders_request", input, reply).await;
        answer
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_asks_for_a_folder_and_the_workspaces_rules_answer_first() {
        let data = tempfile::tempdir().expect("temp dir");
        let mine = tempfile::tempdir().expect("temp dir");
        let root = std::fs::canonicalize(mine.path()).expect("real");
        let (lib, secret, other) = (root.join("lib"), root.join("secret"), root.join("other"));
        for dir in [&lib, &secret, &other] {
            std::fs::create_dir_all(dir).expect("made");
        }
        let mut core = core(data.path()).await;
        let mut work = silt_workspace::Workspace::new(silt_workspace::WorkspaceId::new("work").expect("an id"), "Work");
        work.policy.folders = vec![
            silt_workspace::FolderRule::parse(&format!("{}:allow", lib.display())).expect("a rule"),
            silt_workspace::FolderRule::parse(&format!("{}:never", secret.display())).expect("a rule"),
        ];
        let mut all = silt_workspace::Workspaces::single(None);
        all.workspaces.push(work);
        core.set_workspaces(all).expect("saved");
        let thread = ThreadId(7).to_string();
        let this_run = crate::folders::run_of_token("work-run");

        // Allowed by its rule, it's granted to this run without asking.
        let answer = ask_for(&mut core, "work-run", &lib, false).await;
        let Outcome::Done { output } = outcome(answer.await.expect("answers")) else { panic!("not done") };
        assert!(output.contains("End your turn"), "{output}");
        let lib_text = lib.to_string_lossy().into_owned();
        assert_eq!(
            crate::folders::granted(data.path(), &thread, Some(&this_run), 1),
            BTreeMap::from([(lib_text.clone(), false)])
        );
        assert!(crate::folders::granted(data.path(), &thread, Some("another run"), 1).is_empty());
        assert!(crate::folders::take_pending(data.path(), &thread), "the agent restarts with it");

        // Writing isn't given where its rule doesn't give it, and never is never.
        for (path, writable, said) in [(&lib, true, "doesn't give writing"), (&secret, false, ":never")] {
            let answer = ask_for(&mut core, "work-run", path, writable).await;
            let Outcome::Refused { why } = outcome(answer.await.expect("answers")) else { panic!("not refused") };
            assert!(why.contains(said), "{why}");
        }
        // Silt's own refusals hold whatever the rules say.
        let home = silt_os::home_dir().expect("a home");
        let answer = ask_for(&mut core, "work-run", &home, false).await;
        let Outcome::Refused { why } = outcome(answer.await.expect("answers")) else { panic!("not refused") };
        assert!(why.contains("never handed over"), "{why}");

        // One no rule names is asked; granted always, it's the workspace's own.
        let waiting = ask_for(&mut core, "work-run", &other, true).await;
        let pending = core.approvals_view(None, false).await.expect("approvals");
        assert_eq!(pending.len(), 1);
        let card = pending[0].card.as_ref().expect("a card");
        assert!(card.title.contains("read and write"), "{}", card.title);
        assert!(super::super::wasm::card_text(card).contains("The agent says: the app uses it"));
        core.approval_answer(pending[0].id, true, &pending[0].hash, Some(GrantScope::Workspace)).await.expect("approves");
        assert!(matches!(outcome(waiting.await.expect("answers")), Outcome::Done { .. }));
        let saved = core.workspaces();
        let work = saved.workspaces.iter().find(|workspace| workspace.id.as_str() == "work").expect("work");
        assert_eq!(work.policy.paths.get(other.to_string_lossy().as_ref()), Some(&true));
        assert!(
            crate::folders::granted(data.path(), &thread, Some("a later run"), 1).contains_key(other.to_string_lossy().as_ref())
        );

        // The Access tab lists them, and taking one back narrows at the next restart.
        let access = core.access_view(&thread).await.expect("access");
        assert_eq!(access.folders.len(), 2, "{:?}", access.folders);
        assert!(access.folders.iter().any(|folder| folder.path == lib_text && folder.run_only && !folder.writable));
        core.folder_revoke(&thread, &lib_text).expect("taken back");
        assert!(!crate::folders::granted(data.path(), &thread, Some(&this_run), 1).contains_key(&lib_text));
        assert!(crate::folders::take_pending(data.path(), &thread));
        assert!(core.folder_revoke(&thread, &lib_text).is_err(), "not granted any more");
    }

    fn git(dir: &std::path::Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=Ada", "-c", "user.email=ada@example.com", "-c", "init.defaultBranch=main"])
            // The test's own git ignores the agent's hostile config, as Silt's does.
            .args(["-c", "core.fsmonitor=false", "-c", "core.hooksPath=/dev/null"])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .expect("git runs");
        assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    /// GitHub's API as far as the plugin's calls here need it, keeping
    /// what each change was sent with.
    async fn github_stand_in(sent: Arc<std::sync::Mutex<Vec<(String, String, String)>>>) -> u16 {
        use axum::routing::{get, post};
        let kept = sent.clone();
        let app = axum::Router::new()
            .route("/repos/acme/api", get(|| async { axum::Json(json!({"default_branch": "main", "full_name": "acme/api"})) }))
            .route(
                "/repos/acme/api/branches/{*branch}",
                get(|axum::extract::Path(branch): axum::extract::Path<String>| async move {
                    if branch == "main" {
                        (axum::http::StatusCode::OK, axum::Json(json!({"name": "main", "protected": true})))
                    } else {
                        (axum::http::StatusCode::NOT_FOUND, axum::Json(json!({"message": "Branch not found"})))
                    }
                }),
            )
            .route(
                "/repos/acme/api/pulls",
                post(move |headers: axum::http::HeaderMap, body: String| {
                    let kept = kept.clone();
                    async move {
                        let auth =
                            headers.get("authorization").and_then(|value| value.to_str().ok()).unwrap_or_default().to_owned();
                        kept.lock().unwrap().push(("POST /pulls".to_owned(), auth, body));
                        (axum::http::StatusCode::CREATED, axum::Json(json!({"number": 7})))
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("binds");
        let port = listener.local_addr().expect("an address").port();
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        port
    }

    async fn github(core: &mut Core, tool: &str, input: Value) -> oneshot::Receiver<Response> {
        let (reply, answer) = oneshot::channel();
        core.plugin_call("work-run", tool, input, reply).await;
        answer
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn github_pushes_exactly_the_approved_commit_from_its_own_mirror() {
        let data = tempfile::tempdir().expect("temp dir");
        let remotes = tempfile::tempdir().expect("temp dir");
        let remotes = std::fs::canonicalize(remotes.path()).expect("real");
        let sent = Arc::new(std::sync::Mutex::new(Vec::new()));
        let port = github_stand_in(sent.clone()).await;
        std::fs::create_dir_all(data.path().join("plugins")).expect("made");
        std::fs::write(
            data.path().join("plugins/redirect.json"),
            json!({"https://api.github.com": format!("http://127.0.0.1:{port}"), "https://github.com": format!("file://{}", remotes.display())})
                .to_string(),
        )
        .expect("written");
        crate::credentials::save(
            data.path(),
            &[crate::credentials::StoredCredential {
                name: "gh".into(),
                workspace: "work".into(),
                host: "api.github.com".into(),
                path: "/".into(),
                header: "authorization".into(),
                scheme: "Bearer".into(),
                source: crate::credentials::Source::Command { argv: vec!["echo".into(), "s3cret".into()] },
                placeholder: None,
            }],
        )
        .expect("saved");

        // GitHub's repo, and the agent's clone of it in its work area, whose config is hostile.
        let seed = remotes.join("seed");
        std::fs::create_dir_all(&seed).expect("made");
        git(&seed, &["init", "-q"]);
        std::fs::write(seed.join("README.md"), "api\n").expect("written");
        git(&seed, &["add", "."]);
        git(&seed, &["commit", "-qm", "start"]);
        git(&remotes, &["clone", "-q", "--bare", "seed", "acme/api.git"]);
        let bare = remotes.join("acme/api.git");
        let main = git(&bare, &["rev-parse", "refs/heads/main"]);
        let thread = ThreadId(7).to_string();
        let work = data.path().join("work").join(&thread);
        std::fs::create_dir_all(&work).expect("made");
        git(&work, &["clone", "-q", &bare.to_string_lossy(), "api"]);
        let clone = work.join("api");
        let marker = remotes.join("pwned");
        for (key, value) in [
            ("core.fsmonitor", format!("touch {}", marker.display())),
            ("uploadpack.packObjectsHook", format!("touch {}", marker.display())),
            ("core.hooksPath", remotes.display().to_string()),
        ] {
            git(&clone, &["config", key, &value]);
        }
        git(&clone, &["checkout", "-qb", "ada/fix"]);
        std::fs::write(clone.join("fix.rs"), "fn fix() {}\n").expect("written");
        git(&clone, &["add", "."]);
        git(&clone, &["commit", "-qm", "Fix the thing"]);
        let first = git(&clone, &["rev-parse", "HEAD"]);

        let mut core = core(data.path()).await;
        installed::update(data.path(), "silt.github", |installed| {
            installed.workspaces.push("work".into());
            installed.environments.insert("acme".into(), vec!["acme/*".into()]);
            installed.credentials.insert("token".into(), "gh".into());
            Ok(())
        })
        .expect("on in work");

        // Reads go: the mirror is made where the agent sees it.
        let cloned = github(&mut core, "github_clone", json!({"repo": "acme/api"})).await;
        let Outcome::Done { output } = outcome(cloned.await.expect("answers")) else { panic!("not cloned") };
        assert!(output.contains("/silt/github/acme/api.git"), "{output}");
        let mirror = installed::thread_dir(data.path(), "silt.github", &thread).join("agent/acme/api.git");
        assert!(mirror.join("HEAD").is_file());

        // A push asks, shows its commits, and lands as approved.
        let push = json!({"repo": "acme/api", "path": "/silt/work/api", "branch": "ada/fix"});
        let waiting = github(&mut core, "github_push", push.clone()).await;
        let pending = core.approvals_view(None, false).await.expect("approvals");
        assert_eq!(pending.len(), 1);
        let card = pending[0].card.as_ref().expect("a card");
        assert_eq!(card.title, "Push ada/fix to acme/api");
        let shown = super::super::wasm::card_text(card);
        assert!(shown.contains("Fix the thing") && shown.contains("fix.rs  +1 -0"), "{shown}");
        core.approval_answer(pending[0].id, true, &pending[0].hash, None).await.expect("approves");
        let Outcome::Done { .. } = outcome(waiting.await.expect("answers")) else { panic!("not pushed") };
        assert_eq!(git(&bare, &["rev-parse", "refs/heads/ada/fix"]), first);

        // Approved where the branch was: moved meanwhile, nothing is pushed.
        std::fs::write(clone.join("fix.rs"), "fn fix() { 1 }\n").expect("written");
        git(&clone, &["commit", "-qam", "Fix it more"]);
        let waiting = github(&mut core, "github_push", push.clone()).await;
        let pending = core.approvals_view(None, false).await.expect("approvals");
        git(&bare, &["update-ref", "refs/heads/ada/fix", &main]);
        core.approval_answer(pending[0].id, true, &pending[0].hash, None).await.expect("approves");
        assert!(!matches!(outcome(waiting.await.expect("answers")), Outcome::Done { .. }), "the lease holds");
        assert_eq!(git(&bare, &["rev-parse", "refs/heads/ada/fix"]), main);

        // A branch rewritten from under what's there is a force-push: never.
        git(&bare, &["update-ref", "refs/heads/ada/fix", &first]);
        git(&clone, &["reset", "-q", "--hard", &main]);
        std::fs::write(clone.join("other.rs"), "fn other() {}\n").expect("written");
        git(&clone, &["add", "."]);
        git(&clone, &["commit", "-qm", "Start over"]);
        let forced = github(&mut core, "github_push", push.clone()).await;
        let Outcome::Refused { why } = outcome(forced.await.expect("answers")) else { panic!("not refused") };
        assert!(why.contains("Force-push"), "{why}");
        assert_eq!(git(&bare, &["rev-parse", "refs/heads/ada/fix"]), first);

        // The default branch asks each time: no grant covers it.
        git(&clone, &["checkout", "-q", "main"]);
        git(&clone, &["merge", "-q", "--ff-only", "ada/fix"]);
        let _waiting =
            github(&mut core, "github_push", json!({"repo": "acme/api", "path": "/silt/work/api", "branch": "main"})).await;
        let pending = core.approvals_view(None, false).await.expect("approvals");
        assert!(pending[0].once, "{:?}", pending[0]);
        assert!(core.approval_answer(pending[0].id, true, &pending[0].hash, Some(GrantScope::Run)).await.is_err());
        core.approval_answer(pending[0].id, false, &pending[0].hash, None).await.expect("rejects");
        assert_eq!(git(&bare, &["rev-parse", "refs/heads/main"]), main);

        // A pull request is opened with exactly the text approved, and the token added on the way.
        let opened = github(
            &mut core,
            "github_pr_create",
            json!({"repo": "acme/api", "head": "ada/fix", "base": "main", "title": "Fix", "body": "Why\u{202e}"}),
        )
        .await;
        let pending = core.approvals_view(None, false).await.expect("approvals");
        assert_eq!(pending.len(), 1);
        core.approval_answer(pending[0].id, true, &pending[0].hash, None).await.expect("approves");
        assert!(matches!(outcome(opened.await.expect("answers")), Outcome::Done { .. }));
        let sent = sent.lock().unwrap().clone();
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(sent[0].1, "Bearer s3cret");
        let body: Value = serde_json::from_str(&sent[0].2).expect("json");
        assert_eq!(body, json!({"base": "main", "body": "Why\u{202e}", "draft": false, "head": "ada/fix", "title": "Fix"}));
        assert!(!marker.exists(), "nothing of the agent's repo's config ran");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_never_rule_refuses_before_the_plugin_runs() {
        let data = tempfile::tempdir().expect("temp dir");
        probe_installed(data.path());
        installed::update(data.path(), "test.probe", |installed| {
            installed.rules.push(silt_plugin::ScopedRule {
                action: "look".into(),
                field: None,
                pattern: None,
                rule: silt_plugin::Rule::Never,
                once: false,
                source: silt_plugin::RuleSource::Yours,
            });
            Ok(())
        })
        .expect("a rule");
        let mut core = core(data.path()).await;
        let refused = call_tool(&mut core, "work-run", "probe_files").await;
        let Outcome::Refused { why } = outcome(refused.await.expect("answers")) else { panic!("not refused") };
        assert!(why.contains("your rule for Look"), "{why}");
        assert!(!data.path().join("plugins/test.probe/data/note").exists(), "it never ran");
    }

    #[tokio::test]
    async fn a_change_runs_once_after_the_person_approves_what_they_saw() {
        let data = tempfile::tempdir().expect("temp dir");
        put(data.path(), fleet(false)).expect("saves");
        let mut core = core(data.path()).await;

        let waiting = call(&mut core, "work-run").await;
        let pending = core.approvals_view(None, false).await.expect("approvals");
        assert_eq!(pending.len(), 1);
        let approval = &pending[0];
        assert_eq!(approval.shown, "echo 'ran on car-1.fleet.vpn' now");
        assert!(core.approval_answer(approval.id, true, "not-what-was-shown", None).await.is_err());
        core.approval_answer(approval.id, true, &approval.hash, None).await.expect("approves");
        assert!(core.approval_answer(approval.id, true, &approval.hash, None).await.is_err(), "only once");
        assert_eq!(
            outcome(waiting.await.expect("answers")),
            Outcome::Done { output: "exit 0\nran on car-1.fleet.vpn now\n".into() }
        );

        let (reply, again) = oneshot::channel();
        core.plugin_wait("other-thread", approval.id, reply).await;
        assert!(matches!(again.await.expect("answers"), Response::Error { .. }), "another thread's call");
        let access = core.access_view(&ThreadId(7).to_string()).await.expect("access");
        assert_eq!(access.calls.len(), 1);
        assert_eq!(access.calls[0].state, "ended");
    }

    #[tokio::test]
    async fn a_grant_lets_the_same_kind_of_call_run_until_revoked_and_never_in_a_fork() {
        let data = tempfile::tempdir().expect("temp dir");
        put(data.path(), fleet(false)).expect("saves");
        let mut core = core(data.path()).await;
        let first = call(&mut core, "work-run").await;
        let approval = core.approvals_view(None, false).await.expect("approvals").remove(0);
        core.approval_answer(approval.id, true, &approval.hash, Some(GrantScope::Thread { minutes: 60 }))
            .await
            .expect("approves");
        assert!(matches!(outcome(first.await.expect("answers")), Outcome::Done { .. }));

        let second = call(&mut core, "work-again").await;
        assert!(matches!(outcome(second.await.expect("answers")), Outcome::Done { .. }), "covered by the thread's grant");
        let elsewhere = call(&mut core, "other-thread").await;
        assert!(core.approvals_view(None, false).await.expect("approvals").len() == 1, "another thread asks");
        drop(elsewhere);

        let access = core.access_view(&ThreadId(7).to_string()).await.expect("access");
        let grant = access.grants[0].id;
        assert!(access.calls[0].grant == Some(grant));
        core.grant_revoke(grant).await.expect("revokes");
        let _third = call(&mut core, "work-run").await;
        assert_eq!(core.approvals_view(Some(&ThreadId(7).to_string()), false).await.expect("approvals").len(), 1, "asks again");
    }

    #[tokio::test]
    async fn production_once_allows_no_grant_and_a_call_cut_off_reads_unknown() {
        let data = tempfile::tempdir().expect("temp dir");
        put(data.path(), fleet(true)).expect("saves");
        let mut core = core(data.path()).await;
        let _waiting = call(&mut core, "work-run").await;
        let approval = core.approvals_view(None, false).await.expect("approvals").remove(0);
        assert!(approval.once);
        let refused = core.approval_answer(approval.id, true, &approval.hash, Some(GrantScope::Run)).await.unwrap_err();
        assert!(refused.to_string().contains("allows no grant"), "{refused}");

        let id = i64::try_from(approval.id).expect("id");
        core.store.set_plugin_call(id, "running", None, Some(1)).await.expect("runs");
        core.recover_plugin_calls().await.expect("recovers");
        let call = core.store.plugin_call(id).await.expect("reads").expect("kept");
        assert!(matches!(outcome_of(&call), Some(Outcome::Unknown { .. })));
    }
}
