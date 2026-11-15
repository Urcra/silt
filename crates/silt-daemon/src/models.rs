//! Local models on this machine (ADR 0070): the ones downloaded, finding
//! others on Hugging Face, what this machine can run, llama.cpp itself, and
//! downloads, which start only when the person asks for one.

use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use silt_llm::{Budget, Device, Hub, Installed, LlmError, ModelId, Models, Plan, Remote, Repo, Runtimes, Source};

use crate::core::CoreError;
use crate::protocol::{FoundModel, LocalModel, LocalModels, ModelFiles, ModelFit, ModelPull, ModelQuantization};

/// What a download of llama.cpp itself is called while it runs.
pub const RUNTIME: &str = "llama.cpp";
/// Models a search answers with.
const FOUND: usize = 30;

pub(crate) struct LocalModelsHere {
    models: Models,
    runtimes: Runtimes,
    store: silt_store::Store,
    /// The model gateway and its address, once it's started (ADR 0070).
    gateway: tokio::sync::OnceCell<(String, std::sync::Arc<crate::gateway::Gateway>)>,
    hub: Hub,
    /// Where llama.cpp's releases are listed: GitHub's, or a stand-in in tests.
    releases: String,
    client: reqwest::Client,
    pulling: Mutex<Pulling>,
    /// What the server found last listed, so a listing doesn't run it again.
    devices: Mutex<Option<(PathBuf, Vec<Device>)>>,
}

#[derive(Default)]
struct Pulling {
    current: Option<ModelPull>,
    failed: Option<(String, String)>,
}

/// What `silt model pull` and the web UI name: a file on Hugging Face, or a
/// GGUF file here.
enum Wanted {
    Remote(Remote),
    File(PathBuf),
}

impl Wanted {
    fn parse(source: &str) -> Result<Self, LlmError> {
        let path = Path::new(source);
        if path.is_absolute() {
            return Ok(Self::File(path.to_owned()));
        }
        Remote::parse(source).map(Self::Remote)
    }

    fn id(&self) -> Result<ModelId, LlmError> {
        match self {
            Self::Remote(remote) => ModelId::for_file(&remote.file),
            Self::File(path) => ModelId::for_file(&path.to_string_lossy()),
        }
    }
}

/// The `llama-server` this machine runs models with: `$SILT_LLAMA_SERVER`,
/// else the `PATH`'s, else the build of llama.cpp Silt fetched.
pub(crate) fn server_in(runtimes: &Runtimes) -> Option<PathBuf> {
    silt_llm::find_server(env::var_os("SILT_LLAMA_SERVER"), env::var_os("PATH").as_deref(), Path::is_file)
        .or_else(|| runtimes.server())
}

impl LocalModelsHere {
    pub(crate) fn new(data_dir: &Path, store: silt_store::Store) -> Self {
        let hub = env::var("SILT_HUGGINGFACE_URL").map(Hub::new).unwrap_or_default();
        let releases = env::var("SILT_LLAMA_RELEASES_URL").unwrap_or_else(|_| silt_llm::RELEASES.to_owned());
        Self {
            models: Models::new(data_dir.join("models").join("llm")),
            runtimes: Runtimes::new(data_dir.join("runtimes").join("llama.cpp")),
            store,
            gateway: tokio::sync::OnceCell::new(),
            hub,
            releases,
            client: reqwest::Client::new(),
            pulling: Mutex::default(),
            devices: Mutex::default(),
        }
    }

    /// Where agents reach local models, starting the gateway the first time.
    pub(crate) async fn gateway(&self) -> Result<String, CoreError> {
        let (url, _) = self
            .gateway
            .get_or_try_init(|| crate::gateway::start(self.store.clone(), self.models.clone(), self.runtimes.clone()))
            .await?;
        Ok(url.clone())
    }

    /// Stops the model, if one runs, so nothing of an erased thread stays in
    /// its cache.
    pub(crate) async fn unload(&self) {
        if let Some((_, gateway)) = self.gateway.get() {
            gateway.unload().await;
        }
    }

    /// The models here, as `local`'s model setting offers them.
    pub(crate) fn installed_now(&self) -> Vec<Installed> {
        self.models.list_now()
    }

    fn server(&self) -> Option<PathBuf> {
        server_in(&self.runtimes)
    }

    async fn budget(&self, server: Option<&Path>) -> Budget {
        let memory = silt_os::total_memory().unwrap_or(0);
        let Some(server) = server else {
            return Budget { devices: Vec::new(), memory };
        };
        let known = self.devices.lock().expect("lock").clone().filter(|(listed_by, _)| listed_by == server);
        let devices = match known {
            Some((_, devices)) => devices,
            None => {
                let devices = silt_llm::list_devices(server).await.unwrap_or_default();
                *self.devices.lock().expect("lock") = Some((server.to_owned(), devices.clone()));
                devices
            }
        };
        Budget { devices, memory }
    }

    pub(crate) async fn view(&self) -> LocalModels {
        let server = self.server();
        let budget = self.budget(server.as_deref()).await;
        let installed = self.models.list().await;
        let models = installed
            .iter()
            .map(|model| LocalModel {
                id: model.id.to_string(),
                name: model.name.clone(),
                source: source_text(model),
                size: model.size,
                context: model.context,
                fit: ModelFit::from(silt_llm::fit(
                    silt_llm::need(model.size, model.meta.kv_bytes_per_token(), model.context),
                    &budget,
                )),
                path: self.models.weights(model),
            })
            .collect();
        let pulling = self.pulling.lock().expect("lock");
        LocalModels {
            server: server.map(|server| server.display().to_string()),
            devices: budget
                .devices
                .iter()
                .map(|device| crate::protocol::ModelDevice {
                    name: device.name.clone(),
                    description: device.description.clone(),
                    total: device.total,
                    free: device.free,
                })
                .collect(),
            room: budget.room(),
            models,
            pull: pulling.current.clone(),
            failed: pulling
                .failed
                .as_ref()
                .map(|(id, why)| crate::protocol::ModelFailure { id: id.clone(), reason: why.clone() }),
        }
    }

    /// GGUF models on Hugging Face matching `words`, the most downloaded
    /// first; with no words, the most downloaded of all.
    pub(crate) async fn search(&self, words: &str) -> Result<Vec<FoundModel>, CoreError> {
        let found = self.hub.search(&self.client, words, FOUND).await?;
        Ok(found
            .into_iter()
            .map(|found| FoundModel { repo: found.repo.to_string(), downloads: found.downloads, updated: found.updated })
            .collect())
    }

    /// A repository's quantizations, each with whether it fits here and the
    /// context it would start with.
    pub(crate) async fn files(&self, repo: &str) -> Result<ModelFiles, CoreError> {
        let repo = Repo::new(repo)?;
        let model = self.hub.model(&self.client, &repo).await?;
        let budget = self.budget(self.server().as_deref()).await;
        let quantizations = model
            .quantizations
            .iter()
            .map(|quantization| {
                let size = quantization.size();
                let context = silt_llm::context_to_fit(size, None, model.context_length, &budget);
                ModelQuantization {
                    source: format!("{repo}:{}", quantization.file),
                    label: quantization.label.clone(),
                    size,
                    parts: u32::try_from(quantization.files.len()).unwrap_or(u32::MAX),
                    fit: ModelFit::from(silt_llm::fit(silt_llm::need(size, None, context), &budget)),
                    context,
                }
            })
            .collect();
        Ok(ModelFiles {
            repo: repo.to_string(),
            architecture: model.architecture,
            context_length: model.context_length,
            quantizations,
        })
    }

    /// Takes the one download slot for `id`, or says what holds it.
    fn begin(&self, id: &str) -> Result<(), CoreError> {
        let mut pulling = self.pulling.lock().expect("lock");
        if let Some(current) = &pulling.current {
            return Err(CoreError::Invalid(format!("{} is still downloading; one download at a time", current.id)));
        }
        pulling.current = Some(ModelPull { id: id.to_owned(), done: 0, total: 0 });
        pulling.failed = None;
        Ok(())
    }

    fn finish(&self, id: String, result: Result<(), LlmError>) {
        let mut pulling = self.pulling.lock().expect("lock");
        pulling.current = None;
        if let Err(error) = result {
            pulling.failed = Some((id, error.to_string()));
        }
    }

    fn progress(&self, done: u64, total: u64) {
        if let Some(current) = self.pulling.lock().expect("lock").current.as_mut() {
            current.done = done;
            current.total = total;
        }
    }

    /// Starts fetching `source`, one download at a time, and answers with
    /// the model's name.
    pub(crate) fn start_pull(self: &Arc<Self>, source: &str) -> Result<String, CoreError> {
        let wanted = Wanted::parse(source)?;
        let id = wanted.id()?;
        self.begin(id.as_str())?;
        let here = self.clone();
        let name = id.to_string();
        tokio::spawn(async move {
            let result = here.pull(wanted).await.map(drop);
            here.finish(name, result);
        });
        Ok(id.to_string())
    }

    async fn pull(&self, wanted: Wanted) -> Result<Installed, LlmError> {
        let plan = match wanted {
            Wanted::File(path) => {
                let model = self.models.add_file(&path).await?;
                return self.sized(model).await;
            }
            Wanted::Remote(remote) => Plan::remote(&self.client, &self.hub, remote).await?,
        };
        let model = self.models.pull(&self.client, plan, &mut |progress| self.progress(progress.done, progress.total)).await?;
        self.sized(model).await
    }

    /// Gives a model the most context that fits this machine, now that its
    /// header says how large its KV cache is.
    async fn sized(&self, model: Installed) -> Result<Installed, LlmError> {
        let budget = self.budget(self.server().as_deref()).await;
        let context = silt_llm::context_to_fit(model.size, model.meta.kv_bytes_per_token(), model.meta.context_length, &budget);
        if context == model.context {
            return Ok(model);
        }
        self.models.set_context(&model.id, context).await
    }

    /// Starts fetching llama.cpp's latest build for this machine.
    pub(crate) fn start_runtime_pull(self: &Arc<Self>) -> Result<String, CoreError> {
        self.begin(RUNTIME)?;
        let here = self.clone();
        tokio::spawn(async move {
            let result = here.runtimes.pull(&here.client, &here.releases, &mut |done, total| here.progress(done, total)).await;
            if result.is_ok() {
                *here.devices.lock().expect("lock") = None;
            }
            here.finish(RUNTIME.to_owned(), result.map(drop));
        });
        Ok(RUNTIME.to_owned())
    }

    pub(crate) async fn remove(&self, id: &str) -> Result<String, CoreError> {
        let id = ModelId::new(id)?;
        if self.pulling.lock().expect("lock").current.as_ref().is_some_and(|current| current.id == id.as_str()) {
            return Err(CoreError::Invalid(format!("{id} is downloading; wait for it, then remove it")));
        }
        let kept = match self.models.get(&id).await.map(|model| model.source) {
            Some(Source::File { path }) => format!("; {} stays where it is", path.display()),
            _ => String::new(),
        };
        match self.models.remove(&id).await {
            Ok(true) => Ok(format!("removed {id}{kept}")),
            Ok(false) => Err(CoreError::Invalid(format!("there's no model {id} here"))),
            Err(error) => Err(CoreError::Invalid(format!("couldn't remove {id}: {error}"))),
        }
    }
}

/// What Claude Code needs to run on the local model `model`: the gateway as
/// its API, the run's token as its key, the model for every tier, and no
/// traffic of its own besides (ADR 0070).
pub(crate) fn claude_env(gateway: &str, token: &str, model: &Installed) -> Vec<(String, String)> {
    let id = &model.id;
    let mut env = vec![
        // Claude Code otherwise compacts for its own models' context, far past a local one's.
        ("CLAUDE_CODE_AUTO_COMPACT_WINDOW".to_owned(), model.context.to_string()),
        ("ANTHROPIC_BASE_URL".to_owned(), gateway.to_owned()),
        ("ANTHROPIC_AUTH_TOKEN".to_owned(), token.to_owned()),
        ("ANTHROPIC_API_KEY".to_owned(), token.to_owned()),
        ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".to_owned(), "1".to_owned()),
    ];
    // Each of these would send it to a cloud's own endpoint, past the base URL.
    for cloud in ["CLAUDE_CODE_USE_BEDROCK", "CLAUDE_CODE_USE_VERTEX", "CLAUDE_CODE_USE_FOUNDRY"] {
        env.push((cloud.to_owned(), "0".to_owned()));
    }
    env.extend(no_proxy());
    for tier in [
        "ANTHROPIC_MODEL",
        "ANTHROPIC_SMALL_FAST_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
    ] {
        env.push((tier.to_owned(), format!("local:{id}")));
    }
    env
}

/// `NO_PROXY` with loopback added, so a proxy the environment names is
/// never handed what goes to the gateway.
pub(crate) fn no_proxy() -> Vec<(String, String)> {
    let mut hosts: Vec<String> = env::var("NO_PROXY")
        .or_else(|_| env::var("no_proxy"))
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .map(str::to_owned)
        .collect();
    for loopback in ["127.0.0.1", "localhost", "::1"] {
        if !hosts.iter().any(|host| host == loopback) {
            hosts.push(loopback.to_owned());
        }
    }
    let hosts = hosts.join(",");
    vec![("NO_PROXY".to_owned(), hosts.clone()), ("no_proxy".to_owned(), hosts)]
}

impl LocalModelsHere {
    /// The installed model `model`, or why it can't be run.
    pub(crate) async fn installed(&self, model: &ModelId) -> Result<Installed, CoreError> {
        self.models.get(model).await.ok_or_else(|| {
            CoreError::Invalid(format!(
                "there's no local model {model} here; download it with `silt model pull` or in Settings › Local models"
            ))
        })
    }
}

fn source_text(model: &Installed) -> String {
    match &model.source {
        Source::HuggingFace { remote } => format!("{}:{}", remote.repo, remote.file),
        Source::File { path } => path.display().to_string(),
    }
}
