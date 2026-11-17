//! A run's plugin calls (ADR 0086), kept in the store: those that ran,
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
        let (thread, workspace) =
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
        let (data_dir, listed) = (self.data_dir.clone(), self.approvals.listed.clone());
        tokio::spawn(async move {
            let plugins = load(&data_dir);
            for plugin in plugins.iter().filter(|plugin| plugin.workspaces.contains(&workspace)) {
                if let Some(offered) = super::list_mcp(&data_dir, plugin, &workspace).await {
                    listed.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(plugin.name.clone(), offered);
                }
            }
            let known = listed.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
            let _ = reply.send(Response::ok(tools(&plugins, &workspace, &known)));
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
                .grant_for(thread, &run_hash, &prepared.canonical.plugin, &prepared.canonical.environment, prepared.change, now)
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
        let canonical = canonical_of(&call);
        if !approve {
            let outcome = Outcome::Refused { why: "the person rejected it".into() };
            let json = serde_json::to_string(&outcome).unwrap_or_default();
            self.store.set_plugin_call(id, "rejected", Some(&json), Some(now)).await?;
            channel.send_replace(Some(outcome));
            self.announce(BusEvent::Approvals);
            return Ok(());
        }
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
                })
                .await?;
        }
        self.store.set_plugin_call(id, "running", None, Some(now)).await?;
        self.run_call(id, &call.workspace, &canonical, channel);
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
                let canonical = canonical_of(&call);
                let once = plugins
                    .iter()
                    .find(|plugin| plugin.name == canonical.plugin)
                    .and_then(|plugin| plugin.environments.get(&canonical.environment))
                    .is_none_or(|environment| environment.once);
                ApprovalView {
                    id: id_of(call.id),
                    thread: call.thread.to_string(),
                    plugin: canonical.plugin,
                    tool: canonical.tool,
                    environment: canonical.environment,
                    target: canonical.target,
                    shown: shown_input(&canonical.input),
                    hash: call.hash.clone(),
                    created_ms: call.created_ms,
                    state: state_of(&call, now),
                    outcome: outcome_of(&call),
                    change: call.change,
                    once,
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
                let canonical = canonical_of(&call);
                let state =
                    if state_of(&call, now) == ApprovalState::Expired { "expired".to_owned() } else { call.state.clone() };
                CallView {
                    id: id_of(call.id),
                    plugin: canonical.plugin,
                    tool: canonical.tool,
                    environment: canonical.environment,
                    target: canonical.target,
                    shown: shown_input(&canonical.input),
                    change: call.change,
                    asked: call.asked,
                    grant: call.grant.map(id_of),
                    state,
                    outcome: outcome_of(&call),
                    created_ms: call.created_ms,
                    answered_ms: call.answered_ms,
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
        let refused = network_refusals(&crate::sandbox::access_log(&self.data_dir, &thread.to_string()));
        #[cfg(unix)]
        let devices = self.devices_view(thread);
        #[cfg(not(unix))]
        let devices = Vec::new();
        Ok(AccessView { calls, grants, refused, devices })
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
        store.add_run_token(blake3::hash(b"work-run").as_bytes(), ThreadId(7), "work", 1).await.expect("token");
        store.add_run_token(blake3::hash(b"work-again").as_bytes(), ThreadId(7), "work", 1).await.expect("token");
        store.add_run_token(blake3::hash(b"other-thread").as_bytes(), ThreadId(8), "personal", 1).await.expect("token");
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
