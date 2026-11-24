//! Images and videos (ADR 0121): where a thread's blob is for the web UI to
//! read by range, and the files a prompt's sender uploaded first.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use silt_log::{Author, ConvOp, SessionRef};
use silt_proto::view::{ImageView, PreviewView, StoredMedia};
use silt_types::{BlobHash, ThreadId};

use crate::core::{Core, CoreError};
use crate::remote::RemoteConfig;

/// Where the web UI reads a blob: this machine's copy, else the server
/// that has it, which it asks by range.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BlobSource {
    pub(crate) path: Option<PathBuf>,
    pub(crate) remote: Option<RemoteConfig>,
    /// Why the server won't keep it, when it refused it from here.
    #[serde(default)]
    pub(crate) refused: Option<String>,
}

/// Where the files a person sent before their prompt wait for it.
pub(crate) fn uploads_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("uploads")
}

/// An uploaded file, by the name the upload answered with: its hash.
pub(crate) fn uploaded(data_dir: &Path, upload: &str) -> Result<PathBuf, CoreError> {
    let valid = upload.len() == 64 && upload.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    let path = uploads_dir(data_dir).join(upload);
    if !valid || !path.is_file() {
        return Err(CoreError::Invalid("an attached file didn't finish uploading; attach it again".into()));
    }
    Ok(path)
}

/// Uploads left a day without a prompt naming them.
pub(crate) fn drop_stale_uploads(data_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(uploads_dir(data_dir)) else {
        return;
    };
    let day = std::time::Duration::from_secs(24 * 60 * 60);
    for entry in entries.flatten() {
        let old = entry.metadata().and_then(|meta| meta.modified()).is_ok_and(|at| at.elapsed().is_ok_and(|age| age > day));
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Drops the previews made for file pages whose image isn't kept any more.
pub(crate) fn prune_previews(data_dir: &Path, blobs: &silt_store::BlobStore) {
    let Ok(entries) = std::fs::read_dir(data_dir.join("previews")) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let original = name.split('-').next().and_then(|hash| hash.parse::<BlobHash>().ok());
        if original.is_none_or(|hash| !blobs.contains(hash)) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// A file a prompt's sender uploaded first, rather than sent in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Uploaded {
    pub(crate) name: String,
    pub(crate) mime: String,
    pub(crate) path: PathBuf,
}

/// The most of an uploaded file that isn't an image or a video that's
/// read, redacted and kept with the conversation; a larger one goes only to
/// the agent, as a file (ADR 0121).
const KEPT_FILE_BYTES: u64 = 32 << 20;

/// An image larger than this goes to the agent as its preview.
const AGENT_IMAGE_BYTES: u64 = 4 << 20;

/// What a prompt's uploaded files become.
#[derive(Debug, Default)]
pub(crate) struct Taken {
    /// Files read whole, which go on as files sent in the prompt do.
    pub(crate) read: Vec<silt_acp::Attachment>,
    /// Images as the agent gets them, kept already.
    pub(crate) for_agent: Vec<silt_acp::Attachment>,
    /// The images and videos, kept with their previews, for the history.
    pub(crate) media: Vec<ImageView>,
    /// The names they're kept under beside the session.
    pub(crate) names: Vec<String>,
    /// Files the agent reads from disk, with where they are.
    pub(crate) saved: Vec<(String, String, PathBuf)>,
}

impl Core {
    /// Where the web UI reads blob `blob` of `thread`, if the thread names it:
    /// an artifact's version or preview, a prompt's image or video, or a file
    /// its file page showed.
    pub(crate) fn blob_source(&self, thread: &str, blob: &str) -> Result<BlobSource, CoreError> {
        let thread = self.find_thread(thread)?;
        let hash: BlobHash = blob.parse().map_err(|_| CoreError::Invalid(format!("{blob} isn't a blob")))?;
        if !self.names_blob(thread, hash) {
            return Err(CoreError::Invalid(format!("this thread has no file {blob}")));
        }
        Ok(BlobSource {
            path: self.store.blobs().file(hash),
            remote: self.remote_for(thread).map(|remote| remote.config.clone()),
            refused: self.store.blobs().refused(hash),
        })
    }

    fn names_blob(&self, thread: ThreadId, hash: BlobHash) -> bool {
        if self.media_files.lock().expect("lock").contains(&(thread, hash)) {
            return true;
        }
        let blob = hash.to_string();
        self.history_entries(thread).iter().any(|(_, entry)| {
            entry.artifact.as_ref().is_some_and(|artifact| artifact.media_type_of(&blob).is_some())
                || entry
                    .image
                    .as_ref()
                    .and_then(|image| image.stored.as_ref())
                    .is_some_and(|stored| stored.media_type_of(&blob, "").is_some())
        })
    }

    /// Notes that the thread's file page shows blob `hash`, which its page
    /// then reads.
    pub(crate) fn shows_file(&self, thread: ThreadId, hash: BlobHash) {
        self.media_files.lock().expect("lock").insert((thread, hash));
    }

    /// Takes the files a prompt's sender uploaded: an image or a video kept
    /// as it is with an image's previews, other files read for the prompt
    /// when small enough and given to the agent as files when not.
    pub(crate) async fn take_uploads(
        &mut self,
        thread: ThreadId,
        session: Option<SessionRef>,
        uploads: Vec<Uploaded>,
    ) -> Result<Taken, CoreError> {
        let mut taken = Taken::default();
        let family = self.threads[&thread].family;
        let dir = crate::acp::attachments_dir(&self.data_dir, thread);
        for upload in uploads {
            let found = silt_media::sniff(&crate::artifacts::head_of(&upload.path)?);
            let size = std::fs::metadata(&upload.path)?.len();
            let media = found.filter(|(media_type, _)| *media_type != "image/svg+xml");
            let Some((media_type, kind)) = media else {
                if size <= KEPT_FILE_BYTES {
                    let data = tokio::fs::read(&upload.path).await?;
                    let _ = tokio::fs::remove_file(&upload.path).await;
                    taken.read.push(silt_acp::Attachment { name: upload.name, mime: upload.mime, data });
                } else {
                    // Too large to read and redact: it goes to the agent only.
                    let to = saved_path(&dir, &upload)?;
                    move_file(&upload.path, &to).await?;
                    taken.saved.push((upload.name, upload.mime, to));
                }
                continue;
            };
            let (width, height, previews) = match kind {
                silt_media::Kind::Image => self.previews_of(&upload.path).await?,
                silt_media::Kind::Video => (None, None, Vec::new()),
            };
            let (blob, size) = self.store.blobs().put_file(&upload.path, true).await?;
            let name = crate::acp::attachment_name(&blob.to_string(), &upload.name);
            if let Some(session) = &session {
                let op = ConvOp::Attachment { session: session.clone(), name: name.clone(), blob };
                self.append_conv(family, thread, Author::User, op).await?;
                for preview in &previews {
                    let preview_blob = preview.blob.parse().map_err(|_| CoreError::Invalid("a preview isn't kept".into()))?;
                    let preview_name = format!(
                        "{name}.preview-{}.{}",
                        preview.width.max(preview.height),
                        crate::artifacts::extension(&preview.media_type)
                    );
                    let op = ConvOp::Attachment { session: session.clone(), name: preview_name, blob: preview_blob };
                    self.append_conv(family, thread, Author::User, op).await?;
                }
            }
            taken.names.push(name);
            match kind {
                silt_media::Kind::Image => {
                    if let Some(agent) = self.image_for_agent(blob, size, media_type, &previews).await? {
                        taken.for_agent.push(silt_acp::Attachment { name: upload.name.clone(), ..agent });
                    }
                }
                silt_media::Kind::Video => {
                    let to = saved_path(&dir, &upload)?;
                    if let Some(from) = self.store.blobs().file(blob) {
                        copy_for_agent(&from, &to).await?;
                        taken.saved.push((upload.name.clone(), media_type.to_owned(), to));
                    }
                }
            }
            let stored = StoredMedia { blob: blob.to_string(), size, name: Some(upload.name), width, height, previews };
            taken.media.push(ImageView { media_type: media_type.to_owned(), data: String::new(), stored: Some(stored) });
        }
        Ok(taken)
    }

    /// What an image goes to the agent as: its 2048 px preview when it has
    /// one or is large, as models take only so much, else itself.
    async fn image_for_agent(
        &self,
        blob: BlobHash,
        size: u64,
        media_type: &str,
        previews: &[PreviewView],
    ) -> Result<Option<silt_acp::Attachment>, CoreError> {
        if let Some(largest) = previews.last() {
            let hash: BlobHash = largest.blob.parse().map_err(|_| CoreError::Invalid("a preview isn't kept".into()))?;
            let data = self.store.blobs().get(hash).await?;
            return Ok(Some(silt_acp::Attachment { name: String::new(), mime: largest.media_type.clone(), data }));
        }
        if size > AGENT_IMAGE_BYTES {
            return Ok(None);
        }
        let data = self.store.blobs().get(blob).await?;
        Ok(Some(silt_acp::Attachment { name: String::new(), mime: media_type.to_owned(), data }))
    }
}

/// Where an uploaded file the agent reads from disk goes.
fn saved_path(dir: &Path, upload: &Uploaded) -> Result<PathBuf, CoreError> {
    let hash = upload.path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(dir.join(crate::acp::attachment_name(&hash, &upload.name)))
}

async fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(dir) = to.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    if tokio::fs::rename(from, to).await.is_err() {
        tokio::fs::copy(from, to).await?;
        tokio::fs::remove_file(from).await?;
    }
    Ok(())
}

/// A copy of a kept blob for the agent. Not a link: an agent writing to its
/// copy must not change what the conversation keeps.
async fn copy_for_agent(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(dir) = to.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    if !tokio::fs::try_exists(to).await? {
        tokio::fs::copy(from, to).await?;
    }
    Ok(())
}

/// The blobs a file page shows, by thread.
pub(crate) type MediaFiles = std::sync::Mutex<HashSet<(ThreadId, BlobHash)>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn previews_of_images_no_longer_kept_are_dropped() {
        let data = tempfile::tempdir().expect("temp dir");
        let blobs = silt_store::BlobStore::new(data.path().join("blobs"));
        let kept = blobs.put(b"an image").await.expect("put");
        let gone = silt_store::BlobStore::hash(b"an image erased since");
        let previews = data.path().join("previews");
        std::fs::create_dir_all(&previews).expect("dir");
        for name in [format!("{kept}-960"), format!("{gone}-2048"), "stray".to_owned()] {
            std::fs::write(previews.join(name), b"jpeg").expect("write");
        }
        prune_previews(data.path(), &blobs);
        let left: Vec<String> = std::fs::read_dir(&previews)
            .expect("dir")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, [format!("{kept}-960")]);
    }
}
