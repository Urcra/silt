//! What the server knows of a blob besides its bytes (ADR 0121): the machine
//! whose push first named it, why it won't keep it, and whether its last
//! upload arrived damaged. Kept beside the blobs, one small file each, so
//! every machine reading the thread hears of it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use silt_log::{ConvOp, Delta, Op};
use silt_types::BlobHash;

use crate::{Caller, Server};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BlobNote {
    /// The device that pushed the first delta naming it, by its name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) from: Option<String>,
    /// Why the server refused it, over `max_file_mib`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) refused: Option<String>,
    /// Its last upload didn't match its hash, so it's being sent again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) damaged: bool,
}

impl BlobNote {
    /// What a blob's state says besides how much of it has arrived.
    pub(crate) fn state(self, stored: bool, received: u64, size: Option<u64>) -> silt_proto::BlobState {
        silt_proto::BlobState { stored, received, size, refused: self.refused, damaged: self.damaged && !stored, from: self.from }
    }
}

impl Server {
    pub(crate) fn blob_notes_dir(&self) -> PathBuf {
        self.shared.data_dir.join("blob-notes")
    }

    fn blob_note_path(&self, hash: BlobHash) -> PathBuf {
        self.blob_notes_dir().join(format!("{hash}.json"))
    }

    pub(crate) async fn blob_note(&self, hash: BlobHash) -> BlobNote {
        let read = tokio::fs::read(self.blob_note_path(hash)).await;
        read.ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
    }

    /// Changes the blob's note; a note left empty is removed.
    pub(crate) async fn change_blob_note(&self, hash: BlobHash, change: impl FnOnce(&mut BlobNote)) {
        let mut note = self.blob_note(hash).await;
        let before = note.clone();
        change(&mut note);
        if note == before {
            return;
        }
        let path = self.blob_note_path(hash);
        let written = if note == BlobNote::default() {
            tokio::fs::remove_file(&path).await.or_else(|error| match error.kind() {
                std::io::ErrorKind::NotFound => Ok(()),
                _ => Err(error),
            })
        } else {
            async {
                tokio::fs::create_dir_all(self.blob_notes_dir()).await?;
                tokio::fs::write(&path, serde_json::to_vec(&note).unwrap_or_default()).await
            }
            .await
        };
        if let Err(error) = written {
            tracing::warn!(%hash, %error, "could not keep a blob's note");
        }
    }

    /// Notes which device named the files a conversation's new deltas
    /// attach that the server doesn't have yet, so others can say where
    /// they're coming from.
    pub(crate) async fn note_attachments(&self, deltas: &[Delta], caller: Caller) {
        let waiting: Vec<BlobHash> = deltas
            .iter()
            .filter_map(|delta| match &delta.op {
                Op::Conv(ConvOp::Attachment { blob, .. }) => Some(*blob),
                _ => None,
            })
            .filter(|blob| !self.shared.blobs.contains(*blob))
            .collect();
        if waiting.is_empty() {
            return;
        }
        let name = match crate::control::device_name(self, caller.device).await {
            Ok(name) => name,
            Err(_) => return,
        };
        for blob in waiting {
            let name = name.clone();
            self.change_blob_note(blob, |note| {
                note.from.get_or_insert(name);
            })
            .await;
        }
    }
}
