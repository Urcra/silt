//! Hugging Face, where people find their models: searching its GGUF
//! models, a model's quantizations with their sizes, and the SHA-256 it
//! keeps for each file, which pins the download. Silt keeps no list of its
//! own, which would be out of date by the time it ships.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{LlmError, Remote, Repo, checked_file, split_part};

/// A file of a model, pinned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedFile {
    /// Relative to the repository and to the model's directory.
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

/// A model repository a search found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Found {
    pub repo: Repo,
    /// Downloads in the last month, as Hugging Face counts them.
    pub downloads: u64,
    pub updated: Option<String>,
}

/// One way a model was quantized: a file, or a split model's parts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quantization {
    /// What `silt model pull` takes after `owner/repo:`: the first part.
    pub file: String,
    /// Its kind, such as `Q4_K_M` or `UD-Q4_K_XL`.
    pub label: String,
    pub files: Vec<PinnedFile>,
}

impl Quantization {
    pub fn size(&self) -> u64 {
        self.files.iter().map(|file| file.size).sum()
    }
}

/// What Hugging Face says of a model repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Model {
    pub repo: Repo,
    pub architecture: Option<String>,
    /// The most context the model was trained for.
    pub context_length: Option<u64>,
    /// Smallest first.
    pub quantizations: Vec<Quantization>,
}

/// A file of the model endpoint's `siblings`.
#[derive(Debug, Clone, Deserialize)]
struct Sibling {
    rfilename: String,
    #[serde(default)]
    lfs: Option<Lfs>,
}

#[derive(Debug, Clone, Deserialize)]
struct Lfs {
    sha256: String,
    size: u64,
}

fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Files that ship beside a model and aren't one.
fn is_weights(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    name.ends_with(".gguf") && !name.starts_with("mmproj") && !name.contains("imatrix")
}

/// A quantization's kind from its file's stem: from the first part that
/// names one, such as `Q4_K_M`, `UD-Q4_K_XL`, `IQ3_XXS` or `MXFP4_MOE`.
pub fn label_of(stem: &str) -> String {
    let parts: Vec<&str> = stem.split(['-', '.']).collect();
    let names_one = |part: &str| {
        let upper = part.to_ascii_uppercase();
        let digit_after =
            |prefix: &str| upper.strip_prefix(prefix).is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()));
        upper == "UD"
            || digit_after("Q")
            || digit_after("IQ")
            || ["MXFP4", "NVFP4", "BF16", "F16", "F32"].iter().any(|kind| upper.starts_with(kind))
    };
    match parts.iter().position(|part| names_one(part)) {
        Some(at) => stem.split(['-', '.']).skip(at).collect::<Vec<_>>().join("-"),
        None => stem.to_owned(),
    }
}

fn pinned(sibling: &Sibling) -> Option<PinnedFile> {
    let lfs = sibling.lfs.as_ref().filter(|lfs| is_sha256(&lfs.sha256))?;
    let path = checked_file(&sibling.rfilename).ok()?;
    Some(PinnedFile { path: path.to_owned(), sha256: lfs.sha256.to_ascii_lowercase(), size: lfs.size })
}

/// A repository's quantizations, from its files: each whole file, and each
/// split model whose every part is there, as one.
fn quantizations(siblings: &[Sibling]) -> Vec<Quantization> {
    let mut found = Vec::new();
    for sibling in siblings.iter().filter(|sibling| is_weights(&sibling.rfilename)) {
        let path = &sibling.rfilename;
        let (dir, name) = path.rsplit_once('/').map_or(("", path.as_str()), |(dir, name)| (dir, name));
        let stem = name.trim_end_matches(".gguf");
        match split_part(stem) {
            Some(part) if part.index == 1 => {
                let files: Option<Vec<PinnedFile>> = (1..=part.count)
                    .map(|index| {
                        let name = format!("{}-{index:05}-of-{:05}.gguf", part.base, part.count);
                        let wanted = if dir.is_empty() { name } else { format!("{dir}/{name}") };
                        siblings.iter().find(|other| other.rfilename == wanted).and_then(pinned)
                    })
                    .collect();
                if let Some(files) = files {
                    found.push(Quantization { file: path.clone(), label: label_of(part.base), files });
                }
            }
            Some(_) => {}
            None => {
                if let Some(file) = pinned(sibling) {
                    found.push(Quantization { file: path.clone(), label: label_of(stem), files: vec![file] });
                }
            }
        }
    }
    found.sort_by_key(Quantization::size);
    found
}

/// Reads the model endpoint's answer (`/api/models/<repo>?blobs=true`).
pub fn parse_model(repo: Repo, answer: &Value) -> Result<Model, LlmError> {
    let siblings: Vec<Sibling> = serde_json::from_value(answer["siblings"].clone())
        .map_err(|error| LlmError::Fetch(format!("Hugging Face's listing of {repo} is unreadable: {error}")))?;
    let gguf = &answer["gguf"];
    Ok(Model {
        architecture: gguf["architecture"].as_str().map(str::to_owned),
        context_length: gguf["context_length"].as_u64(),
        quantizations: quantizations(&siblings),
        repo,
    })
}

/// Reads a search's answer (`/api/models?…`), skipping what isn't a
/// repository name Silt takes.
pub fn parse_search(answer: &Value) -> Vec<Found> {
    answer
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|model| {
            Some(Found {
                repo: Repo::new(model["id"].as_str()?).ok()?,
                downloads: model["downloads"].as_u64().unwrap_or(0),
                updated: model["lastModified"].as_str().or_else(|| model["createdAt"].as_str()).map(str::to_owned),
            })
        })
        .collect()
}

/// The pinned files `remote` names: its quantization's, a split model's
/// every part.
pub fn select(model: &Model, file: &str) -> Result<Vec<PinnedFile>, LlmError> {
    model
        .quantizations
        .iter()
        .find(|quantization| quantization.file == file || quantization.files.iter().any(|part| part.path == file))
        .map(|quantization| quantization.files.clone())
        .ok_or_else(|| LlmError::NotOnHub(format!("{}:{file}", model.repo)))
}

/// Hugging Face, or what stands in for it (`SILT_HUGGINGFACE_URL`, in tests).
#[derive(Debug, Clone)]
pub struct Hub {
    base: String,
}

impl Default for Hub {
    fn default() -> Self {
        Self::new("https://huggingface.co")
    }
}

/// Repositories a search answers with, at most.
pub const MOST_FOUND: usize = 50;

impl Hub {
    pub fn new(base: impl Into<String>) -> Self {
        Self { base: base.into().trim_end_matches('/').to_owned() }
    }

    pub fn file_url(&self, remote: &Remote, path: &str) -> String {
        format!("{}/{}/resolve/main/{path}", self.base, remote.repo)
    }

    async fn get(&self, client: &reqwest::Client, url: &str, query: &[(&str, &str)]) -> Result<Value, LlmError> {
        let response = client.get(url).query(query).send().await.map_err(|error| LlmError::Fetch(error.to_string()))?;
        if !response.status().is_success() {
            return Err(LlmError::Fetch(format!("Hugging Face answered {} for {url}", response.status())));
        }
        response.json().await.map_err(|error| LlmError::Fetch(error.to_string()))
    }

    /// GGUF text-generation models, the most downloaded first; with no
    /// words, the most downloaded of all.
    pub async fn search(&self, client: &reqwest::Client, words: &str, limit: usize) -> Result<Vec<Found>, LlmError> {
        let limit = limit.clamp(1, MOST_FOUND).to_string();
        let mut query =
            vec![("filter", "gguf"), ("pipeline_tag", "text-generation"), ("sort", "downloads"), ("limit", limit.as_str())];
        let words = words.trim();
        if !words.is_empty() {
            query.push(("search", words));
        }
        let answer = self.get(client, &format!("{}/api/models", self.base), &query).await?;
        Ok(parse_search(&answer))
    }

    /// A repository's quantizations and what it says of the model.
    pub async fn model(&self, client: &reqwest::Client, repo: &Repo) -> Result<Model, LlmError> {
        let answer = self.get(client, &format!("{}/api/models/{repo}", self.base), &[("blobs", "true")]).await?;
        parse_model(repo.clone(), &answer)
    }

    /// The pinned files `remote` needs.
    pub async fn files(&self, client: &reqwest::Client, remote: &Remote) -> Result<Vec<PinnedFile>, LlmError> {
        let model = self.model(client, &remote.repo).await?;
        select(&model, &remote.file)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const A: &str = "fadc3e5f8d42bf7e894a785b05082e47daee4df26680389817e2093056f088ad";
    const B: &str = "467c9bd92ea518539cf75bf5a5fbfbd35e9a0b40d766ccaa67bf120e12041df3";

    fn file(path: &str, size: u64, sha: Option<&str>) -> Value {
        match sha {
            Some(sha) => json!({"rfilename": path, "size": size, "lfs": {"sha256": sha, "size": size, "pointerSize": 133}}),
            None => json!({"rfilename": path, "size": size}),
        }
    }

    /// As Hugging Face answers for a repository like unsloth's.
    fn answer() -> Value {
        json!({
            "id": "unsloth/Big-GGUF",
            "gguf": {"architecture": "qwen35moe", "context_length": 262_144, "total": 122_111_526_912_u64},
            "siblings": [
                file(".gitattributes", 9487, None),
                file("README.md", 100, None),
                file("Big-UD-Q4_K_XL.gguf", 4000, Some(A)),
                file("Big-Q8_0.gguf", 8000, Some(A)),
                file("mmproj-F16.gguf", 900, Some(A)),
                file("imatrix_unsloth.gguf", 400, Some(A)),
                file("Q4_K_M/Big-Q4_K_M-00001-of-00003.gguf", 10, Some(B)),
                file("Q4_K_M/Big-Q4_K_M-00002-of-00003.gguf", 5000, Some(A)),
                file("Q4_K_M/Big-Q4_K_M-00003-of-00003.gguf", 2000, Some(A)),
                file("Q6_K/Big-Q6_K-00001-of-00002.gguf", 10, Some(B)),
                file("Unpinned-Q2_K.gguf", 100, None),
            ],
        })
    }

    #[test]
    fn a_repository_lists_its_quantizations_split_ones_as_one() {
        let model = parse_model(Repo::new("unsloth/Big-GGUF").expect("repo"), &answer()).expect("parses");
        assert_eq!(model.architecture.as_deref(), Some("qwen35moe"));
        assert_eq!(model.context_length, Some(262_144));
        let shown: Vec<(&str, &str, u64)> = model
            .quantizations
            .iter()
            .map(|quantization| (quantization.label.as_str(), quantization.file.as_str(), quantization.size()))
            .collect();
        assert_eq!(
            shown,
            [
                ("UD-Q4_K_XL", "Big-UD-Q4_K_XL.gguf", 4000),
                ("Q4_K_M", "Q4_K_M/Big-Q4_K_M-00001-of-00003.gguf", 7010),
                ("Q8_0", "Big-Q8_0.gguf", 8000),
            ],
            "the projector, the imatrix, a split model missing a part and an unpinned file aren't offered"
        );
        let parts = select(&model, "Q4_K_M/Big-Q4_K_M-00002-of-00003.gguf").expect("found by any part");
        assert_eq!(parts.len(), 3);
        assert!(matches!(select(&model, "Unpinned-Q2_K.gguf"), Err(LlmError::NotOnHub(_))));
    }

    #[test]
    fn a_search_answers_with_the_repositories_it_found() {
        let answer = json!([
            {"id": "unsloth/Qwen3.6-27B-GGUF", "downloads": 229_032, "lastModified": "2026-10-01T00:00:00.000Z"},
            {"id": "not a repo", "downloads": 5},
            {"id": "ggml-org/gpt-oss-20b-GGUF", "createdAt": "2025-08-05T00:00:00.000Z"},
        ]);
        let found = parse_search(&answer);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].repo.as_str(), "unsloth/Qwen3.6-27B-GGUF");
        assert_eq!(found[0].downloads, 229_032);
        assert_eq!(found[1].updated.as_deref(), Some("2025-08-05T00:00:00.000Z"));
        assert_eq!(parse_search(&json!({"error": "x"})), []);
    }

    #[test]
    fn a_files_kind_is_read_from_its_name() {
        assert_eq!(label_of("Qwen3.6-27B-UD-Q4_K_XL"), "UD-Q4_K_XL");
        assert_eq!(label_of("Qwen3.6-35B-A3B-MXFP4_MOE"), "MXFP4_MOE");
        assert_eq!(label_of("gpt-oss-20b-MXFP4"), "MXFP4");
        assert_eq!(label_of("Huihui-Qwen3.6-35B-A3B-abliterated-ggml-model-Q4_K"), "Q4_K");
        assert_eq!(label_of("model.Q5_K_M"), "Q5_K_M");
        assert_eq!(label_of("plain"), "plain");
    }
}
