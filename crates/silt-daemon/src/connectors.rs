//! Page connectors (ADR 0080): MCP servers the person lists for pages, and
//! the daemon as their client. A page reaches only the tools the person
//! ticked, through the viewer's own daemon, and every action is confirmed
//! by the viewer in the page holding it.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rmcp::RoleClient;
use rmcp::ServiceExt as _;
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::core::{Core, CoreError};
use crate::design::{read_capped, write_atomically};

/// Where this machine keeps its page connectors; never synced.
const FILE: &str = "page-connectors.json";
/// How long a server stays up unused.
const IDLE: Duration = Duration::from_secs(5 * 60);
/// The most a tool's result may be, as JSON.
pub(crate) const RESULT_BYTES: usize = 1 << 20;

/// An MCP server the person set up for pages.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PageConnector {
    pub name: String,
    /// A stdio server's program, with `args`.
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    /// A Streamable HTTP server's address.
    #[serde(default)]
    pub url: Option<String>,
    /// Headers sent to an HTTP server, each from the environment variable
    /// it names, so no secret is kept in the file.
    #[serde(default)]
    pub header_env: BTreeMap<String, String>,
    /// The environment variables a stdio server gets beyond the few every
    /// program needs ([`BASE_ENV`]), by name; nothing else of the daemon's
    /// environment reaches it.
    #[serde(default)]
    pub pass_env: Vec<String>,
    /// The tools pages may call, by name; every one is off until ticked.
    #[serde(default)]
    pub tools: BTreeMap<String, bool>,
}

/// A connector's tool, as Settings and pages see it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ConnectorTool {
    pub name: String,
    pub description: Option<String>,
    /// The server marks it as only reading; anything else is an action the
    /// viewer confirms each time.
    pub read_only: bool,
    #[cfg_attr(test, ts(type = "unknown"))]
    pub input_schema: serde_json::Value,
}

pub(crate) fn connectors(data_dir: &Path) -> Vec<PageConnector> {
    read_capped(&data_dir.join(FILE), 1 << 20).and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default()
}

pub(crate) fn set_connectors(data_dir: &Path, list: &[PageConnector]) -> Result<(), CoreError> {
    let mut names = std::collections::HashSet::new();
    for connector in list {
        let name_ok = (1..=64).contains(&connector.name.len())
            && connector.name.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'));
        if !name_ok || !names.insert(connector.name.as_str()) {
            return Err(CoreError::Invalid(format!(
                "connector names are unique, 1 to 64 letters and digits: {}",
                connector.name
            )));
        }
        if connector.command.is_some() == connector.url.is_some() {
            return Err(CoreError::Invalid(format!("{}: give a command or a URL", connector.name)));
        }
        if connector.url.as_deref().is_some_and(|url| !(url.starts_with("https://") || url.starts_with("http://"))) {
            return Err(CoreError::Invalid(format!("{}: an HTTP server's URL starts with https://", connector.name)));
        }
    }
    let json = serde_json::to_vec_pretty(list).map_err(|error| CoreError::Invalid(error.to_string()))?;
    write_atomically(&data_dir.join(FILE), &json)
}

/// The servers Claude Code's configuration lists, every tool off, their
/// secrets left where they are.
pub(crate) fn from_claude(config: &Path) -> Vec<PageConnector> {
    let Some(json) = read_capped(config, 4 << 20) else { return Vec::new() };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&json) else { return Vec::new() };
    let Some(servers) = value.get("mcpServers").and_then(serde_json::Value::as_object) else { return Vec::new() };
    servers
        .iter()
        .filter_map(|(name, server)| {
            let text = |key: &str| server.get(key).and_then(serde_json::Value::as_str).map(str::to_owned);
            let args = server
                .get("args")
                .and_then(serde_json::Value::as_array)
                .map(|args| args.iter().filter_map(|arg| arg.as_str().map(str::to_owned)).collect())
                .unwrap_or_default();
            // The variables Claude Code sets for it, by name only: their values stay there.
            let pass_env = server
                .get("env")
                .and_then(serde_json::Value::as_object)
                .map(|env| env.keys().cloned().collect())
                .unwrap_or_default();
            let connector = PageConnector {
                name: name.clone(),
                command: text("command"),
                args,
                url: text("url"),
                pass_env,
                ..PageConnector::default()
            };
            (connector.command.is_some() != connector.url.is_some()).then_some(connector)
        })
        .collect()
}

/// Where Claude Code keeps its MCP servers for this person.
pub(crate) fn claude_config() -> PathBuf {
    match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dir) => PathBuf::from(dir).join(".claude.json"),
        None => silt_os::home_dir().unwrap_or_default().join(".claude.json"),
    }
}

/// How long a connector has to start, and a call to answer.
const START_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);
const CALL_LIMIT: std::time::Duration = std::time::Duration::from_secs(60);

/// What every stdio connector gets of the daemon's environment: where
/// programs and the person's files are, the locale, and Silt's own home,
/// which `silt mcp` needs; never the daemon's tokens or keys.
const BASE_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "LANG",
    "LC_ALL",
    "TERM",
    "TMPDIR",
    "TMP",
    "TEMP",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
    "XDG_RUNTIME_DIR",
    "SILT_HOME",
    "SystemRoot",
    "SystemDrive",
    "windir",
    "ComSpec",
    "PATHEXT",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
];

/// The running clients, by connector, each with when it was last used.
#[derive(Default)]
pub(crate) struct Clients(Mutex<HashMap<String, (Arc<Client>, Instant)>>);

/// A running connection to one MCP server.
type Client = RunningService<RoleClient, ()>;

impl Clients {
    /// The connector's client, started if it isn't running; idle ones end.
    /// The lock is held only to look and to keep: a connector slow to
    /// start doesn't hold up every other page's calls.
    async fn client(&self, connector: &PageConnector) -> Result<Arc<Client>, String> {
        {
            let mut running = self.0.lock().await;
            let now = Instant::now();
            running.retain(|_, (_, used)| now.duration_since(*used) < IDLE);
            if let Some((client, used)) = running.get_mut(&connector.name) {
                *used = now;
                return Ok(Arc::clone(client));
            }
        }
        let started = tokio::time::timeout(START_LIMIT, start(connector))
            .await
            .map_err(|_| format!("{} didn't start within {} seconds", connector.name, START_LIMIT.as_secs()))??;
        let mut running = self.0.lock().await;
        let (client, _) = running.entry(connector.name.clone()).or_insert_with(|| (Arc::new(started), Instant::now()));
        Ok(Arc::clone(client))
    }

    pub(crate) async fn tools(&self, connector: &PageConnector) -> Result<Vec<ConnectorTool>, String> {
        let client = self.client(connector).await?;
        let tools = tokio::time::timeout(CALL_LIMIT, client.list_all_tools())
            .await
            .map_err(|_| format!("{} didn't list its tools in time", connector.name))?
            .map_err(|error| error.to_string())?;
        Ok(tools
            .into_iter()
            .map(|tool| ConnectorTool {
                read_only: tool.annotations.as_ref().and_then(|annotations| annotations.read_only_hint).unwrap_or(false),
                name: tool.name.to_string(),
                description: tool.description.map(|text| text.to_string()),
                input_schema: serde_json::Value::Object((*tool.input_schema).clone()),
            })
            .collect())
    }

    pub(crate) async fn call(
        &self,
        connector: &PageConnector,
        tool: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let client = self.client(connector).await?;
        let arguments = match input {
            serde_json::Value::Object(object) => Some(object),
            serde_json::Value::Null => None,
            _ => return Err("a tool's input is an object".into()),
        };
        let mut params = CallToolRequestParams::new(tool.to_owned());
        if let Some(arguments) = arguments {
            params = params.with_arguments(arguments);
        }
        let result = tokio::time::timeout(CALL_LIMIT, client.call_tool(params))
            .await
            .map_err(|_| format!("{tool} didn't answer within {} seconds", CALL_LIMIT.as_secs()))?
            .map_err(|error| error.to_string())?;
        let value = serde_json::to_value(&result).map_err(|error| error.to_string())?;
        if value.to_string().len() > RESULT_BYTES {
            return Err(format!("the tool's result is over {} MiB", RESULT_BYTES >> 20));
        }
        Ok(value)
    }
}

async fn start(connector: &PageConnector) -> Result<Client, String> {
    if let Some(program) = &connector.command {
        // On Windows `npx` and its kind are `.cmd` files, which a bare name misses.
        let path = std::env::var_os("PATH").unwrap_or_default();
        let found = silt_os::find_program(program, &path).unwrap_or_else(|| program.into());
        let mut command = tokio::process::Command::new(found);
        command.args(&connector.args).stderr(std::process::Stdio::null()).kill_on_drop(true).env_clear();
        for name in BASE_ENV.iter().copied().chain(connector.pass_env.iter().map(String::as_str)) {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        silt_os::own_group(&mut command);
        let transport =
            rmcp::transport::TokioChildProcess::new(command).map_err(|error| format!("couldn't start {program}: {error}"))?;
        return ().serve(transport).await.map_err(|error| error.to_string());
    }
    let url = connector.url.clone().ok_or("the connector has no command or URL")?;
    let mut headers = reqwest::header::HeaderMap::new();
    for (header, variable) in &connector.header_env {
        let value = std::env::var(variable).map_err(|_| format!("{variable} isn't set, which {header} comes from"))?;
        let name = reqwest::header::HeaderName::from_bytes(header.as_bytes()).map_err(|error| error.to_string())?;
        let value = reqwest::header::HeaderValue::from_str(&value).map_err(|error| error.to_string())?;
        headers.insert(name, value);
    }
    let http = reqwest::Client::builder().default_headers(headers).build().map_err(|error| error.to_string())?;
    let config = rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(url);
    let transport = rmcp::transport::StreamableHttpClientTransport::with_client(http, config);
    ().serve(transport).await.map_err(|error| error.to_string())
}

/// Whether a page's `mcp` declaration names `server`'s `tool`.
pub(crate) fn declared(config: &serde_json::Value, server: &str, tool: &str) -> bool {
    config.get("servers").and_then(serde_json::Value::as_array).is_some_and(|servers| {
        servers.iter().any(|entry| {
            entry.get("server").and_then(serde_json::Value::as_str) == Some(server)
                && entry
                    .get("tools")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|tools| tools.iter().any(|named| named.as_str() == Some(tool)))
        })
    })
}

/// What a page asks of a connector's tool (ADR 0080).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum McpAsk {
    /// The tool's description, and whether it only reads.
    Describe { server: String, tool: String },
    Call {
        server: String,
        tool: String,
        #[serde(default)]
        input: serde_json::Value,
    },
}

impl Core {
    /// Answers a page's `mcp` call on this machine, off the event loop.
    pub(crate) fn page_mcp(
        &self,
        thread: &str,
        artifact: &str,
        asked: McpAsk,
    ) -> Result<impl Future<Output = Result<serde_json::Value, CoreError>> + Send + 'static, CoreError> {
        let (server, tool) = match &asked {
            McpAsk::Describe { server, tool } | McpAsk::Call { server, tool, .. } => (server.clone(), tool.clone()),
        };
        let connector = self.page_connector(thread, artifact, &server, &tool)?;
        let clients = Arc::clone(&self.page_clients);
        Ok(async move {
            match asked {
                McpAsk::Describe { .. } => {
                    let tools = clients.tools(&connector).await.map_err(CoreError::Invalid)?;
                    let found = tools.into_iter().find(|candidate| candidate.name == tool);
                    let found = found.ok_or_else(|| CoreError::Invalid(format!("{server} has no tool {tool}")))?;
                    Ok(serde_json::to_value(found).unwrap_or_default())
                }
                McpAsk::Call { input, .. } => clients.call(&connector, &tool, input).await.map_err(CoreError::Invalid),
            }
        })
    }

    /// A connector's tools, for Settings to tick.
    pub(crate) fn connector_tools(
        &self,
        name: &str,
    ) -> Result<impl Future<Output = Result<Vec<ConnectorTool>, CoreError>> + Send + 'static, CoreError> {
        let connector = connectors(&self.data_dir)
            .into_iter()
            .find(|connector| connector.name == name)
            .ok_or_else(|| CoreError::Invalid(format!("no page connector {name}")))?;
        let clients = Arc::clone(&self.page_clients);
        Ok(async move { clients.tools(&connector).await.map_err(CoreError::Invalid) })
    }

    /// Adds the servers Claude Code lists that aren't here yet, every tool off.
    pub(crate) fn import_page_connectors(&self) -> Result<Vec<PageConnector>, CoreError> {
        let mut list = connectors(&self.data_dir);
        for imported in from_claude(&claude_config()) {
            if !list.iter().any(|connector| connector.name == imported.name) {
                list.push(imported);
            }
        }
        set_connectors(&self.data_dir, &list)?;
        Ok(list)
    }

    /// The connector a page may call `tool` of: declared by the version,
    /// set up here, and ticked for pages.
    pub(crate) fn page_connector(
        &self,
        thread: &str,
        artifact: &str,
        server: &str,
        tool: &str,
    ) -> Result<PageConnector, CoreError> {
        let thread = self.find_thread(thread)?;
        let id: silt_types::ArtifactId =
            artifact.parse().map_err(|_| CoreError::Invalid(format!("{artifact} isn't an artifact id")))?;
        // The thread's workspace, and its org, may name the connectors its pages call.
        if let Some(family) = self.threads.get(&thread).map(|record| record.family) {
            let workspace = self.family_workspace(family);
            self.rules(&workspace)
                .check_connector(server)
                .map_err(|refusal| CoreError::Invalid(format!("not_granted: {refusal}")))?;
        }
        let config = self.declared(thread, id, "mcp")?;
        if !declared(&config, server, tool) {
            return Err(CoreError::Invalid(format!("not_granted: the page doesn't declare {server}'s {tool}")));
        }
        connectors(&self.data_dir)
            .into_iter()
            .find(|connector| connector.name == server && connector.tools.get(tool) == Some(&true))
            .ok_or_else(|| {
                CoreError::Invalid(format!("not_granted: {server}'s {tool} isn't a page connector's tool ticked here"))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_reaches_only_tools_it_declares() {
        let config = serde_json::json!({"servers": [{"server": "tracker", "tools": ["list_issues"]}]});
        assert!(declared(&config, "tracker", "list_issues"));
        assert!(!declared(&config, "tracker", "close_issue"));
        assert!(!declared(&config, "other", "list_issues"));
        assert!(!declared(&serde_json::json!({"servers": [{"server": "tracker", "tools": []}]}), "tracker", "list_issues"));
    }

    #[test]
    fn connectors_are_checked_and_claude_codes_are_imported_with_every_tool_off() {
        let dir = tempfile::tempdir().expect("dir");
        let stdio = PageConnector { name: "tracker".into(), command: Some("tracker-mcp".into()), ..PageConnector::default() };
        assert!(set_connectors(dir.path(), &[stdio.clone(), stdio.clone()]).is_err(), "names are unique");
        assert!(set_connectors(dir.path(), &[PageConnector { name: "none".into(), ..PageConnector::default() }]).is_err());
        set_connectors(dir.path(), std::slice::from_ref(&stdio)).expect("kept");
        assert_eq!(connectors(dir.path()), [stdio]);

        let config = dir.path().join(".claude.json");
        std::fs::write(
            &config,
            r#"{"mcpServers": {"gh": {"url": "https://api.example.com/mcp", "headers": {"Authorization": "Bearer x"}}, "fs": {"command": "fs-mcp", "args": ["--root", "."], "env": {"FS_TOKEN": "secret"}}}}"#,
        )
        .expect("write");
        let imported = from_claude(&config);
        assert_eq!(imported.len(), 2);
        assert!(
            imported.iter().all(|connector| connector.tools.is_empty() && connector.header_env.is_empty()),
            "no secrets, no tools on"
        );
        let fs = imported.iter().find(|connector| connector.name == "fs").expect("fs");
        assert_eq!(fs.pass_env, ["FS_TOKEN"], "the variable's name, never its value");
        assert!(!serde_json::to_string(&imported).expect("json").contains("secret"));
    }
}
