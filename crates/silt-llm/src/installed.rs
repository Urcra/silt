//! The models on this machine: a directory each under the models dir, with
//! its files and `model.json`, which says where they came from. A model's
//! directory appears only once every file is whole.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::io::AsyncReadExt;

use crate::download::{self, Wanted};
use crate::hub::{Hub, PinnedFile};
use crate::{Entry, GgufError, LlmError, Meta, ModelId, Remote, parse_gguf};

const MANIFEST: &str = "model.json";
/// The context a model the catalog doesn't know starts with, at most.
pub const DEFAULT_CONTEXT: u32 = 32_768;
/// How far into a file its header may go: past the vocabulary, which can be
/// a few megabytes.
const MOST_HEADER_BYTES: u64 = 64 << 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    Catalog,
    HuggingFace {
        remote: Remote,
    },
    /// A file the person has, used where it is.
    File {
        path: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installed {
    pub id: ModelId,
    pub name: String,
    pub source: Source,
    /// The downloaded files, the one to load first; none for a file used
    /// where it is.
    pub files: Vec<PinnedFile>,
    pub size: u64,
    pub context: u32,
    pub flags: Vec<String>,
    #[serde(default)]
    pub meta: Meta,
}

/// What to fetch for a model, before any of it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub id: ModelId,
    pub name: String,
    pub source: Source,
    pub files: Vec<(PinnedFile, String)>,
    /// The catalog's tested context, if it has one.
    pub context: Option<u32>,
    pub flags: Vec<String>,
}

impl Plan {
    pub fn size(&self) -> u64 {
        self.files.iter().map(|(file, _)| file.size).sum()
    }

    pub fn catalog(entry: &Entry, hub: &Hub) -> Result<Self, LlmError> {
        let remote = Remote { repo: crate::Repo::new(entry.repo)?, file: entry.files[0].path.to_owned() };
        Ok(Self {
            id: ModelId::new(entry.id)?,
            name: entry.name.to_owned(),
            source: Source::Catalog,
            files: entry
                .files
                .iter()
                .map(|file| {
                    let pinned = PinnedFile { path: file.path.to_owned(), sha256: file.sha256.to_owned(), size: file.size };
                    (pinned, hub.file_url(&remote, file.path))
                })
                .collect(),
            context: Some(entry.context),
            flags: entry.flags.iter().map(|flag| (*flag).to_owned()).collect(),
        })
    }

    /// Asks the Hub for the file's pins; they're kept from then on.
    pub async fn remote(client: &reqwest::Client, hub: &Hub, remote: Remote) -> Result<Self, LlmError> {
        let files = hub.files(client, &remote).await?;
        let name = remote.file.rsplit('/').next().unwrap_or(&remote.file).trim_end_matches(".gguf").to_owned();
        Ok(Self {
            id: ModelId::for_file(&remote.file)?,
            name,
            files: files.into_iter().map(|file| (file.clone(), hub.file_url(&remote, &file.path))).collect(),
            source: Source::HuggingFace { remote },
            context: None,
            flags: Vec::new(),
        })
    }
}

/// How much of a download is there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
}

#[derive(Debug, Clone)]
pub struct Models {
    dir: PathBuf,
}

impl Models {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn model_dir(&self, id: &ModelId) -> PathBuf {
        self.dir.join(id.as_str())
    }

    fn partial_dir(&self, id: &ModelId) -> PathBuf {
        self.dir.join(format!("{id}.partial"))
    }

    pub async fn list(&self) -> Vec<Installed> {
        let mut found = Vec::new();
        let Ok(mut entries) = fs::read_dir(&self.dir).await else {
            return found;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let Some(id) = entry.file_name().to_str().and_then(|name| ModelId::new(name).ok()) else {
                continue;
            };
            if let Some(model) = self.get(&id).await {
                found.push(model);
            }
        }
        found.sort_by(|a, b| a.id.cmp(&b.id));
        found
    }

    pub async fn get(&self, id: &ModelId) -> Option<Installed> {
        let bytes = fs::read(self.model_dir(id).join(MANIFEST)).await.ok()?;
        serde_json::from_slice::<Installed>(&bytes).ok().filter(|model| model.id == *id)
    }

    /// The file `llama-server` loads.
    pub fn weights(&self, model: &Installed) -> PathBuf {
        match (&model.source, model.files.first()) {
            (Source::File { path }, _) => path.clone(),
            (_, Some(first)) => self.model_dir(&model.id).join(&first.path),
            (_, None) => self.model_dir(&model.id),
        }
    }

    /// Removes a model and any download of it left unfinished. A file the
    /// person brought stays where it is.
    pub async fn remove(&self, id: &ModelId) -> io::Result<bool> {
        let mut removed = false;
        for dir in [self.model_dir(id), self.partial_dir(id)] {
            match fs::remove_dir_all(&dir).await {
                Ok(()) => removed = true,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(removed)
    }

    /// Fetches what `plan` names, resuming an earlier try's files, and
    /// reports progress over all of them.
    pub async fn pull(
        &self,
        client: &reqwest::Client,
        plan: Plan,
        on_progress: &mut (impl FnMut(Progress) + Send),
    ) -> Result<Installed, LlmError> {
        if let Some(model) = self.get(&plan.id).await {
            return if model.source == plan.source { Ok(model) } else { Err(LlmError::Taken(plan.id)) };
        }
        let partial = self.partial_dir(&plan.id);
        let total = plan.size();
        let mut before = 0;
        for (file, url) in &plan.files {
            let to = partial.join(&file.path);
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent).await.map_err(LlmError::Disk)?;
            }
            if fs::metadata(&to).await.is_err() {
                let wanted = Wanted { url: url.clone(), sha256: file.sha256.clone(), size: file.size };
                download::fetch(client, &wanted, &to, &mut |done| on_progress(Progress { done: before + done, total })).await?;
            }
            before += file.size;
            on_progress(Progress { done: before, total });
        }
        let first = plan.files.first().map(|(file, _)| partial.join(&file.path));
        let meta = match &first {
            Some(first) => header(first).await?,
            None => Meta::default(),
        };
        let model = Installed {
            context: context(plan.context, &meta),
            id: plan.id,
            name: meta.name.clone().filter(|_| plan.source != Source::Catalog).unwrap_or(plan.name),
            source: plan.source,
            files: plan.files.into_iter().map(|(file, _)| file).collect(),
            size: total,
            flags: plan.flags,
            meta,
        };
        let manifest = serde_json::to_vec_pretty(&model).expect("a manifest serializes");
        fs::write(partial.join(MANIFEST), manifest).await.map_err(LlmError::Disk)?;
        fs::rename(&partial, self.model_dir(&model.id)).await.map_err(LlmError::Disk)?;
        Ok(model)
    }

    /// Adds a GGUF file the person has, where it is.
    pub async fn add_file(&self, path: &Path) -> Result<Installed, LlmError> {
        let path = fs::canonicalize(path).await.map_err(LlmError::Disk)?;
        let not_a_model = || LlmError::NotAModel(path.clone());
        let name =
            path.file_name().and_then(|name| name.to_str()).filter(|name| name.ends_with(".gguf")).ok_or_else(not_a_model)?;
        let id = ModelId::for_file(name)?;
        let source = Source::File { path: path.clone() };
        if let Some(model) = self.get(&id).await {
            return if model.source == source { Ok(model) } else { Err(LlmError::Taken(id)) };
        }
        let size = fs::metadata(&path).await.map_err(LlmError::Disk)?.len();
        let meta = header(&path).await?;
        let model = Installed {
            context: context(None, &meta),
            name: meta.name.clone().unwrap_or_else(|| name.trim_end_matches(".gguf").to_owned()),
            id,
            source,
            files: Vec::new(),
            size,
            flags: Vec::new(),
            meta,
        };
        let dir = self.model_dir(&model.id);
        fs::create_dir_all(&dir).await.map_err(LlmError::Disk)?;
        let manifest = serde_json::to_vec_pretty(&model).expect("a manifest serializes");
        fs::write(dir.join(MANIFEST), manifest).await.map_err(LlmError::Disk)?;
        Ok(model)
    }
}

fn context(tested: Option<u32>, meta: &Meta) -> u32 {
    let most = meta.context_length.and_then(|length| u32::try_from(length).ok()).unwrap_or(u32::MAX);
    tested.unwrap_or(DEFAULT_CONTEXT).min(most)
}

/// Reads a GGUF file's header, reading further only when it goes on.
async fn header(path: &Path) -> Result<Meta, LlmError> {
    let mut file = fs::File::open(path).await.map_err(LlmError::Disk)?;
    let mut bytes = Vec::new();
    let mut want = 1 << 20;
    loop {
        let missing = want - bytes.len() as u64;
        let read = (&mut file).take(missing).read_to_end(&mut bytes).await.map_err(LlmError::Disk)?;
        match parse_gguf(&bytes) {
            Err(GgufError::Truncated) if read > 0 && want < MOST_HEADER_BYTES => want *= 4,
            Err(GgufError::Truncated) => return Err(LlmError::NotAModel(path.to_owned())),
            result => return Ok(result?),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gguf::tests::Header;

    fn model_bytes() -> Vec<u8> {
        let mut bytes = Header::default()
            .text("general.architecture", "llama")
            .text("general.name", "Tiny Llama")
            .number("llama.context_length", 4096)
            .bytes();
        bytes.extend([0u8; 512]);
        bytes
    }

    #[tokio::test]
    async fn a_file_the_person_has_is_used_where_it_is() {
        let dir = tempfile::tempdir().expect("temp");
        let file = dir.path().join("Tiny Llama Q4.gguf");
        std::fs::write(&file, model_bytes()).expect("write");
        let models = Models::new(dir.path().join("models"));
        let model = models.add_file(&file).await.expect("added");
        assert_eq!(model.id.as_str(), "tiny-llama-q4");
        assert_eq!(model.name, "Tiny Llama");
        assert_eq!(model.context, 4096, "no more than the model knows");
        assert_eq!(models.weights(&model), std::fs::canonicalize(&file).expect("canonical"));
        assert_eq!(models.list().await, std::slice::from_ref(&model));
        assert!(models.remove(&model.id).await.expect("removed"));
        assert!(file.exists(), "the person's file stays");
        assert_eq!(models.list().await, []);
    }

    #[tokio::test]
    async fn what_isnt_a_gguf_is_refused() {
        let dir = tempfile::tempdir().expect("temp");
        let models = Models::new(dir.path().join("models"));
        let text = dir.path().join("notes.gguf");
        std::fs::write(&text, b"just text").expect("write");
        assert!(matches!(models.add_file(&text).await, Err(LlmError::Gguf(GgufError::NotGguf))));
        let other = dir.path().join("model.bin");
        std::fs::write(&other, model_bytes()).expect("write");
        assert!(matches!(models.add_file(&other).await, Err(LlmError::NotAModel(_))));
    }

    #[tokio::test]
    async fn a_header_longer_than_the_first_read_is_read_on() {
        let dir = tempfile::tempdir().expect("temp");
        let file = dir.path().join("long.gguf");
        let long = "x".repeat(3 << 20);
        let mut bytes = Header::default().text("general.architecture", "llama").text("tokenizer.chat_template", &long).bytes();
        bytes.extend([0u8; 16]);
        std::fs::write(&file, bytes).expect("write");
        let meta = header(&file).await.expect("read");
        assert!(meta.chat_template);
    }

    #[tokio::test]
    async fn a_model_from_the_hub_is_pinned_fetched_and_kept() {
        use axum::Router;
        use axum::routing::get;
        use sha2::{Digest, Sha256};

        let bytes = model_bytes();
        let sha256: String = Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect();
        let listing = serde_json::json!([
            {"type": "file", "path": "README.md", "size": 5},
            {"type": "file", "path": "Tiny-Q4.gguf", "size": bytes.len(), "lfs": {"oid": sha256, "size": bytes.len()}},
        ]);
        let served = bytes.clone();
        let app = Router::new()
            .route("/api/models/owner/tiny/tree/main", get(move || async move { axum::Json(listing) }))
            .route("/owner/tiny/resolve/main/Tiny-Q4.gguf", get(move || async move { served }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let hub = Hub::new(format!("http://{}", listener.local_addr().expect("address")));
        tokio::spawn(async move { axum::serve(listener, app).await });

        let dir = tempfile::tempdir().expect("temp");
        let models = Models::new(dir.path().to_owned());
        let client = reqwest::Client::new();
        let remote = Remote::parse("owner/tiny:Tiny-Q4.gguf").expect("remote");
        let plan = Plan::remote(&client, &hub, remote.clone()).await.expect("planned");
        assert_eq!(plan.size(), bytes.len() as u64);
        let mut last = None;
        let model = models.pull(&client, plan.clone(), &mut |progress| last = Some(progress)).await.expect("pulled");
        assert_eq!(last, Some(Progress { done: bytes.len() as u64, total: bytes.len() as u64 }));
        assert_eq!(model.id.as_str(), "tiny-q4");
        assert_eq!(model.name, "Tiny Llama");
        assert_eq!(model.files[0].sha256, sha256);
        assert_eq!(std::fs::read(models.weights(&model)).expect("read"), bytes);
        assert!(!dir.path().join("tiny-q4.partial").exists());
        assert_eq!(models.pull(&client, plan, &mut |_| ()).await.expect("kept"), model, "a second pull finds it there");
    }

    #[test]
    fn a_new_models_context_is_the_tested_one_or_the_default_within_what_it_knows() {
        let knows = |length| Meta { context_length: Some(length), ..Meta::default() };
        assert_eq!(context(None, &Meta::default()), DEFAULT_CONTEXT);
        assert_eq!(context(Some(65_536), &knows(131_072)), 65_536);
        assert_eq!(context(None, &knows(8192)), 8192);
    }
}
