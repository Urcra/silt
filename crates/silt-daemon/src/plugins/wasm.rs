//! WebAssembly plugins' calls (ADR 0100): the tools they offer a run, a call
//! prepared by its plugin and decided by its rules, approved by the hash of
//! what it prepared, and run with only the effects it declared.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use silt_plugin::{Block, Card, Context, Decision, Prepared, Rule, Rules, Tool, decide, environment_for};
use silt_plugin_host::{Env, Host, Limits, Loaded, Scope};

use super::PluginTool;
use super::capabilities::DaemonCapabilities;
use super::installed::{self, Installed};

/// The host, made on first use, and each plugin compiled once per version
/// and grant.
#[derive(Default)]
pub(crate) struct Wasm {
    host: OnceLock<Result<Arc<Host>, String>>,
    loaded: Mutex<HashMap<String, (String, Arc<Loaded>)>>,
    tools: Mutex<HashMap<String, (String, Vec<Tool>)>>,
}

/// A plugin call as kept in the store: what its plugin prepared, how its
/// rules decided it, and what the plugin was told.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WasmRequest {
    pub(crate) prepared: Prepared,
    pub(crate) decision: Decision,
    pub(crate) context: Context,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Wasm {
    fn host(&self) -> Result<Arc<Host>, String> {
        self.host.get_or_init(|| Host::new(Limits::default()).map(Arc::new).map_err(|error| error.to_string())).clone()
    }

    /// `installed`, compiled with its grants.
    pub(crate) fn loaded(&self, data_dir: &Path, installed: &Installed) -> Result<Arc<Loaded>, String> {
        let key = format!("{}@{}:{:?}", installed.id, installed.version, installed.granted);
        if let Some((cached, loaded)) = lock(&self.loaded).get(&installed.id)
            && *cached == key
        {
            return Ok(loaded.clone());
        }
        let manifest = installed::manifest(data_dir, &installed.id).map_err(|error| error.to_string())?;
        let wasm = installed::component(data_dir, &installed.id).map_err(|error| error.to_string())?;
        let loaded = Arc::new(self.host()?.load(manifest, &wasm, &installed.granted).map_err(|error| error.to_string())?);
        lock(&self.loaded).insert(installed.id.clone(), (key, loaded.clone()));
        Ok(loaded)
    }

    /// The tools `installed` offers, asked once per version.
    fn tools_of(&self, data_dir: &Path, installed: &Installed) -> Result<Vec<Tool>, String> {
        if let Some((version, tools)) = lock(&self.tools).get(&installed.id)
            && *version == installed.version
        {
            return Ok(tools.clone());
        }
        let loaded = self.loaded(data_dir, installed)?;
        let env = env_for(
            data_dir,
            installed,
            &loaded,
            Scope { plugin: installed.id.clone(), ..Scope::default() },
            None,
            tokio::runtime::Handle::current(),
        );
        let tools = self.host()?.describe(&loaded, &env).map_err(|error| error.to_string())?;
        lock(&self.tools).insert(installed.id.clone(), (installed.version.clone(), tools.clone()));
        Ok(tools)
    }

    /// The tools of the plugins on in `workspace`, as agents see them.
    pub(crate) fn tools(&self, data_dir: &Path, workspace: &str) -> Vec<PluginTool> {
        let mut offered = Vec::new();
        for installed in installed::load(data_dir).into_iter().filter(|installed| installed.on_in(workspace)) {
            match self.tools_of(data_dir, &installed) {
                Ok(tools) => offered.extend(tools.into_iter().map(|tool| PluginTool {
                    name: installed::tool_name(&installed.id, &tool.name),
                    description: tool.description,
                    input_schema: tool.input_schema,
                })),
                Err(error) => tracing::warn!(plugin = %installed.id, %error, "a plugin's tools couldn't be listed"),
            }
        }
        offered
    }

    /// The plugin on in `workspace` that offers `name`, and its own name for
    /// the tool.
    pub(crate) fn find(&self, data_dir: &Path, workspace: &str, name: &str) -> Option<(Installed, Tool)> {
        installed::load(data_dir).into_iter().filter(|installed| installed.on_in(workspace)).find_map(|installed| {
            let tools = self.tools_of(data_dir, &installed).ok()?;
            let tool = tools.into_iter().find(|tool| installed::tool_name(&installed.id, &tool.name) == name)?;
            Some((installed, tool))
        })
    }

    /// Prepares a call and decides it by the plugin's rules. Blocking: runs
    /// the plugin.
    pub(crate) fn prepare(&self, call: &CallParts, input: &Value) -> Result<WasmRequest, String> {
        let loaded = self.loaded(&call.data_dir, &call.installed)?;
        let manifest = &loaded.manifest;
        // The environment a call goes to is picked from its input before the
        // plugin looks at anything, so the right credential goes with it.
        let environment = match &manifest.environments {
            Some(environments) => {
                let value = input
                    .get(&environments.field)
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("give `{}`", environments.field))?;
                let globs = call.installed.environment_globs();
                let picked = environment_for(value, &globs).ok_or_else(|| {
                    format!("{value} is in none of {}'s {}; add it to one in its settings", manifest.name, environments.label)
                })?;
                Some((environments.field.clone(), value.to_owned(), picked.to_owned()))
            }
            None => None,
        };
        let context = Context {
            thread: call.thread.clone(),
            workspace: call.workspace.clone(),
            environment: environment.as_ref().map(|(_, _, name)| name.clone()),
            environments: call.installed.environments.clone().into_iter().collect(),
        };
        let env = call.env(&loaded);
        let prepared = self.host()?.prepare(&loaded, &env, &call.tool, input, &context).map_err(|error| error.to_string())?;
        if let Some((field, value, _)) = &environment
            && prepared.fields.get(field) != Some(value)
        {
            return Err(format!("{} prepared a call for another {field} than it was asked for", manifest.name));
        }
        let action = manifest.action(&prepared.action).ok_or_else(|| format!("{} isn't one of its actions", prepared.action))?;
        let trusted = installed::trusted(&call.data_dir).contains(&manifest.publisher);
        let rules = Rules(call.installed.rules.clone());
        let decision = decide(action, trusted, &rules, None, &prepared.fields);
        Ok(WasmRequest { prepared, decision, context })
    }

    /// Runs an allowed call. Blocking: runs the plugin.
    pub(crate) fn run(&self, call: &CallParts, request: &WasmRequest) -> Result<silt_plugin_host::Ran, String> {
        let loaded = self.loaded(&call.data_dir, &call.installed)?;
        if loaded.manifest.version != request.prepared.version {
            return Err(format!("{} was updated since this call was prepared; ask again", loaded.manifest.name));
        }
        let env = call.env(&loaded);
        self.host()?.run(&loaded, &env, &request.prepared, &request.context).map_err(|error| error.to_string())
    }
}

/// What a call needs from the daemon, gathered before it goes to a
/// blocking thread.
#[derive(Clone)]
pub(crate) struct CallParts {
    pub(crate) data_dir: PathBuf,
    pub(crate) installed: Installed,
    pub(crate) tool: String,
    pub(crate) thread: String,
    pub(crate) workspace: String,
    pub(crate) environment: Option<String>,
    pub(crate) checkout: Option<PathBuf>,
    pub(crate) handle: tokio::runtime::Handle,
}

impl CallParts {
    fn env(&self, loaded: &Loaded) -> Env {
        let scope = Scope {
            plugin: self.installed.id.clone(),
            thread: self.thread.clone(),
            workspace: self.workspace.clone(),
            environment: self.environment.clone(),
        };
        env_for(&self.data_dir, &self.installed, loaded, scope, self.checkout.clone(), self.handle.clone())
    }
}

fn env_for(
    data_dir: &Path,
    installed: &Installed,
    loaded: &Loaded,
    scope: Scope,
    checkout: Option<PathBuf>,
    handle: tokio::runtime::Handle,
) -> Env {
    let thread_dir = installed::thread_dir(data_dir, &installed.id, if scope.thread.is_empty() { "none" } else { &scope.thread });
    let work = data_dir.join("work").join(&scope.thread);
    let capabilities = DaemonCapabilities {
        data_dir: data_dir.to_owned(),
        manifest: loaded.manifest.clone(),
        installed: installed.clone(),
        thread_dir,
        checkout,
        work: (!scope.thread.is_empty() && work.is_dir()).then_some(work),
        handle,
    };
    Env {
        capabilities: Arc::new(capabilities),
        scope,
        data_dir: installed::dir(data_dir, &installed.id).join("data"),
        granted: installed.granted.clone(),
    }
}

/// A card as text, for the terminal and for agents told what waits.
pub(crate) fn card_text(card: &Card) -> String {
    let card = card.shown();
    let mut out = vec![card.title.clone()];
    for block in &card.blocks {
        match block {
            Block::Fields { pairs } => out.extend(pairs.iter().map(|(name, value)| format!("  {name}  {value}"))),
            Block::List { label, items } => {
                out.push(format!("{label}:"));
                out.extend(items.iter().map(|item| match &item.key {
                    Some(key) => format!("  {key}  {}", item.text),
                    None => format!("  {}", item.text),
                }));
            }
            Block::Diff { label, files } => {
                out.push(format!("{label}:"));
                out.extend(files.iter().map(|file| format!("  {}  +{} -{}", file.path, file.added, file.removed)));
            }
            Block::Code { label, text } => {
                out.push(format!("{label}:"));
                out.extend(text.lines().map(|line| format!("  {line}")));
            }
            Block::Note { text, .. } => out.push(format!("note: {text}")),
        }
    }
    out.join("\n")
}

/// Whether the decision lets the call go without asking, ask, or refuses.
pub(crate) fn asks(decision: &Decision) -> Option<bool> {
    match decision.rule {
        Rule::Allow => Some(false),
        Rule::Ask => Some(true),
        Rule::Never => None,
    }
}

/// The plugins' installed capabilities a thread's sandbox gets as folders:
/// each plugin with `agent-files` on in `workspace` shows its `agent/`
/// folder for `thread` at `/silt/<name>`, read-only.
pub(crate) fn agent_folders(data_dir: &Path, workspace: &str, thread: &str) -> Vec<(PathBuf, PathBuf)> {
    installed::load(data_dir)
        .into_iter()
        .filter(|installed| installed.on_in(workspace) && installed.granted.contains("agent-files"))
        .map(|installed| {
            let source = installed::thread_dir(data_dir, &installed.id, thread).join("agent");
            let name = installed.id.split_once('.').map_or(installed.id.as_str(), |(_, name)| name).to_owned();
            (source, PathBuf::from("/silt").join(name))
        })
        .collect()
}

/// The capabilities `manifest` declares, as granted when it's installed
/// with the person's consent.
pub(crate) fn declared(manifest: &silt_plugin::Manifest) -> BTreeSet<String> {
    manifest.capabilities.iter().map(|capability| capability.name().to_owned()).collect()
}
