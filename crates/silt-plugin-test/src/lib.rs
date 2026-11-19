//! A stand-in for Silt's plugin host, for testing a plugin (ADR 0103). It
//! loads the component as Silt does, with what its manifest declares
//! granted, answers its HTTP from what the test says, and records every
//! request, so a test sees what each call prepares and what it sends.
//!
//! ```no_run
//! use silt_plugin_test::Harness;
//!
//! let notes = Harness::build(env!("CARGO_MANIFEST_DIR").as_ref()).unwrap();
//! notes.answer("POST", "https://api.notes.example/", 201, "{}");
//! let call = notes.prepare("add", serde_json::json!({"notebook": "work", "text": "hi"})).unwrap();
//! let ran = notes.run(&call).unwrap();
//! assert!(ran.refused.is_empty());
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

pub use silt_plugin::{Call, Card, Context, Effect, Manifest, Observed, Output, Prepared, Refused, Tool};
use silt_plugin_host::{Capabilities, Env, Host, Limits, Loaded, Scope};
pub use silt_plugin_host::{HttpRequest, HttpResponse, Ran};

#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    #[error("the manifest: {0}")]
    Manifest(String),
    #[error(transparent)]
    Host(#[from] silt_plugin_host::HostError),
    #[error("building it: {0}")]
    Build(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
struct Answer {
    method: String,
    prefix: String,
    response: HttpResponse,
}

#[derive(Default)]
struct StandIn {
    settings: Mutex<BTreeMap<String, serde_json::Value>>,
    answers: Mutex<Vec<Answer>>,
    requests: Mutex<Vec<HttpRequest>>,
}

impl StandIn {
    fn requests(&self) -> std::sync::MutexGuard<'_, Vec<HttpRequest>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Capabilities for StandIn {
    fn setting(&self, _scope: &Scope, key: &str) -> Option<String> {
        self.settings.lock().unwrap_or_else(PoisonError::into_inner).get(key).map(ToString::to_string)
    }

    fn http(&self, _scope: &Scope, request: HttpRequest) -> Result<HttpResponse, String> {
        self.requests().push(request.clone());
        let answers = self.answers.lock().unwrap_or_else(PoisonError::into_inner);
        answers
            .iter()
            .rev()
            .find(|answer| answer.method.eq_ignore_ascii_case(&request.method) && request.url.starts_with(&answer.prefix))
            .map(|answer| answer.response.clone())
            .ok_or_else(|| format!("the test gave no answer for {} {}", request.method, request.url))
    }
}

/// A plugin loaded in the stand-in host.
pub struct Harness {
    host: Host,
    loaded: Loaded,
    stand_in: Arc<StandIn>,
    env: Env,
    context: Context,
    _data: tempfile::TempDir,
}

impl Harness {
    /// Loads `wasm`, built from the manifest `manifest_text`, with every
    /// capability it declares granted: a component importing more doesn't
    /// load, as in Silt.
    pub fn new(manifest_text: &str, wasm: &[u8]) -> Result<Self, HarnessError> {
        let manifest = Manifest::parse(manifest_text).map_err(|error| HarnessError::Manifest(error.to_string()))?;
        let granted: BTreeSet<String> = manifest.capabilities.iter().map(|capability| capability.name().to_owned()).collect();
        let host = Host::new(Limits::default())?;
        let loaded = host.load(manifest.clone(), wasm, &granted)?;
        let stand_in = Arc::new(StandIn::default());
        let data = tempfile::tempdir()?;
        let (thread, workspace) = ("test-thread".to_owned(), "test".to_owned());
        let scope =
            Scope { plugin: manifest.id.clone(), thread: thread.clone(), workspace: workspace.clone(), environment: None };
        let env = Env { capabilities: stand_in.clone(), scope, data_dir: data.path().join("data"), granted };
        let context = Context { thread, workspace, ..Context::default() };
        Ok(Self { host, loaded, stand_in, env, context, _data: data })
    }

    /// Builds the plugin in `folder`, a crate with its `silt-plugin.toml`,
    /// for `wasm32-wasip2` with cargo, and loads it.
    pub fn build(folder: &Path) -> Result<Self, HarnessError> {
        let manifest_text = std::fs::read_to_string(folder.join("silt-plugin.toml"))?;
        let manifest = Manifest::parse(&manifest_text).map_err(|error| HarnessError::Manifest(error.to_string()))?;
        let silt_plugin::Build::Cargo { package, .. } = &manifest.build else {
            return Err(HarnessError::Build("only a plugin built with cargo builds here".to_owned()));
        };
        let target = folder.join("target").join("silt-plugin-test");
        let status = std::process::Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
            .args(["build", "--release", "--target", "wasm32-wasip2", "-p", package, "--target-dir"])
            .arg(&target)
            .current_dir(folder)
            .status()?;
        if !status.success() {
            return Err(HarnessError::Build(format!("cargo build of {package} failed")));
        }
        let wasm = target.join("wasm32-wasip2/release").join(format!("{}.wasm", package.replace('-', "_")));
        Self::new(&manifest_text, &std::fs::read(wasm)?)
    }

    pub fn manifest(&self) -> &Manifest {
        &self.loaded.manifest
    }

    /// Sets one of its settings, as the person would.
    pub fn set(&self, key: &str, value: serde_json::Value) -> &Self {
        self.stand_in.settings.lock().unwrap_or_else(PoisonError::into_inner).insert(key.to_owned(), value);
        self
    }

    /// Calls as in the environment `name`, as Silt picks one by its rules.
    pub fn in_environment(&mut self, name: &str) -> &mut Self {
        self.env.scope.environment = Some(name.to_owned());
        self.context.environment = Some(name.to_owned());
        self
    }

    /// Answers `method` requests to URLs starting with `prefix` with
    /// `status` and `body`; the latest answer that fits wins.
    pub fn answer(&self, method: &str, prefix: &str, status: u16, body: impl Into<Vec<u8>>) -> &Self {
        self.stand_in.answers.lock().unwrap_or_else(PoisonError::into_inner).push(Answer {
            method: method.to_owned(),
            prefix: prefix.to_owned(),
            response: HttpResponse { status, headers: Vec::new(), body: body.into() },
        });
        self
    }

    /// Every request that reached the stand-in, in order: what Silt let
    /// through, never what it refused.
    pub fn requests(&self) -> Vec<HttpRequest> {
        self.stand_in.requests().clone()
    }

    pub fn tools(&self) -> Result<Vec<Tool>, HarnessError> {
        Ok(self.host.describe(&self.loaded, &self.env)?)
    }

    /// Prepares a call of `tool`: its card and the effects it would have,
    /// checked against the manifest as Silt checks them.
    pub fn prepare(&self, tool: &str, input: serde_json::Value) -> Result<Prepared, HarnessError> {
        Ok(self.host.prepare(&self.loaded, &self.env, tool, &input, &self.context)?)
    }

    /// Runs `prepared` as approved: only its effects get through, and what
    /// else the plugin tries is in [`Ran::refused`].
    pub fn run(&self, prepared: &Prepared) -> Result<Ran, HarnessError> {
        Ok(self.host.run(&self.loaded, &self.env, prepared, &self.context)?)
    }
}

/// Checks a build as Silt would load it: it imports only what its manifest
/// declares, and describes its tools, each for an action it declares.
pub fn check(manifest_text: &str, wasm: &[u8]) -> Result<Vec<Tool>, HarnessError> {
    Harness::new(manifest_text, wasm)?.tools()
}
