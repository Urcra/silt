//! Local models on this machine (ADR 0066): the ones downloaded, the
//! catalog's others, what this machine can run, and downloads, which start
//! only when the person asks for one.

use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use silt_llm::{Budget, Device, Hub, Installed, LlmError, ModelId, Models, Plan, Remote, Source};

use crate::core::CoreError;
use crate::protocol::{CatalogModel, LocalModel, LocalModels, ModelFit, ModelPull};

pub(crate) struct LocalModelsHere {
    models: Models,
    store: silt_store::Store,
    /// The model gateway's address, once it's started (ADR 0066).
    gateway: tokio::sync::OnceCell<String>,
    hub: Hub,
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

/// What `silt model pull` and the web UI name: a catalog entry, a file on
/// Hugging Face, or a GGUF file here.
enum Wanted {
    Catalog(&'static silt_llm::Entry),
    Remote(Remote),
    File(PathBuf),
}

impl Wanted {
    fn parse(source: &str) -> Result<Self, LlmError> {
        if let Some(entry) = silt_llm::entry(source) {
            return Ok(Self::Catalog(entry));
        }
        let path = Path::new(source);
        if path.is_absolute() {
            return Ok(Self::File(path.to_owned()));
        }
        Remote::parse(source).map(Self::Remote)
    }

    fn id(&self) -> Result<ModelId, LlmError> {
        match self {
            Self::Catalog(entry) => ModelId::new(entry.id),
            Self::Remote(remote) => ModelId::for_file(&remote.file),
            Self::File(path) => ModelId::for_file(&path.to_string_lossy()),
        }
    }
}

impl LocalModelsHere {
    pub(crate) fn new(data_dir: &Path, store: silt_store::Store) -> Self {
        let hub = env::var("SILT_HUGGINGFACE_URL").map(Hub::new).unwrap_or_default();
        Self {
            models: Models::new(data_dir.join("models").join("llm")),
            store,
            gateway: tokio::sync::OnceCell::new(),
            hub,
            client: reqwest::Client::new(),
            pulling: Mutex::default(),
            devices: Mutex::default(),
        }
    }

    /// Where agents reach local models, starting the gateway the first time.
    pub(crate) async fn gateway(&self) -> Result<String, CoreError> {
        let url = self
            .gateway
            .get_or_try_init(|| crate::gateway::start(self.store.clone(), self.models.clone(), Self::server()))
            .await?;
        Ok(url.clone())
    }

    /// The models here, as `local`'s model setting offers them.
    pub(crate) fn installed_now(&self) -> Vec<Installed> {
        self.models.list_now()
    }

    /// The `llama-server` this machine runs models with.
    pub(crate) fn server() -> Option<PathBuf> {
        silt_llm::find_server(env::var_os("SILT_LLAMA_SERVER"), env::var_os("PATH").as_deref(), Path::is_file)
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
        let server = Self::server();
        let budget = self.budget(server.as_deref()).await;
        let installed = self.models.list().await;
        let fit =
            |size: u64, kv: Option<u64>, context: u32| ModelFit::from(silt_llm::fit(silt_llm::need(size, kv, context), &budget));
        let models = installed
            .iter()
            .map(|model| LocalModel {
                id: model.id.to_string(),
                name: model.name.clone(),
                source: source_text(model),
                size: model.size,
                context: model.context,
                fit: fit(model.size, model.meta.kv_bytes_per_token(), model.context),
                path: self.models.weights(model),
            })
            .collect();
        let catalog = silt_llm::CATALOG
            .iter()
            .filter(|entry| !installed.iter().any(|model| model.id.as_str() == entry.id))
            .map(|entry| CatalogModel {
                id: entry.id.to_owned(),
                name: entry.name.to_owned(),
                about: entry.about.to_owned(),
                size: entry.size(),
                context: entry.context,
                tested: entry.tested,
                fit: fit(entry.size(), None, entry.context),
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
            catalog,
            pull: pulling.current.clone(),
            failed: pulling
                .failed
                .as_ref()
                .map(|(id, why)| crate::protocol::ModelFailure { id: id.clone(), reason: why.clone() }),
        }
    }

    /// Starts fetching `source`, one download at a time, and answers with
    /// the model's name.
    pub(crate) fn start_pull(self: &Arc<Self>, source: &str) -> Result<String, CoreError> {
        let wanted = Wanted::parse(source)?;
        let id = wanted.id()?;
        {
            let mut pulling = self.pulling.lock().expect("lock");
            if let Some(current) = &pulling.current {
                return Err(CoreError::Invalid(format!("{} is still downloading; one model at a time", current.id)));
            }
            pulling.current = Some(ModelPull { id: id.to_string(), done: 0, total: 0 });
            pulling.failed = None;
        }
        let here = self.clone();
        let name = id.to_string();
        tokio::spawn(async move {
            let result = here.pull(wanted).await;
            let mut pulling = here.pulling.lock().expect("lock");
            pulling.current = None;
            if let Err(error) = result {
                pulling.failed = Some((name, error.to_string()));
            }
        });
        Ok(id.to_string())
    }

    async fn pull(&self, wanted: Wanted) -> Result<Installed, LlmError> {
        let plan = match wanted {
            Wanted::File(path) => return self.models.add_file(&path).await,
            Wanted::Catalog(entry) => Plan::catalog(entry, &self.hub)?,
            Wanted::Remote(remote) => Plan::remote(&self.client, &self.hub, remote).await?,
        };
        let pulling = &self.pulling;
        self.models
            .pull(&self.client, plan, &mut |progress| {
                if let Some(current) = pulling.lock().expect("lock").current.as_mut() {
                    current.done = progress.done;
                    current.total = progress.total;
                }
            })
            .await
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
/// traffic of its own besides (ADR 0066).
pub(crate) fn claude_env(gateway: &str, token: &str, model: &ModelId) -> Vec<(String, String)> {
    let mut env = vec![
        ("ANTHROPIC_BASE_URL".to_owned(), gateway.to_owned()),
        ("ANTHROPIC_AUTH_TOKEN".to_owned(), token.to_owned()),
        ("ANTHROPIC_API_KEY".to_owned(), token.to_owned()),
        ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".to_owned(), "1".to_owned()),
    ];
    for tier in [
        "ANTHROPIC_MODEL",
        "ANTHROPIC_SMALL_FAST_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
    ] {
        env.push((tier.to_owned(), format!("local:{model}")));
    }
    env
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
        Source::Catalog => "Silt's catalog".to_owned(),
        Source::HuggingFace { remote } => format!("{}:{}", remote.repo, remote.file),
        Source::File { path } => path.display().to_string(),
    }
}
