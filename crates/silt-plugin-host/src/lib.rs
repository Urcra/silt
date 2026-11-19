//! Runs plugins (ADR 0104). Each is a WebAssembly component, compiled once
//! and instantiated fresh for every call, with:
//! - no files but its own data folder, no network and no process;
//! - limits on memory, time and what it returns;
//! - only the `silt:plugin` interfaces its manifest declares and the person
//!   granted: a plugin importing another doesn't load;
//! - while it prepares a call, only what looks: fetching, reading, resolving;
//! - while it runs one, only the effects that call was approved for.
//!
//! What the capabilities do on this machine (the gateway, git, SSH, the MCP
//! server's sandbox) is the daemon's, behind [`Capabilities`]; this crate
//! decides whether each may be done.

mod capabilities;
mod limits;
mod state;
#[doc(hidden)]
pub mod testing;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use silt_plugin::{Call, Context, Manifest, Observed, Output, Prepared, Refused, Tool};
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Engine, Store};

pub use capabilities::{Capabilities, Commit, Connection, FileChange, Folder, HttpRequest, HttpResponse, Scope, SshOutput};
pub use limits::Limits;
use state::{Phase, State};

mod bindings {
    wasmtime::component::bindgen!({
        world: "silt-plugin",
        path: "../silt-plugin-api/wit",
        with: {
            "silt:plugin/sockets.connection": crate::capabilities::ConnectionResource,
        },
    });
}

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("{plugin} isn't a component Silt can run: {why}")]
    Load { plugin: String, why: String },
    #[error("{plugin} uses {interface}, which needs the {capability} capability it wasn't granted")]
    NotGranted { plugin: String, interface: String, capability: String },
    #[error("{plugin} uses {interface}, which this Silt doesn't offer plugins")]
    Unknown { plugin: String, interface: String },
    #[error("{plugin} stopped: {why}")]
    Trap { plugin: String, why: String },
    #[error("{plugin} answered with something Silt can't use: {why}")]
    Answer { plugin: String, why: String },
    #[error("{0}")]
    Plugin(String),
    #[error("{plugin} declared the effects {declared}, which its capabilities can't cause")]
    Undeclarable { plugin: String, declared: String },
}

/// A plugin compiled, its imports checked against its grants.
pub struct Loaded {
    pub manifest: Arc<Manifest>,
    component: Component,
}

/// Everything one call of a plugin is given.
#[derive(Clone)]
pub struct Env {
    pub capabilities: Arc<dyn Capabilities>,
    pub scope: Scope,
    /// The plugin's own folder: all of the file system it sees.
    pub data_dir: PathBuf,
    /// The capabilities the person granted, by name.
    pub granted: BTreeSet<String>,
}

/// How a run went, as the host saw it.
#[derive(Debug)]
pub struct Ran {
    pub output: Result<Output, String>,
    /// Every effect the plugin caused, in order.
    pub observed: Vec<Observed>,
    /// Effects it tried that weren't approved, each refused.
    pub refused: Vec<Refused>,
}

pub struct Host {
    engine: Engine,
    linker: Linker<State>,
    limits: Limits,
    _ticker: limits::Ticker,
}

/// Which interface stands for which capability. `host` is always there.
const INTERFACES: [(&str, Option<&str>); 8] = [
    ("host", None),
    ("http", Some("http")),
    ("git", Some("git")),
    ("thread", Some("thread-read")),
    ("reach", Some("reach")),
    ("sockets", Some("sockets")),
    ("ssh", Some("ssh")),
    ("mcp", Some("mcp-process")),
];

impl Host {
    pub fn new(limits: Limits) -> Result<Self, HostError> {
        let mut config = wasmtime::Config::new();
        config.wasm_component_model(true);
        config.epoch_interruption(true);
        config.max_wasm_stack(1 << 20);
        let engine = Engine::new(&config).map_err(|error| HostError::Load { plugin: "silt".into(), why: error.to_string() })?;
        let mut linker = Linker::<State>::new(&engine);
        let mut wiring = || -> wasmtime::Result<()> {
            wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
            bindings::SiltPlugin::add_to_linker::<State, wasmtime::component::HasSelf<State>>(&mut linker, |state| state)?;
            Ok(())
        };
        wiring().map_err(|error| HostError::Load { plugin: "silt".into(), why: error.to_string() })?;
        let ticker = limits::Ticker::start(engine.clone());
        Ok(Self { engine, linker, limits, _ticker: ticker })
    }

    /// Compiles `wasm` and checks it imports only what `granted` allows.
    pub fn load(&self, manifest: Manifest, wasm: &[u8], granted: &BTreeSet<String>) -> Result<Loaded, HostError> {
        let plugin = manifest.id.clone();
        let component = Component::from_binary(&self.engine, wasm)
            .map_err(|error| HostError::Load { plugin: plugin.clone(), why: error.to_string() })?;
        let kind = component.component_type();
        for (name, _) in kind.imports(&self.engine) {
            check_import(&plugin, name, &manifest, granted)?;
        }
        Ok(Loaded { manifest: Arc::new(manifest), component })
    }

    fn store(&self, loaded: &Loaded, env: &Env, phase: Phase) -> Result<Store<State>, HostError> {
        let plugin = loaded.manifest.id.clone();
        std::fs::create_dir_all(&env.data_dir)
            .map_err(|error| HostError::Load { plugin: plugin.clone(), why: error.to_string() })?;
        let mut wasi = wasmtime_wasi::WasiCtxBuilder::new();
        wasi.allow_tcp(false).allow_udp(false).allow_ip_name_lookup(false);
        wasi.preopened_dir(&env.data_dir, "/data", wasmtime_wasi::FsPerms::ReadWrite)
            .map_err(|error| HostError::Load { plugin: plugin.clone(), why: error.to_string() })?;
        let state = State {
            wasi: wasi.build(),
            table: ResourceTable::new(),
            limits: self.limits.store_limits(),
            env: env.clone(),
            manifest: loaded.manifest.clone(),
            phase,
            observed: Vec::new(),
            refused: Vec::new(),
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|state| &mut state.limits);
        Ok(store)
    }

    fn instantiate(&self, loaded: &Loaded, store: &mut Store<State>, seconds: u64) -> Result<bindings::SiltPlugin, HostError> {
        store.set_epoch_deadline(seconds * limits::TICKS_PER_SECOND);
        bindings::SiltPlugin::instantiate(&mut *store, &loaded.component, &self.linker)
            .map_err(|error| HostError::Trap { plugin: loaded.manifest.id.clone(), why: limits::explain(&error) })
    }

    /// The tools the plugin offers, checked against its actions.
    pub fn describe(&self, loaded: &Loaded, env: &Env) -> Result<Vec<Tool>, HostError> {
        let plugin = loaded.manifest.id.clone();
        let mut store = self.store(loaded, env, Phase::Describe)?;
        let instance = self.instantiate(loaded, &mut store, self.limits.describe_seconds)?;
        let text = instance
            .silt_plugin_plugin()
            .call_describe(&mut store)
            .map_err(|error| HostError::Trap { plugin: plugin.clone(), why: limits::explain(&error) })?;
        let tools: Vec<Tool> = self.answer(&plugin, &text)?;
        let mut names = BTreeSet::new();
        for tool in &tools {
            if loaded.manifest.action(&tool.action).is_none() {
                return Err(HostError::Answer {
                    plugin,
                    why: format!("its tool {} is for {}, an action its manifest doesn't declare", tool.name, tool.action),
                });
            }
            if !names.insert(tool.name.as_str()) || tool.name.is_empty() {
                return Err(HostError::Answer { plugin, why: format!("it offers {:?} twice, or without a name", tool.name) });
            }
        }
        Ok(tools)
    }

    /// Prepares a call. Nothing may change while it does.
    pub fn prepare(
        &self,
        loaded: &Loaded,
        env: &Env,
        tool: &str,
        input: &serde_json::Value,
        context: &Context,
    ) -> Result<Prepared, HostError> {
        let plugin = loaded.manifest.id.clone();
        let mut store = self.store(loaded, env, Phase::Prepare)?;
        let instance = self.instantiate(loaded, &mut store, self.limits.prepare_seconds)?;
        let answer = instance
            .silt_plugin_plugin()
            .call_prepare(&mut store, tool, &input.to_string(), &json(context))
            .map_err(|error| HostError::Trap { plugin: plugin.clone(), why: limits::explain(&error) })?
            .map_err(HostError::Plugin)?;
        let call: Call = self.answer(&plugin, &answer)?;
        let action = loaded.manifest.action(&call.action).ok_or_else(|| HostError::Answer {
            plugin: plugin.clone(),
            why: format!("{} isn't one of its actions", call.action),
        })?;
        if let Some(field) = call.fields.keys().find(|field| !action.fields.contains(field)) {
            return Err(HostError::Answer { plugin, why: format!("{field} isn't a field {} declares", action.id) });
        }
        call.card.check().map_err(|why| HostError::Answer { plugin: plugin.clone(), why })?;
        capabilities::check_effects(&loaded.manifest, env, &call.effects)
            .map_err(|declared| HostError::Undeclarable { plugin: plugin.clone(), declared })?;
        Ok(Prepared {
            plugin,
            version: loaded.manifest.version.clone(),
            tool: tool.to_owned(),
            action: call.action,
            environment: context.environment.clone(),
            fields: call.fields,
            input: input.clone(),
            card: call.card,
            effects: call.effects,
        })
    }

    /// Runs a prepared call as it was allowed: only its effects get through.
    pub fn run(&self, loaded: &Loaded, env: &Env, prepared: &Prepared, context: &Context) -> Result<Ran, HostError> {
        let plugin = loaded.manifest.id.clone();
        let reads = loaded.manifest.action(&prepared.action).is_some_and(|action| action.kind == silt_plugin::ActionKind::Read);
        let ledger = silt_plugin::Ledger::new(&prepared.effects, reads);
        let mut store = self.store(loaded, env, Phase::Run(Box::new(ledger)))?;
        let instance = self.instantiate(loaded, &mut store, self.limits.run_seconds)?;
        let call = Call {
            action: prepared.action.clone(),
            fields: prepared.fields.clone(),
            card: prepared.card.clone(),
            effects: prepared.effects.clone(),
        };
        let answer = instance
            .silt_plugin_plugin()
            .call_run(&mut store, &prepared.tool, &prepared.input.to_string(), &json(&call), &json(context))
            .map_err(|error| HostError::Trap { plugin: plugin.clone(), why: limits::explain(&error) });
        let state = store.into_data();
        let output = match answer {
            Ok(Ok(text)) => self.answer::<Output>(&plugin, &text).map_err(|error| error.to_string()),
            Ok(Err(why)) => Err(why),
            Err(error) => Err(error.to_string()),
        };
        Ok(Ran { output, observed: state.observed, refused: state.refused })
    }

    fn answer<T: serde::de::DeserializeOwned>(&self, plugin: &str, text: &str) -> Result<T, HostError> {
        if text.len() > self.limits.answer_bytes {
            return Err(HostError::Answer {
                plugin: plugin.to_owned(),
                why: format!("its answer is over {} bytes", self.limits.answer_bytes),
            });
        }
        serde_json::from_str(text).map_err(|error| HostError::Answer { plugin: plugin.to_owned(), why: error.to_string() })
    }
}

fn json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "{}".into())
}

/// Whether a plugin may import `name`: WASI's own, and the `silt:plugin`
/// interfaces of the capabilities it was granted.
fn check_import(plugin: &str, name: &str, manifest: &Manifest, granted: &BTreeSet<String>) -> Result<(), HostError> {
    if name.starts_with("wasi:") {
        return if name.starts_with("wasi:http/") {
            Err(HostError::Unknown { plugin: plugin.to_owned(), interface: name.to_owned() })
        } else {
            Ok(())
        };
    }
    let interface = name
        .strip_prefix("silt:plugin/")
        .map(|rest| rest.split('@').next().unwrap_or(rest))
        .ok_or_else(|| HostError::Unknown { plugin: plugin.to_owned(), interface: name.to_owned() })?;
    let (_, capability) = INTERFACES
        .iter()
        .find(|(known, _)| *known == interface)
        .ok_or_else(|| HostError::Unknown { plugin: plugin.to_owned(), interface: name.to_owned() })?;
    match capability {
        None => Ok(()),
        Some(capability) if manifest.capability(capability).is_some() && granted.contains(*capability) => Ok(()),
        Some(capability) => Err(HostError::NotGranted {
            plugin: plugin.to_owned(),
            interface: name.to_owned(),
            capability: (*capability).to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests;
