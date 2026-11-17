//! Plugins (ADR 0093): what a sandboxed agent reaches production and other
//! systems through. Each machine keeps its plugins in `plugins.json`, each
//! turned on in some of its workspaces; their calls run here, outside every
//! sandbox, with credentials the agent never holds. A call its rules say
//! asks waits as an approval, bound to the hash of Silt's own copy of the
//! request: only that copy runs, once, and only after the person says so.
//! What comes back passes the environment's data rules first.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use silt_access::{Canonical, DataClass};
use tokio::sync::{oneshot, watch};

use crate::CoreError;
use crate::core::{Core, now_ms, to_response};
use crate::protocol::{BusEvent, Response};

const FILE: &str = "plugins.json";

/// How long a call waits for the person before the agent is told to wait
/// on, and how long an approval stays open.
const WAIT: Duration = Duration::from_secs(45);
const OPEN_MS: i64 = 15 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Plugin {
    /// Its name, which its tools start with.
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub kind: Kind,
    /// The workspaces it's on in, by id.
    #[serde(default)]
    pub workspaces: Vec<String>,
    /// Its environments, such as `staging` and `production`, by name.
    pub environments: BTreeMap<String, Environment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Kind {
    /// A GraphQL API: queries are reads, mutations and subscriptions changes.
    Graphql,
    /// A program run here, such as `ssh` or `kubectl`: `argv` with
    /// `{target}` in place of the target, the agent's arguments after it.
    /// Every call counts as a change.
    Command { argv: Vec<String> },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    #[default]
    Allow,
    Ask,
}

fn ask() -> Rule {
    Rule::Ask
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Environment {
    /// A GraphQL endpoint's URL, or the targets a command may name, such
    /// as `car-*.fleet.vpn`.
    pub target: String,
    #[serde(default)]
    pub reads: Rule,
    #[serde(default = "ask")]
    pub changes: Rule,
    /// What a result may never carry to an agent.
    #[serde(default)]
    pub data: Vec<DataClass>,
    /// A credential of `silt credential` added to its requests.
    #[serde(default)]
    pub credential: Option<String>,
}

pub fn load(data_dir: &Path) -> Vec<Plugin> {
    std::fs::read(data_dir.join(FILE)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

pub fn save(data_dir: &Path, plugins: &[Plugin]) -> Result<(), CoreError> {
    let bytes = serde_json::to_vec_pretty(plugins).map_err(|error| CoreError::Invalid(error.to_string()))?;
    silt_os::write_private(&data_dir.join(FILE), &bytes)?;
    Ok(())
}

/// Adds `plugin`, or replaces the one of its name.
pub fn put(data_dir: &Path, plugin: Plugin) -> Result<(), CoreError> {
    let named = !plugin.name.is_empty() && plugin.name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ' '));
    if !named {
        return Err(CoreError::Invalid("a plugin's name is letters, digits, spaces, `-` and `_`".into()));
    }
    if tool_name(&plugin.name, "").starts_with("silt") {
        return Err(CoreError::Invalid("Silt's own tools start with `silt`; name the plugin otherwise".into()));
    }
    if plugin.environments.is_empty() {
        return Err(CoreError::Invalid(format!("{} has no environments", plugin.name)));
    }
    if let Kind::Command { argv } = &plugin.kind
        && argv.is_empty()
    {
        return Err(CoreError::Invalid(format!("{} runs no program", plugin.name)));
    }
    let mut plugins = load(data_dir);
    let tool = tool_name(&plugin.name, "");
    if plugins.iter().any(|other| other.name != plugin.name && tool_name(&other.name, "") == tool) {
        return Err(CoreError::Invalid(format!("another plugin's tools would share {}'s names", plugin.name)));
    }
    plugins.retain(|other| other.name != plugin.name);
    plugins.push(plugin);
    plugins.sort_by(|one, other| one.name.cmp(&other.name));
    save(data_dir, &plugins)
}

pub fn remove(data_dir: &Path, name: &str) -> Result<(), CoreError> {
    let mut plugins = load(data_dir);
    let before = plugins.len();
    plugins.retain(|plugin| plugin.name != name);
    if plugins.len() == before {
        return Err(CoreError::Invalid(format!("no plugin {name}")));
    }
    save(data_dir, &plugins)
}

/// A tool a plugin offers agents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PluginTool {
    pub name: String,
    pub description: String,
    #[cfg_attr(test, ts(type = "unknown"))]
    pub input_schema: Value,
}

fn tool_name(plugin: &str, verb: &str) -> String {
    let base: String = plugin.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    format!("{base}_{verb}")
}

/// The tools of the plugins on in `workspace`.
pub fn tools(plugins: &[Plugin], workspace: &str) -> Vec<PluginTool> {
    let mut tools = Vec::new();
    for plugin in plugins.iter().filter(|plugin| plugin.workspaces.iter().any(|on| on == workspace)) {
        let environments: Vec<&String> = plugin.environments.keys().collect();
        let environment = json!({"type": "string", "enum": environments, "description": "Which of its environments"});
        let about = if plugin.description.is_empty() { plugin.name.clone() } else { plugin.description.clone() };
        match &plugin.kind {
            Kind::Graphql => {
                let schema = json!({"type": "object", "required": ["environment", "query"], "properties": {
                    "environment": environment,
                    "query": {"type": "string", "description": "The GraphQL document"},
                    "variables": {"type": "object"}
                }});
                tools.push(PluginTool {
                    name: tool_name(&plugin.name, "query"),
                    description: format!("{about}: run a GraphQL query, a read. Fields its data rules keep from agents are refused."),
                    input_schema: schema.clone(),
                });
                tools.push(PluginTool {
                    name: tool_name(&plugin.name, "mutate"),
                    description: format!(
                        "{about}: run a GraphQL mutation. Where its rules say so it waits for the person to approve exactly this document and these variables, and runs once."
                    ),
                    input_schema: schema,
                });
            }
            Kind::Command { argv } => tools.push(PluginTool {
                name: tool_name(&plugin.name, "run"),
                description: format!(
                    "{about}: run `{}` with your arguments, with no terminal and only the input you give. Where its rules say so it waits for the person's approval.",
                    argv.join(" ")
                ),
                input_schema: json!({"type": "object", "required": ["environment", "target", "args"], "properties": {
                    "environment": environment,
                    "target": {"type": "string", "description": "Where it runs, among the environment's targets"},
                    "args": {"type": "array", "items": {"type": "string"}},
                    "stdin": {"type": "string"}
                }}),
            }),
        }
    }
    tools
}

/// What a call runs. Not `Debug`: it holds a credential.
#[derive(Clone)]
enum Exec {
    Graphql { url: String, body: Value, header: Option<(String, String)> },
    Command { argv: Vec<String>, stdin: String },
}

/// A call worked out: what runs, whether it waits for the person, and what
/// its result may not carry.
struct Prepared {
    canonical: Canonical,
    ask: bool,
    data: Vec<DataClass>,
    exec: Exec,
}

fn text_field(input: &Value, name: &str) -> Result<String, String> {
    input.get(name).and_then(Value::as_str).map(str::to_owned).ok_or_else(|| format!("give `{name}`"))
}

/// Works out `tool`'s call with `input`, for an agent in `workspace`.
fn prepare(data_dir: &Path, plugins: &[Plugin], workspace: &str, tool: &str, input: &Value) -> Result<Prepared, String> {
    let (plugin, verb) = plugins
        .iter()
        .filter(|plugin| plugin.workspaces.iter().any(|on| on == workspace))
        .find_map(|plugin| {
            ["query", "mutate", "run"].into_iter().find(|verb| tool_name(&plugin.name, verb) == tool).map(|verb| (plugin, verb))
        })
        .ok_or_else(|| format!("no plugin offers {tool} in this workspace"))?;
    let environment_name = text_field(input, "environment")?;
    let environment = plugin
        .environments
        .get(&environment_name)
        .ok_or_else(|| format!("{} has no environment {environment_name}", plugin.name))?;
    let data = environment.data.clone();
    match (&plugin.kind, verb) {
        (Kind::Graphql, "query" | "mutate") => {
            let query = text_field(input, "query")?;
            let variables = input.get("variables").cloned().unwrap_or_else(|| json!({}));
            let classified = silt_access::classify(&query)?;
            if verb == "query" && classified.changes() {
                return Err(format!(
                    "it changes something; send it with {}, which asks the person first",
                    tool_name(&plugin.name, "mutate")
                ));
            }
            if verb == "mutate" && !classified.changes() {
                return Err(format!("it changes nothing; send it with {}", tool_name(&plugin.name, "query")));
            }
            for class in &data {
                if let Some(finding) = silt_access::graphql_fields(*class, &classified.fields) {
                    return Err(format!(
                        "refused before it was sent: {}, and {} in {environment_name} is kept from agents; no approval lifts it",
                        finding.what,
                        class.label()
                    ));
                }
            }
            let header = match &environment.credential {
                Some(name) => {
                    let credential = crate::credentials::resolve_named(data_dir, workspace, name)
                        .ok_or_else(|| format!("its credential {name} has no secret to add here"))?;
                    Some((credential.header, credential.value))
                }
                None => None,
            };
            let body = json!({"query": query, "variables": variables});
            let rule = if classified.changes() { environment.changes } else { environment.reads };
            Ok(Prepared {
                canonical: Canonical {
                    plugin: plugin.name.clone(),
                    tool: tool.to_owned(),
                    environment: environment_name,
                    target: environment.target.clone(),
                    input: body.clone(),
                },
                ask: rule == Rule::Ask,
                data,
                exec: Exec::Graphql { url: environment.target.clone(), body, header },
            })
        }
        (Kind::Command { argv }, "run") => {
            let target = text_field(input, "target")?;
            let allowed = environment.target.split(',').map(str::trim).any(|pattern| target_matches(pattern, &target));
            if !allowed || target.starts_with('-') {
                return Err(format!("{target} isn't among {environment_name}'s targets ({})", environment.target));
            }
            let args: Vec<String> = input
                .get("args")
                .and_then(Value::as_array)
                .ok_or("give `args`, a list")?
                .iter()
                .map(|arg| arg.as_str().map(str::to_owned).ok_or("each argument is a string"))
                .collect::<Result<_, _>>()?;
            let stdin = input.get("stdin").and_then(Value::as_str).unwrap_or_default().to_owned();
            let mut full: Vec<String> = argv.iter().map(|part| part.replace("{target}", &target)).collect();
            full.extend(args.iter().cloned());
            Ok(Prepared {
                canonical: Canonical {
                    plugin: plugin.name.clone(),
                    tool: tool.to_owned(),
                    environment: environment_name,
                    target,
                    input: json!({"argv": full, "stdin": stdin}),
                },
                ask: environment.changes == Rule::Ask,
                data,
                exec: Exec::Command { argv: full, stdin },
            })
        }
        _ => Err(format!("{} offers no {tool}", plugin.name)),
    }
}

/// How a call ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum Outcome {
    Done {
        output: String,
    },
    /// Refused here, by a rule or the person; nothing of it was shown.
    Refused {
        why: String,
    },
    /// It failed before anything was sent.
    Failed {
        error: String,
    },
    /// Sent, with no end seen: it may or may not have happened. Never sent
    /// again by itself.
    Unknown {
        why: String,
    },
    /// Waiting for the person.
    Pending {
        approval: u64,
    },
}

impl Outcome {
    /// What the agent reads.
    pub fn said(&self) -> String {
        match self {
            Self::Done { output } => output.clone(),
            Self::Refused { why } => format!("Refused: {why}"),
            Self::Failed { error } => format!("Failed before anything was sent: {error}"),
            Self::Unknown { why } => format!(
                "Outcome unknown: {why}. It may or may not have happened; Silt won't send it again by itself. Check before asking again."
            ),
            Self::Pending { approval } => format!(
                "Waiting for the person to approve it (approval {approval}). Call silt_plugin_wait with this approval to wait on; don't send it again."
            ),
        }
    }
}

/// Runs what a call prepared, and checks what comes back.
async fn execute(exec: Exec, data: Vec<DataClass>) -> Outcome {
    match exec {
        Exec::Graphql { url, body, header } => {
            let client = reqwest::Client::new();
            let mut request = client.post(&url).json(&body).timeout(Duration::from_secs(60));
            if let Some((name, value)) = header {
                request = request.header(name, value);
            }
            let response = match request.send().await {
                Ok(response) => response,
                Err(error) if error.is_connect() || error.is_builder() => return Outcome::Failed { error: error.to_string() },
                Err(error) => return Outcome::Unknown { why: format!("the connection ended without an answer ({error})") },
            };
            let status = response.status();
            let text = match response.text().await {
                Ok(text) => text,
                Err(error) => return Outcome::Unknown { why: format!("the answer broke off ({error})") },
            };
            let value: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));
            for class in &data {
                if let Some(finding) = silt_access::json(*class, &value) {
                    return refused_result(*class, &finding.what);
                }
            }
            let shown = serde_json::to_string_pretty(&value).unwrap_or_default();
            Outcome::Done { output: format!("HTTP {status}\n{shown}") }
        }
        Exec::Command { argv, stdin } => {
            let Some((program, args)) = argv.split_first() else {
                return Outcome::Failed { error: "no program".into() };
            };
            let mut command = tokio::process::Command::new(program);
            command
                .args(args)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);
            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(error) => return Outcome::Failed { error: format!("cannot start {program}: {error}") },
            };
            if let Some(mut input) = child.stdin.take() {
                let _ = tokio::io::AsyncWriteExt::write_all(&mut input, stdin.as_bytes()).await;
            }
            let output = match tokio::time::timeout(Duration::from_secs(120), child.wait_with_output()).await {
                Ok(Ok(output)) => output,
                Ok(Err(error)) => return Outcome::Unknown { why: format!("{program} ended without a status ({error})") },
                Err(_) => return Outcome::Unknown { why: format!("{program} ran past two minutes and was stopped") },
            };
            let mut bytes = output.stdout;
            bytes.extend_from_slice(&output.stderr);
            let Ok(text) = String::from_utf8(bytes) else {
                return Outcome::Refused { why: "its output isn't text, which a data rule can't check".into() };
            };
            for class in &data {
                if let Some(finding) = silt_access::text(*class, &text) {
                    return refused_result(*class, &finding.what);
                }
            }
            let code = output.status.code().map_or_else(|| "a signal".to_owned(), |code| code.to_string());
            Outcome::Done { output: format!("exit {code}\n{text}") }
        }
    }
}

fn refused_result(class: DataClass, what: &str) -> Outcome {
    Outcome::Refused {
        why: format!(
            "Silt ran it and kept the result: {what}, and {} is kept from agents here; no approval lifts it",
            class.label()
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    Pending,
    Approved,
    Rejected,
    Expired,
}

/// A call waiting for, or answered by, the person.
pub struct Approval {
    id: u64,
    thread: String,
    canonical: Canonical,
    hash: String,
    created_ms: i64,
    state: ApprovalState,
    outcome: Arc<watch::Sender<Option<Outcome>>>,
    exec: Option<(Exec, Vec<DataClass>)>,
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
}

#[derive(Default)]
pub struct Approvals {
    next: u64,
    all: BTreeMap<u64, Approval>,
}

impl Core {
    /// The workspace and thread a run's token was made for.
    async fn run_of(&self, token: &str) -> Result<(String, String), CoreError> {
        let hash = blake3::hash(token.as_bytes());
        let silt_store::RunToken { thread, workspace, .. } = self
            .store
            .run_token(hash.as_bytes())
            .await?
            .ok_or_else(|| CoreError::Invalid("this run's token isn't known here".into()))?;
        Ok((thread.to_string(), workspace))
    }

    pub(crate) async fn plugin_tools(&self, token: &str) -> Result<Vec<PluginTool>, CoreError> {
        let (_, workspace) = self.run_of(token).await?;
        Ok(tools(&load(&self.data_dir), &workspace))
    }

    /// Runs a plugin's tool for a run, or makes it an approval to wait on.
    pub(crate) async fn plugin_call(&mut self, token: &str, tool: &str, input: Value, reply: oneshot::Sender<Response>) {
        let (thread, workspace) = match self.run_of(token).await {
            Ok(run) => run,
            Err(error) => {
                let _ = reply.send(to_response::<()>(Err(error)));
                return;
            }
        };
        let prepared = match prepare(&self.data_dir, &load(&self.data_dir), &workspace, tool, &input) {
            Ok(prepared) => prepared,
            Err(why) => {
                let _ = reply.send(Response::ok(Outcome::Refused { why }));
                return;
            }
        };
        if !prepared.ask {
            tokio::spawn(async move {
                let outcome = execute(prepared.exec, prepared.data).await;
                let _ = reply.send(Response::ok(outcome));
            });
            return;
        }
        self.approvals.next += 1;
        let id = self.approvals.next;
        let (outcome, _) = watch::channel(None);
        let outcome = Arc::new(outcome);
        let hash = prepared.canonical.hash();
        self.approvals.all.insert(
            id,
            Approval {
                id,
                thread,
                canonical: prepared.canonical,
                hash,
                created_ms: now_ms(),
                state: ApprovalState::Pending,
                outcome: outcome.clone(),
                exec: Some((prepared.exec, prepared.data)),
            },
        );
        self.announce(BusEvent::Approvals);
        wait_on(outcome, id, reply);
    }

    /// Waits on an approval a run's earlier call made.
    pub(crate) async fn plugin_wait(&mut self, token: &str, id: u64, reply: oneshot::Sender<Response>) {
        let thread = match self.run_of(token).await {
            Ok((thread, _)) => thread,
            Err(error) => {
                let _ = reply.send(to_response::<()>(Err(error)));
                return;
            }
        };
        match self.approvals.all.get(&id).filter(|approval| approval.thread == thread) {
            Some(approval) => wait_on(approval.outcome.clone(), id, reply),
            None => {
                let _ = reply.send(Response::error(format!("this run has no approval {id}")));
            }
        }
    }

    /// The person's answer to approval `id`, naming the `hash` they saw.
    pub(crate) fn approval_answer(&mut self, id: u64, approve: bool, hash: &str) -> Result<(), CoreError> {
        let now = now_ms();
        let approval = self.approvals.all.get_mut(&id).ok_or_else(|| CoreError::Invalid(format!("no approval {id}")))?;
        if approval.state == ApprovalState::Pending && now - approval.created_ms > OPEN_MS {
            approval.state = ApprovalState::Expired;
            approval.outcome.send_replace(Some(Outcome::Refused { why: "its approval expired".into() }));
        }
        if approval.state != ApprovalState::Pending {
            return Err(CoreError::Invalid(format!("approval {id} isn't waiting any more")));
        }
        if approval.hash != hash {
            return Err(CoreError::Invalid("that isn't the request this approval holds; look again".into()));
        }
        if approve {
            approval.state = ApprovalState::Approved;
            if let Some((exec, data)) = approval.exec.take() {
                let outcome = approval.outcome.clone();
                tokio::spawn(async move {
                    outcome.send_replace(Some(execute(exec, data).await));
                });
            }
        } else {
            approval.state = ApprovalState::Rejected;
            approval.exec = None;
            approval.outcome.send_replace(Some(Outcome::Refused { why: "the person rejected it".into() }));
        }
        self.announce(BusEvent::Approvals);
        Ok(())
    }

    /// Approvals, newest first; a thread's alone when `thread` names it.
    pub(crate) fn approvals_view(&self, thread: Option<&str>) -> Vec<ApprovalView> {
        let now = now_ms();
        self.approvals
            .all
            .values()
            .rev()
            .filter(|approval| thread.is_none_or(|thread| approval.thread.starts_with(thread)))
            .map(|approval| {
                let expired = approval.state == ApprovalState::Pending && now - approval.created_ms > OPEN_MS;
                ApprovalView {
                    id: approval.id,
                    thread: approval.thread.clone(),
                    plugin: approval.canonical.plugin.clone(),
                    tool: approval.canonical.tool.clone(),
                    environment: approval.canonical.environment.clone(),
                    target: approval.canonical.target.clone(),
                    shown: shown_input(&approval.canonical.input),
                    hash: approval.hash.clone(),
                    created_ms: approval.created_ms,
                    state: if expired { ApprovalState::Expired } else { approval.state },
                    outcome: approval.outcome.borrow().clone(),
                }
            })
            .collect()
    }
}

/// A request's input as the person reads it: a GraphQL document and its
/// variables, or a command line and its input.
fn shown_input(input: &Value) -> String {
    let text = if let Some(query) = input.get("query").and_then(Value::as_str) {
        let variables = input.get("variables").map(|variables| serde_json::to_string_pretty(variables).unwrap_or_default());
        format!("{query}\n\nvariables: {}", variables.unwrap_or_default())
    } else if let Some(argv) = input.get("argv").and_then(Value::as_array) {
        let line: Vec<String> = argv.iter().filter_map(Value::as_str).map(quoted).collect();
        let stdin = input.get("stdin").and_then(Value::as_str).unwrap_or_default();
        if stdin.is_empty() { line.join(" ") } else { format!("{}\n\nstdin:\n{stdin}", line.join(" ")) }
    } else {
        serde_json::to_string_pretty(input).unwrap_or_default()
    };
    silt_access::shown(&text)
}

/// An argument as a shell would read it back, quoted only when it needs to be.
fn quoted(arg: &str) -> String {
    let plain = !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:,@%+".contains(c));
    if plain { arg.to_owned() } else { format!("'{}'", arg.replace('\'', "'\\''")) }
}

/// Whether `target` is one `pattern` names, a `*` standing for part of
/// one of its labels.
fn target_matches(pattern: &str, target: &str) -> bool {
    let (pattern, target) = (pattern.to_ascii_lowercase(), target.to_ascii_lowercase());
    let labels: Vec<&str> = pattern.split('.').collect();
    let parts: Vec<&str> = target.split('.').collect();
    labels.len() == parts.len() && labels.iter().zip(&parts).all(|(label, part)| label_matches(label, part))
}

fn label_matches(label: &str, part: &str) -> bool {
    let valid = !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    let Some((first, rest)) = label.split_once('*') else {
        return valid && label == part;
    };
    if !valid || !part.starts_with(first) {
        return false;
    }
    let tail = &part[first.len()..];
    if !rest.contains('*') {
        return tail.ends_with(rest);
    }
    (0..=tail.len()).any(|skip| tail.is_char_boundary(skip) && label_matches(rest, &tail[skip..]))
}

/// Answers `reply` with the approval's outcome once there is one, or after
/// [`WAIT`] with its being pending.
fn wait_on(outcome: Arc<watch::Sender<Option<Outcome>>>, id: u64, reply: oneshot::Sender<Response>) {
    let mut seen = outcome.subscribe();
    tokio::spawn(async move {
        let done = tokio::time::timeout(WAIT, seen.wait_for(Option::is_some)).await;
        let answer = match done {
            Ok(Ok(outcome)) => outcome.clone().unwrap_or(Outcome::Pending { approval: id }),
            _ => Outcome::Pending { approval: id },
        };
        let _ = reply.send(Response::ok(answer));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fleet() -> Plugin {
        Plugin {
            name: "fleet-api".into(),
            description: String::new(),
            kind: Kind::Graphql,
            workspaces: vec!["work".into()],
            environments: BTreeMap::from([(
                "production".to_owned(),
                Environment {
                    target: "https://api.fleet.example/graphql".into(),
                    reads: Rule::Allow,
                    changes: Rule::Ask,
                    data: vec![DataClass::Location],
                    credential: None,
                },
            )]),
        }
    }

    fn ssh() -> Plugin {
        Plugin {
            name: "test fleet".into(),
            description: String::new(),
            kind: Kind::Command { argv: vec!["ssh".into(), "-T".into(), "{target}".into(), "--".into()] },
            workspaces: vec!["work".into()],
            environments: BTreeMap::from([(
                "fleet".to_owned(),
                Environment {
                    target: "car-*.fleet.vpn".into(),
                    reads: Rule::Allow,
                    changes: Rule::Allow,
                    data: Vec::new(),
                    credential: None,
                },
            )]),
        }
    }

    #[test]
    fn a_workspace_offers_only_its_plugins() {
        let names: Vec<String> = tools(&[fleet(), ssh()], "work").into_iter().map(|tool| tool.name).collect();
        assert_eq!(names, ["fleet_api_query", "fleet_api_mutate", "test_fleet_run"]);
        assert!(tools(&[fleet()], "personal").is_empty());
    }

    #[test]
    fn reads_go_changes_ask_and_location_never_leaves() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = [fleet()];
        let call = |tool: &str, query: &str| {
            prepare(dir.path(), &plugins, "work", tool, &json!({"environment": "production", "query": query}))
        };
        let read = call("fleet_api_query", "{ vehicle(id: 1) { id otaEnabled } }").unwrap();
        assert!(!read.ask);
        let change = call("fleet_api_mutate", "mutation { setOta(car: 1, on: true) { id } }").unwrap();
        assert!(change.ask, "a change asks");
        assert!(
            call("fleet_api_query", "mutation { setOta(car: 1, on: true) { id } }").err().unwrap().contains("fleet_api_mutate")
        );
        let refused = call("fleet_api_query", "{ vehicle(id: 1) { id lastPosition { lat lon } } }").err().unwrap();
        assert!(refused.contains("refused before it was sent") && refused.contains("location"), "{refused}");
        assert!(prepare(dir.path(), &plugins, "personal", "fleet_api_query", &json!({})).is_err(), "not on in personal");
    }

    #[test]
    fn a_command_runs_only_at_its_targets() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = [ssh()];
        let run = |target: &str| {
            prepare(
                dir.path(),
                &plugins,
                "work",
                "test_fleet_run",
                &json!({"environment": "fleet", "target": target, "args": ["uptime"]}),
            )
        };
        let ok = run("car-0142.fleet.vpn").unwrap();
        assert_eq!(ok.canonical.input["argv"], json!(["ssh", "-T", "car-0142.fleet.vpn", "--", "uptime"]));
        assert!(!ok.ask);
        assert!(run("prod-db.acme.dev").is_err());
        assert!(run("-oProxyCommand=sh").is_err(), "no option in a target's place");
        assert!(run("car-1.evil.vpn.fleet.vpn").is_err() && run("car-1.fleet.vpn.evil").is_err());
        assert!(target_matches("car-*.fleet.vpn", "CAR-7.fleet.vpn") && !target_matches("car-*.fleet.vpn", "car-a b.fleet.vpn"));
    }

    #[tokio::test]
    async fn a_commands_output_passes_the_data_rules() {
        let outcome = execute(
            Exec::Command { argv: vec!["echo".into(), "fix at 59.329323, 18.068581".into()], stdin: String::new() },
            vec![DataClass::Location],
        )
        .await;
        assert!(matches!(&outcome, Outcome::Refused { why } if why.contains("kept the result")), "{outcome:?}");
        let outcome = execute(Exec::Command { argv: vec!["cat".into()], stdin: "hello".into() }, vec![DataClass::Location]).await;
        assert_eq!(outcome, Outcome::Done { output: "exit 0\nhello".into() });
    }

    #[tokio::test]
    async fn a_change_runs_once_after_the_person_approves_what_they_saw() {
        use silt_store::{Store, ThreadRecord};
        use silt_types::{ActorId, FamilyId, ThreadId};

        let data = tempfile::tempdir().expect("temp dir");
        let store = Store::open(data.path()).await.expect("opens");
        let thread = ThreadId(7);
        let record = ThreadRecord {
            id: thread,
            family: FamilyId(9),
            repo_root: std::path::PathBuf::new(),
            repo: String::new(),
            created_ms: 1,
            owner: String::new(),
        };
        store.create_thread(&record).await.expect("thread");
        store.create_thread(&ThreadRecord { id: ThreadId(8), family: FamilyId(10), ..record }).await.expect("thread");
        store.add_run_token(blake3::hash(b"work-run").as_bytes(), thread, "work", None, 1).await.expect("token");
        store.add_run_token(blake3::hash(b"home-run").as_bytes(), ThreadId(8), "personal", None, 1).await.expect("token");
        let mut plugin = ssh();
        plugin.kind = Kind::Command { argv: vec!["echo".into(), "ran on {target}".into()] };
        plugin.environments.get_mut("fleet").expect("fleet").changes = Rule::Ask;
        put(data.path(), plugin).expect("saves");
        let (events, _inbox) = tokio::sync::mpsc::unbounded_channel();
        let mut core = Core::load(store, ActorId(1), data.path(), events).await.expect("loads");
        let input = json!({"environment": "fleet", "target": "car-1.fleet.vpn", "args": ["now"]});

        assert!(core.plugin_tools("home-run").await.expect("tools").is_empty(), "off in personal");
        let (reply, refused) = oneshot::channel();
        core.plugin_call("home-run", "test_fleet_run", input.clone(), reply).await;
        assert!(matches!(outcome(refused.await.expect("answers")), Outcome::Refused { .. }));

        let (reply, waiting) = oneshot::channel();
        core.plugin_call("work-run", "test_fleet_run", input, reply).await;
        let pending = core.approvals_view(None);
        assert_eq!(pending.len(), 1);
        let approval = &pending[0];
        assert_eq!(approval.state, ApprovalState::Pending);
        assert_eq!(approval.shown, "echo 'ran on car-1.fleet.vpn' now");
        assert!(core.approval_answer(approval.id, true, "not-what-was-shown").is_err());
        core.approval_answer(approval.id, true, &approval.hash).expect("approves");
        assert!(core.approval_answer(approval.id, true, &approval.hash).is_err(), "only once");
        assert_eq!(
            outcome(waiting.await.expect("answers")),
            Outcome::Done { output: "exit 0\nran on car-1.fleet.vpn now\n".into() }
        );
        let (reply, again) = oneshot::channel();
        core.plugin_wait("home-run", approval.id, reply).await;
        assert!(matches!(again.await.expect("answers"), Response::Error { .. }), "another run's approval");
    }

    fn outcome(response: Response) -> Outcome {
        match response {
            Response::Ok { data } => serde_json::from_value(data).expect("an outcome"),
            Response::Error { message } => panic!("{message}"),
        }
    }

    #[test]
    fn what_the_person_sees_hides_nothing() {
        let shown = shown_input(&json!({"argv": ["sh", "-c", "echo ok\u{202E}"], "stdin": ""}));
        assert!(shown.contains("U+202E"), "{shown}");
    }
}
