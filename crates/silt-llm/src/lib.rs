//! Local models (ADR 0066): the ones Silt offers, whether one fits this
//! machine, fetching them on consent, which model `llama-server` should
//! hold, and running it.

mod budget;
mod catalog;
mod download;
mod gguf;
mod hub;
mod installed;
mod schedule;
mod server;

use std::path::PathBuf;

pub use budget::{Budget, Device, Fit, fit, need, parse_devices};
pub use catalog::{
    CATALOG, CatalogFile, Entry, ModelId, Remote, Repo, SplitPart, checked_file, entry, local_model, local_model_in_args,
    split_part,
};
pub use download::{Check, Wanted, fetch, part_path};
pub use gguf::{GgufError, Meta, parse as parse_gguf};
pub use hub::{Hub, PinnedFile};
pub use installed::{DEFAULT_CONTEXT, Installed, Models, Plan, Progress, Source};
pub use schedule::{Schedule, Step, Ticket};
pub use server::{Launch, Server, find_server, free_port, list_devices};

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("{0:?} isn't a model name: use lowercase letters, digits, `.`, `-` and `_`")]
    BadId(String),
    #[error("{0:?} isn't a Hugging Face repository and GGUF file, such as `owner/repo:model.gguf`")]
    BadSource(String),
    #[error(transparent)]
    Gguf(#[from] GgufError),
    #[error("{} isn't a GGUF model", .0.display())]
    NotAModel(PathBuf),
    #[error("Hugging Face has no {0}")]
    NotOnHub(String),
    #[error("Hugging Face keeps no SHA-256 for {0}, so it can't be checked")]
    NotPinned(String),
    #[error("{0}")]
    Fetch(String),
    #[error("{0} didn't match its pinned size and SHA-256, so it was thrown away")]
    Corrupt(String),
    #[error("a model named {0} is already here, from somewhere else")]
    Taken(ModelId),
    #[error("{0}")]
    Start(String),
    #[error(transparent)]
    Disk(std::io::Error),
}
