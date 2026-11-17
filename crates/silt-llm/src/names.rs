//! The names a model has: on this machine, and on Hugging Face.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::LlmError;

/// A model's name on this machine. It is also its directory under the
/// models dir, so it holds only what a file name safely can.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ModelId(String);

const MOST_ID_CHARS: usize = 80;

impl ModelId {
    pub fn new(id: impl Into<String>) -> Result<Self, LlmError> {
        let id = id.into();
        let starts_well = id.bytes().next().is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit());
        let plain =
            id.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_'));
        // `<id>.partial` is a download's folder, so no model is named that.
        let ok = starts_well && plain && id.len() <= MOST_ID_CHARS && !id.contains("..") && !id.ends_with(".partial");
        if ok { Ok(Self(id)) } else { Err(LlmError::BadId(id)) }
    }

    /// The name of a GGUF file the person brings: its stem, lowercased, with
    /// each run of other characters made one `-`, and a split file's part
    /// number dropped.
    pub fn for_file(file: &str) -> Result<Self, LlmError> {
        let name = file.rsplit('/').next().unwrap_or(file);
        let stem = name.strip_suffix(".gguf").unwrap_or(name);
        let stem = split_part(stem).map_or(stem, |part| part.base);
        let mut id = String::new();
        for char in stem.chars().map(|char| char.to_ascii_lowercase()) {
            if char.is_ascii_lowercase() || char.is_ascii_digit() || char == '.' || char == '_' {
                id.push(char);
            } else if !id.ends_with('-') {
                id.push('-');
            }
        }
        let id: String = id.replace("..", ".").trim_matches(['-', '.', '_']).chars().take(MOST_ID_CHARS).collect();
        Self::new(id.trim_end_matches(['-', '.', '_']))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ModelId {
    type Error = LlmError;

    fn try_from(id: String) -> Result<Self, Self::Error> {
        Self::new(id)
    }
}

impl From<ModelId> for String {
    fn from(id: ModelId) -> Self {
        id.0
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The local model a value names: `local:<id>`.
pub fn local_model(value: &str) -> Option<ModelId> {
    ModelId::new(value.strip_prefix("local:")?).ok()
}

/// `--model local:<id>` or `--model=local:<id>` among an agent's arguments:
/// the model, and the arguments without the flag.
pub fn local_model_in_args(args: &[String]) -> Option<(ModelId, Vec<String>)> {
    let mut found = None;
    let mut rest = Vec::with_capacity(args.len());
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let value = match arg.strip_prefix("--model=") {
            Some(value) => Some(value.to_owned()),
            None if arg == "--model" => iter.next().cloned(),
            None => {
                rest.push(arg.clone());
                continue;
            }
        };
        match value.as_deref().and_then(local_model) {
            Some(model) if found.is_none() => found = Some(model),
            _ => {
                rest.push(arg.clone());
                if arg == "--model"
                    && let Some(value) = value
                {
                    rest.push(value);
                }
            }
        }
    }
    found.map(|model| (model, rest))
}

/// A Hugging Face repository, `owner/name`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Repo(String);

impl Repo {
    pub fn new(repo: impl Into<String>) -> Result<Self, LlmError> {
        let repo = repo.into();
        let parts: Vec<&str> = repo.split('/').collect();
        if parts.len() == 2 && parts.iter().all(|part| is_plain_name(part)) {
            Ok(Self(repo))
        } else {
            Err(LlmError::BadSource(repo))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Repo {
    type Error = LlmError;

    fn try_from(repo: String) -> Result<Self, Self::Error> {
        Self::new(repo)
    }
}

impl From<Repo> for String {
    fn from(repo: Repo) -> Self {
        repo.0
    }
}

impl fmt::Display for Repo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name != ".."
        && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

/// A GGUF file in a repository: a relative path of plain names, so it can
/// name a file under the model's directory too.
pub fn checked_file(file: &str) -> Result<&str, LlmError> {
    let plain = file.split('/').all(is_plain_name);
    if plain && file.ends_with(".gguf") { Ok(file) } else { Err(LlmError::BadSource(file.to_owned())) }
}

/// A model on Hugging Face as the person names one: `owner/repo:file.gguf`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remote {
    pub repo: Repo,
    pub file: String,
}

impl Remote {
    pub fn parse(source: &str) -> Result<Self, LlmError> {
        let (repo, file) = source.split_once(':').ok_or_else(|| LlmError::BadSource(source.to_owned()))?;
        Ok(Self { repo: Repo::new(repo)?, file: checked_file(file)?.to_owned() })
    }

    /// Where Hugging Face serves `file` of this repository.
    pub fn url(&self, file: &str) -> String {
        format!("https://huggingface.co/{}/resolve/main/{file}", self.repo)
    }
}

/// One part of a model split across files: `name-00001-of-00003`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitPart<'a> {
    pub base: &'a str,
    pub index: u32,
    pub count: u32,
}

/// The part a file's stem names, if it's one of a split model's.
pub fn split_part(stem: &str) -> Option<SplitPart<'_>> {
    let (rest, count) = stem.rsplit_once("-of-")?;
    let (base, index) = rest.rsplit_once('-')?;
    let numeric = |part: &str| part.len() == 5 && part.bytes().all(|byte| byte.is_ascii_digit());
    if !numeric(index) || !numeric(count) {
        return None;
    }
    let (index, count) = (index.parse().ok()?, count.parse().ok()?);
    (1..=count).contains(&index).then_some(SplitPart { base, index, count })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_ids_hold_only_what_a_file_name_safely_can() {
        assert!(ModelId::new("gpt-oss-20b").is_ok());
        for bad in ["", "-x", ".x", "a/b", "a..b", "Upper", "a b", "x.partial", &"a".repeat(81)] {
            assert!(ModelId::new(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_file_brought_by_the_person_gets_a_plain_name() {
        let id = |file| ModelId::for_file(file).map(String::from).ok();
        assert_eq!(id("Qwen3-Coder-30B-A3B-Instruct-Q4_K_M.gguf").as_deref(), Some("qwen3-coder-30b-a3b-instruct-q4_k_m"));
        assert_eq!(id("dir/Model (abliterated) v2.gguf").as_deref(), Some("model-abliterated-v2"));
        assert_eq!(id("big-00001-of-00003.gguf").as_deref(), Some("big"));
        assert_eq!(id("..gguf"), None);
    }

    #[test]
    fn a_remote_names_a_repository_and_a_gguf_in_it() {
        let remote = Remote::parse("unsloth/Some-GGUF:Q4/model.gguf").expect("parses");
        assert_eq!(remote.repo.as_str(), "unsloth/Some-GGUF");
        assert_eq!(remote.url(&remote.file), "https://huggingface.co/unsloth/Some-GGUF/resolve/main/Q4/model.gguf");
        for bad in ["unsloth/x", "x:model.gguf", "a/b/c:m.gguf", "a/b:../m.gguf", "a/b:m.bin", "a/b:.hidden.gguf", "../b:m.gguf"]
        {
            assert!(Remote::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_local_model_flag_is_taken_out_of_the_arguments() {
        let args = |list: &[&str]| list.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
        let (model, rest) = local_model_in_args(&args(&["-p", "hi", "--model", "local:tiny"])).expect("named");
        assert_eq!((model.as_str(), rest), ("tiny", args(&["-p", "hi"])));
        let (model, rest) = local_model_in_args(&args(&["--model=local:gpt-oss-20b", "--verbose"])).expect("named");
        assert_eq!((model.as_str(), rest), ("gpt-oss-20b", args(&["--verbose"])));
        assert_eq!(local_model_in_args(&args(&["--model", "sonnet"])), None);
        assert_eq!(local_model_in_args(&args(&["--model", "local:Bad Name"])), None);
        assert_eq!(local_model("local:tiny").map(String::from).as_deref(), Some("tiny"));
    }

    #[test]
    fn split_parts_are_numbered_from_one() {
        assert_eq!(split_part("m-00002-of-00003"), Some(SplitPart { base: "m", index: 2, count: 3 }));
        assert_eq!(split_part("m-00004-of-00003"), None);
        assert_eq!(split_part("m-2-of-3"), None);
        assert_eq!(split_part("model"), None);
    }
}
