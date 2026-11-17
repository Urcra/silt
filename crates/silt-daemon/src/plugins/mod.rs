//! Plugins (ADR 0093): what a sandboxed agent reaches production and other
//! systems through. Each machine keeps its plugins in `plugins.json`, each
//! turned on in some of its workspaces; their calls run here, outside every
//! sandbox, with credentials the agent never holds. A call its rules say
//! asks waits as an approval, bound to the hash of Silt's own copy of the
//! request: only that copy runs, once, and only after the person says so.
//! What comes back passes the environment's data rules first.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use silt_access::{Canonical, DataClass, Schema};

use crate::CoreError;

mod calls;
mod mcp;

pub use calls::{AccessView, ApprovalState, ApprovalView, Approvals, CallView, GrantScope, GrantView, NetworkRefusal, RelayView};

const FILE: &str = "plugins.json";

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
    /// An MCP server run here, `argv` with `{target}` in place of the
    /// environment's target: its tools are offered as the plugin's, a tool
    /// it marks read-only a read and any other a change.
    Mcp { argv: Vec<String> },
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
    /// A credential of `silt credential` added to its requests, or for a
    /// command or MCP server, given to it in `variable`.
    #[serde(default)]
    pub credential: Option<String>,
    /// The variable a command or MCP server reads the credential's secret
    /// from, such as `FLEET_TOKEN`.
    #[serde(default)]
    pub variable: Option<String>,
    /// Each change is approved on its own: no grant covers it.
    #[serde(default)]
    pub once: bool,
    /// A GraphQL environment its agents may also query straight through
    /// the gateway, which reads each request: reads go, changes only where
    /// `changes` allows them, and data rules hold both ways.
    #[serde(default)]
    pub direct: bool,
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
    if let Some(name) = plugin
        .environments
        .keys()
        .find(|name| name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    {
        return Err(CoreError::Invalid(format!("an environment's name is letters, digits, `-` and `_`, not {name:?}")));
    }
    if let Kind::Command { argv } | Kind::Mcp { argv } = &plugin.kind
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

/// Where a GraphQL environment's schema is kept, as `silt plugin schema`
/// fetched it.
pub fn schema_path(data_dir: &Path, plugin: &str, environment: &str) -> std::path::PathBuf {
    data_dir.join("plugins").join(plugin).join(format!("{environment}.schema.json"))
}

/// The schema kept for `plugin`'s `environment`.
pub fn schema_of(data_dir: &Path, plugin: &str, environment: &str) -> Option<Schema> {
    let bytes = std::fs::read(schema_path(data_dir, plugin, environment)).ok()?;
    Schema::from_introspection(&serde_json::from_slice(&bytes).ok()?).ok()
}

/// Asks `plugin`'s `environment` for its schema, with the credential
/// `workspace` has for it, and keeps it for data rules to walk.
pub async fn fetch_schema(data_dir: &Path, plugin: &Plugin, environment: &str, workspace: &str) -> Result<(), String> {
    let found =
        plugin.environments.get(environment).ok_or_else(|| format!("{} has no environment {environment}", plugin.name))?;
    if plugin.kind != Kind::Graphql {
        return Err(format!("{} isn't a GraphQL plugin", plugin.name));
    }
    let mut request = reqwest::Client::new()
        .post(&found.target)
        .json(&json!({"query": silt_access::INTROSPECTION}))
        .timeout(Duration::from_secs(60));
    if let Some(name) = &found.credential {
        let credential = crate::credentials::resolve_named(data_dir, workspace, name)
            .ok_or_else(|| format!("its credential {name} has no secret in {workspace}"))?;
        request = request.header(credential.header, credential.value);
    }
    let answer: Value =
        request.send().await.map_err(|error| error.to_string())?.json().await.map_err(|error| error.to_string())?;
    Schema::from_introspection(&answer)?;
    let path = schema_path(data_dir, &plugin.name, environment);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    }
    std::fs::write(&path, serde_json::to_vec(&answer).unwrap_or_default()).map_err(|error| error.to_string())
}

/// The GraphQL environments `workspace`'s agents query straight through
/// the gateway.
pub fn direct_graphql(data_dir: &Path, workspace: &str) -> Vec<silt_gateway::GraphqlRule> {
    let mut rules = Vec::new();
    for plugin in load(data_dir)
        .into_iter()
        .filter(|plugin| plugin.kind == Kind::Graphql && plugin.workspaces.iter().any(|on| on == workspace))
    {
        for (name, environment) in plugin.environments.iter().filter(|(_, environment)| environment.direct) {
            let Ok(url) = reqwest::Url::parse(&environment.target) else { continue };
            let Some(host) = url.host_str() else { continue };
            rules.push(silt_gateway::GraphqlRule {
                host: host.to_owned(),
                path: url.path().to_owned(),
                changes: environment.changes == Rule::Allow,
                data: environment.data.clone(),
                schema: schema_of(data_dir, &plugin.name, name).map(std::sync::Arc::new),
                plugin: plugin.name.clone(),
            });
        }
    }
    rules
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
pub fn tools(plugins: &[Plugin], workspace: &str, listed: &BTreeMap<String, Vec<Value>>) -> Vec<PluginTool> {
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
            Kind::Mcp { .. } => {
                for tool in listed.get(&plugin.name).into_iter().flatten() {
                    let Some(name) = tool.get("name").and_then(Value::as_str) else { continue };
                    let mut schema = tool.get("inputSchema").cloned().unwrap_or_else(|| json!({"type": "object"}));
                    if let Some(object) = schema.as_object_mut() {
                        let properties = object.entry("properties").or_insert_with(|| json!({}));
                        if let Some(properties) = properties.as_object_mut() {
                            properties.insert("environment".into(), environment.clone());
                        }
                        let required = object.entry("required").or_insert_with(|| json!([]));
                        if let Some(required) = required.as_array_mut() {
                            required.push(json!("environment"));
                        }
                    }
                    let what = tool.get("description").and_then(Value::as_str).unwrap_or(name);
                    let asks = if mcp::reads_only(tool) { "" } else { " Where its rules say so it waits for the person's approval." };
                    tools.push(PluginTool { name: tool_name(&plugin.name, name), description: format!("{about}: {what}.{asks}"), input_schema: schema });
                }
            }
        }
    }
    tools
}

/// The tools an MCP plugin's server offers, listed from its first
/// environment's; none for another kind, or a server that can't say.
async fn list_mcp(data_dir: &Path, plugin: &Plugin, workspace: &str) -> Option<Vec<Value>> {
    let Kind::Mcp { argv } = &plugin.kind else { return None };
    let (_, environment) = plugin.environments.iter().next()?;
    let argv: Vec<String> = argv.iter().map(|part| part.replace("{target}", &environment.target)).collect();
    let env = program_env(data_dir, workspace, environment).ok()?;
    let listing = async {
        let mut server = mcp::Server::start(&argv, &env).await.ok()?;
        let tools = server.tools().await.ok();
        server.stop().await;
        tools
    };
    match tokio::time::timeout(Duration::from_secs(20), listing).await {
        Ok(Some(tools)) => Some(tools),
        _ => {
            tracing::warn!(plugin = %plugin.name, "the plugin's MCP server didn't list its tools");
            None
        }
    }
}

/// The MCP plugin and its server's own tool `tool` is, with whether the
/// server marks it read-only; a tool not listed yet counts as a change.
fn mcp_tool<'a>(
    plugins: &'a [Plugin],
    workspace: &str,
    tool: &str,
    listed: &BTreeMap<String, Vec<Value>>,
) -> Option<(&'a Plugin, String, bool)> {
    plugins
        .iter()
        .filter(|plugin| matches!(plugin.kind, Kind::Mcp { .. }) && plugin.workspaces.iter().any(|on| on == workspace))
        .filter(|plugin| tool.starts_with(&tool_name(&plugin.name, "")))
        .max_by_key(|plugin| plugin.name.len())
        .map(|plugin| {
            let known = listed.get(&plugin.name).into_iter().flatten().find(|offered| {
                offered.get("name").and_then(Value::as_str).is_some_and(|name| tool_name(&plugin.name, name) == tool)
            });
            match known {
                Some(offered) => (plugin, offered["name"].as_str().unwrap_or_default().to_owned(), mcp::reads_only(offered)),
                None => (plugin, tool[tool_name(&plugin.name, "").len()..].to_owned(), false),
            }
        })
}

/// What a call runs. Not `Debug`: it holds a credential.
#[derive(Clone)]
enum Exec {
    Graphql { url: String, body: Value, header: Option<(String, String)> },
    Command { argv: Vec<String>, stdin: String, env: Vec<(String, String)> },
    Mcp { argv: Vec<String>, env: Vec<(String, String)>, tool: String, arguments: Value },
}

/// A call worked out: Silt's copy of what it sends, and whether it waits
/// for the person.
struct Prepared {
    canonical: Canonical,
    ask: bool,
    /// Whether it may change something, rather than only read.
    change: bool,
    /// No grant may cover it.
    once: bool,
}

fn text_field(input: &Value, name: &str) -> Result<String, String> {
    input.get(name).and_then(Value::as_str).map(str::to_owned).ok_or_else(|| format!("give `{name}`"))
}

/// Works out `tool`'s call with `input`, for an agent in `workspace`.
fn prepare(
    plugins: &[Plugin],
    schemas: &dyn Fn(&str, &str) -> Option<Schema>,
    listed: &BTreeMap<String, Vec<Value>>,
    workspace: &str,
    tool: &str,
    input: &Value,
) -> Result<Prepared, String> {
    let fixed = plugins
        .iter()
        .filter(|plugin| !matches!(plugin.kind, Kind::Mcp { .. }) && plugin.workspaces.iter().any(|on| on == workspace))
        .find_map(|plugin| {
            ["query", "mutate", "run"].into_iter().find(|verb| tool_name(&plugin.name, verb) == tool).map(|verb| (plugin, verb))
        });
    let Some((plugin, verb)) = fixed else {
        let (plugin, name, reads) =
            mcp_tool(plugins, workspace, tool, listed).ok_or_else(|| format!("no plugin offers {tool} in this workspace"))?;
        let environment_name = text_field(input, "environment")?;
        let environment = plugin
            .environments
            .get(&environment_name)
            .ok_or_else(|| format!("{} has no environment {environment_name}", plugin.name))?;
        let mut arguments = input.clone();
        if let Some(object) = arguments.as_object_mut() {
            object.remove("environment");
        }
        let rule = if reads { environment.reads } else { environment.changes };
        return Ok(Prepared {
            canonical: Canonical {
                plugin: plugin.name.clone(),
                tool: tool.to_owned(),
                environment: environment_name,
                target: environment.target.clone(),
                input: json!({"tool": name, "arguments": arguments}),
            },
            ask: rule == Rule::Ask,
            change: !reads,
            once: environment.once,
        });
    };
    let environment_name = text_field(input, "environment")?;
    let environment = plugin
        .environments
        .get(&environment_name)
        .ok_or_else(|| format!("{} has no environment {environment_name}", plugin.name))?;
    let data = &environment.data;
    let schema = schemas(&plugin.name, &environment_name);
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
            for class in data {
                let found = match silt_access::graphql_fields(*class, &classified.fields) {
                    Some(finding) => Some(finding),
                    None => schema.as_ref().map(|schema| schema.finding(*class, &query)).transpose()?.flatten(),
                };
                if let Some(finding) = found {
                    return Err(format!(
                        "refused before it was sent: {}, and {} in {environment_name} is kept from agents; no approval lifts it",
                        finding.what,
                        class.label()
                    ));
                }
            }
            let body = json!({"query": query, "variables": variables});
            let change = classified.changes();
            let rule = if change { environment.changes } else { environment.reads };
            Ok(Prepared {
                canonical: Canonical {
                    plugin: plugin.name.clone(),
                    tool: tool.to_owned(),
                    environment: environment_name,
                    target: environment.target.clone(),
                    input: body,
                },
                ask: rule == Rule::Ask,
                change,
                once: environment.once,
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
                change: true,
                once: environment.once,
            })
        }
        _ => Err(format!("{} offers no {tool}", plugin.name)),
    }
}

/// What runs for `canonical`, worked out as it runs: its credential then,
/// and the environment's data rules then. A plugin changed since so that
/// it would send something else is refused rather than followed.
fn exec_of(
    data_dir: &Path,
    plugins: &[Plugin],
    workspace: &str,
    canonical: &Canonical,
) -> Result<(Exec, Vec<DataClass>), String> {
    let plugin = plugins
        .iter()
        .find(|plugin| plugin.name == canonical.plugin && plugin.workspaces.iter().any(|on| on == workspace))
        .ok_or_else(|| format!("{} isn't on in this workspace any more", canonical.plugin))?;
    let environment = plugin
        .environments
        .get(&canonical.environment)
        .ok_or_else(|| format!("{} has no environment {} any more", plugin.name, canonical.environment))?;
    let changed = || format!("{}'s {} changed since this was asked; ask again", plugin.name, canonical.environment);
    let exec = match &plugin.kind {
        Kind::Graphql => {
            if environment.target != canonical.target {
                return Err(changed());
            }
            let header = match &environment.credential {
                Some(name) => {
                    let credential = crate::credentials::resolve_named(data_dir, workspace, name)
                        .ok_or_else(|| format!("its credential {name} has no secret to add here"))?;
                    Some((credential.header, credential.value))
                }
                None => None,
            };
            Exec::Graphql { url: environment.target.clone(), body: canonical.input.clone(), header }
        }
        Kind::Mcp { argv: template } => {
            if environment.target != canonical.target {
                return Err(changed());
            }
            let argv = template.iter().map(|part| part.replace("{target}", &canonical.target)).collect();
            let tool = canonical.input.get("tool").and_then(Value::as_str).unwrap_or_default().to_owned();
            let arguments = canonical.input.get("arguments").cloned().unwrap_or_else(|| json!({}));
            Exec::Mcp { argv, env: program_env(data_dir, workspace, environment)?, tool, arguments }
        }
        Kind::Command { argv: template } => {
            let argv: Vec<String> = canonical
                .input
                .get("argv")
                .and_then(Value::as_array)
                .map(|argv| argv.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                .unwrap_or_default();
            let expected: Vec<String> = template.iter().map(|part| part.replace("{target}", &canonical.target)).collect();
            if !argv.starts_with(&expected) {
                return Err(changed());
            }
            let stdin = canonical.input.get("stdin").and_then(Value::as_str).unwrap_or_default().to_owned();
            Exec::Command { argv, stdin, env: program_env(data_dir, workspace, environment)? }
        }
    };
    Ok((exec, environment.data.clone()))
}

/// What a command or MCP server gets of the environment's credential.
fn program_env(data_dir: &Path, workspace: &str, environment: &Environment) -> Result<Vec<(String, String)>, String> {
    match (&environment.credential, &environment.variable) {
        (Some(name), Some(variable)) => {
            let secret = crate::credentials::secret_named(data_dir, workspace, name)
                .ok_or_else(|| format!("its credential {name} has no secret to give it here"))?;
            Ok(vec![(variable.clone(), secret)])
        }
        _ => Ok(Vec::new()),
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
        Exec::Mcp { argv, env, tool, arguments } => {
            let mut server = match mcp::Server::start(&argv, &env).await {
                Ok(server) => server,
                Err(error) => return Outcome::Failed { error },
            };
            let answer = server.call(&tool, &arguments).await;
            server.stop().await;
            let result = match answer {
                Ok(result) => result,
                Err(why) => return Outcome::Unknown { why: format!("the plugin's server gave no answer ({why})") },
            };
            let (text, failed) = match mcp::text_of(&result) {
                Ok(text) => text,
                Err(why) => return Outcome::Refused { why },
            };
            for class in &data {
                if let Some(finding) = silt_access::text(*class, &text) {
                    return refused_result(*class, &finding.what);
                }
            }
            Outcome::Done { output: if failed { format!("the tool failed:\n{text}") } else { text } }
        }
        Exec::Command { argv, stdin, env } => {
            let Some((program, args)) = argv.split_first() else {
                return Outcome::Failed { error: "no program".into() };
            };
            let mut command = tokio::process::Command::new(program);
            command
                .args(args)
                .envs(env)
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
                    once: false,
                    direct: false,
                    variable: None,
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
                    once: false,
                    direct: false,
                    variable: None,
                },
            )]),
        }
    }

    #[test]
    fn a_workspace_offers_only_its_plugins() {
        let names: Vec<String> = tools(&[fleet(), ssh()], "work", &BTreeMap::new()).into_iter().map(|tool| tool.name).collect();
        assert_eq!(names, ["fleet_api_query", "fleet_api_mutate", "test_fleet_run"]);
        assert!(tools(&[fleet()], "personal", &BTreeMap::new()).is_empty());
    }

    #[test]
    fn reads_go_changes_ask_and_location_never_leaves() {
        let plugins = [fleet()];
        let call = |tool: &str, query: &str| {
            prepare(&plugins, &|_, _| None, &BTreeMap::new(), "work", tool, &json!({"environment": "production", "query": query}))
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
        assert!(
            prepare(&plugins, &|_, _| None, &BTreeMap::new(), "personal", "fleet_api_query", &json!({})).is_err(),
            "not on in personal"
        );

        // With its schema kept, a field whose type carries location is refused whatever it's called.
        let field = |name: &str, kind: &str| json!({"name": name, "type": {"kind": "OBJECT", "name": kind}});
        let schema = Schema::from_introspection(&json!({"__schema": {
            "queryType": {"name": "Query"},
            "types": [
                {"name": "Query", "kind": "OBJECT", "fields": [field("vehicle", "Vehicle")]},
                {"name": "Vehicle", "kind": "OBJECT", "fields": [field("id", "ID"), field("spot", "Fix")]},
                {"name": "Fix", "kind": "OBJECT", "fields": [field("lat", "Float"), field("lng", "Float")]}
            ]
        }}))
        .unwrap();
        let schemas = |_: &str, _: &str| Some(schema.clone());
        let input = json!({"environment": "production", "query": "{ vehicle { id spot { __typename } } }"});
        let refused = prepare(&plugins, &schemas, &BTreeMap::new(), "work", "fleet_api_query", &input).err().unwrap();
        assert!(refused.contains("vehicle.spot, a Fix"), "{refused}");
    }

    #[test]
    fn a_command_runs_only_at_its_targets() {
        let plugins = [ssh()];
        let run = |target: &str| {
            prepare(
                &plugins,
                &|_, _| None,
                &BTreeMap::new(),
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

    #[test]
    fn what_runs_is_worked_out_again_from_silts_copy_and_refused_if_the_plugin_changed() {
        let dir = tempfile::tempdir().unwrap();
        let mut plugins = [ssh()];
        let input = json!({"environment": "fleet", "target": "car-1.fleet.vpn", "args": ["uptime"]});
        let prepared = prepare(&plugins, &|_, _| None, &BTreeMap::new(), "work", "test_fleet_run", &input).unwrap();
        assert!(exec_of(dir.path(), &plugins, "work", &prepared.canonical).is_ok());
        assert!(exec_of(dir.path(), &plugins, "personal", &prepared.canonical).is_err(), "off there");
        plugins[0].kind = Kind::Command { argv: vec!["ssh".into(), "{target}".into(), "sudo".into()] };
        assert!(exec_of(dir.path(), &plugins, "work", &prepared.canonical).err().unwrap().contains("changed since"));
    }

    #[tokio::test]
    async fn a_commands_output_passes_the_data_rules() {
        let outcome = execute(
            Exec::Command {
                argv: vec!["echo".into(), "fix at 59.329323, 18.068581".into()],
                stdin: String::new(),
                env: Vec::new(),
            },
            vec![DataClass::Location],
        )
        .await;
        assert!(matches!(&outcome, Outcome::Refused { why } if why.contains("kept the result")), "{outcome:?}");
        let outcome = execute(
            Exec::Command { argv: vec!["cat".into()], stdin: "hello".into(), env: Vec::new() },
            vec![DataClass::Location],
        )
        .await;
        assert_eq!(outcome, Outcome::Done { output: "exit 0\nhello".into() });
    }

    /// An MCP server in a few lines of shell: one tool that reads, one that
    /// doesn't, and the secret it got in `FLEET_TOKEN` in its answers.
    #[cfg(unix)]
    const FAKE_MCP: &str = r#"while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"initialize"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fake","version":"1"}}}\n' "$id";;
    *'"tools/list"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"get_car","description":"Reads a car","inputSchema":{"type":"object","properties":{"car":{"type":"string"}}},"annotations":{"readOnlyHint":true}},{"name":"set_ota","inputSchema":{"type":"object"}}]}}\n' "$id";;
    *'"tools/call"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"car ok with %s"}]}}\n' "$id" "$FLEET_TOKEN";;
  esac
done"#;

    #[cfg(unix)]
    #[tokio::test]
    async fn an_mcp_servers_tools_are_the_plugins_and_what_doesnt_only_read_asks() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-mcp.sh");
        std::fs::write(&script, FAKE_MCP).unwrap();
        crate::credentials::save(
            dir.path(),
            &[crate::credentials::StoredCredential {
                name: "fleet".into(),
                workspace: "work".into(),
                host: "fleet.example".into(),
                path: "/".into(),
                header: "authorization".into(),
                scheme: "Bearer".into(),
                source: crate::credentials::Source::Command { argv: vec!["echo".into(), "s3cret".into()] },
                placeholder: None,
            }],
        )
        .unwrap();
        let plugin = Plugin {
            name: "fleet".into(),
            description: String::new(),
            kind: Kind::Mcp { argv: vec!["sh".into(), script.to_string_lossy().into_owned(), "{target}".into()] },
            workspaces: vec!["work".into()],
            environments: BTreeMap::from([(
                "prod".to_owned(),
                Environment {
                    target: "https://fleet.example".into(),
                    reads: Rule::Allow,
                    changes: Rule::Ask,
                    data: vec![DataClass::Location],
                    credential: Some("fleet".into()),
                    once: false,
                    direct: false,
                    variable: Some("FLEET_TOKEN".into()),
                },
            )]),
        };
        let offered = list_mcp(dir.path(), &plugin, "work").await.expect("lists its tools");
        let listed = BTreeMap::from([("fleet".to_owned(), offered)]);
        let plugins = [plugin];
        let names: Vec<String> = tools(&plugins, "work", &listed).into_iter().map(|tool| tool.name).collect();
        assert_eq!(names, ["fleet_get_car", "fleet_set_ota"]);
        let read =
            prepare(&plugins, &|_, _| None, &listed, "work", "fleet_get_car", &json!({"environment": "prod", "car": "c1"}))
                .unwrap();
        assert!(!read.ask && !read.change);
        assert_eq!(read.canonical.input, json!({"tool": "get_car", "arguments": {"car": "c1"}}));
        let change = prepare(&plugins, &|_, _| None, &listed, "work", "fleet_set_ota", &json!({"environment": "prod"})).unwrap();
        assert!(change.ask && change.change, "a tool not marked read-only is a change");
        let unlisted =
            prepare(&plugins, &|_, _| None, &BTreeMap::new(), "work", "fleet_get_car", &json!({"environment": "prod"})).unwrap();
        assert!(unlisted.change, "a tool not listed yet counts as a change");
        let (exec, data) = exec_of(dir.path(), &plugins, "work", &read.canonical).unwrap();
        assert_eq!(execute(exec, data).await, Outcome::Done { output: "car ok with s3cret\n".into() });
    }

    #[test]
    fn what_the_person_sees_hides_nothing() {
        let shown = shown_input(&json!({"argv": ["sh", "-c", "echo ok\u{202E}"], "stdin": ""}));
        assert!(shown.contains("U+202E"), "{shown}");
    }
}
