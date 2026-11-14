//! What an agent the web UI runs shows the person: an image in the
//! conversation, or a versioned artifact beside it (ADR 0066).

use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use silt_log::{Author, ConvOp};
use silt_proto::view::{ArtifactContent, ArtifactView};
use silt_types::{BlobHash, ThreadId};

use crate::core::{Core, CoreError};
use crate::protocol::{ArtifactPut, NewArtifact};

/// The largest artifact or image kept.
pub(crate) const ARTIFACT_BYTES: u64 = 10 << 20;
const TITLE_CHARS: usize = 200;
const CAPTION_CHARS: usize = 500;

/// What an artifact's kind is shown as, and whether its content is text.
fn text_media_type(kind: &str) -> Option<&'static str> {
    match kind {
        "html" => Some("text/html"),
        "svg" => Some("image/svg+xml"),
        "markdown" => Some("text/markdown"),
        "mermaid" => Some("text/vnd.mermaid"),
        _ => None,
    }
}

/// An image's type from its first bytes; SVG only as text that says so.
fn image_media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    let text = std::str::from_utf8(bytes).ok()?;
    looks_like_svg(text).then_some("image/svg+xml")
}

fn looks_like_svg(text: &str) -> bool {
    let start: String = text.trim_start_matches('\u{feff}').trim_start().chars().take(1024).collect();
    start.contains("<svg")
}

/// An artifact id as an agent passes it back: 12 lowercase hex characters.
pub(crate) fn valid_id(id: &str) -> bool {
    id.len() == 12 && id.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn new_id() -> String {
    format!("{:032x}", uuid::Uuid::new_v4().as_u128())[..12].to_owned()
}

/// The file `path` names for a run in `checkout`, if an agent may show it:
/// a regular file of at most [`ARTIFACT_BYTES`], outside every folder in
/// `denied`.
pub(crate) fn readable_file(path: &str, checkout: &Path, denied: &[PathBuf]) -> Result<PathBuf, CoreError> {
    let given = Path::new(path);
    let joined = if given.is_absolute() { given.to_owned() } else { checkout.join(given) };
    let real = silt_os::canonicalize(&joined).map_err(|_| CoreError::Invalid(format!("there is no file at {path}")))?;
    for folder in denied {
        let folder = silt_os::canonicalize(folder).unwrap_or_else(|_| folder.clone());
        if real.starts_with(&folder) {
            return Err(CoreError::Invalid(format!("{path} is in a folder agents here may not show")));
        }
    }
    let meta = std::fs::metadata(&real).map_err(|_| CoreError::Invalid(format!("there is no file at {path}")))?;
    if !meta.is_file() {
        return Err(CoreError::Invalid(format!("{path} isn't a file")));
    }
    if meta.len() > ARTIFACT_BYTES {
        return Err(CoreError::Invalid(format!("{path} is too large: an artifact is at most 10 MiB")));
    }
    Ok(real)
}

/// The content and its media type, checked against the kind asked for.
fn content_of(new: &NewArtifact, kind: &str, file: Option<&Path>) -> Result<(Vec<u8>, &'static str), CoreError> {
    let bytes = match (&new.content, file) {
        (Some(_), Some(_)) => return Err(CoreError::Invalid("give content or path, not both".into())),
        (Some(content), None) => content.clone().into_bytes(),
        (None, Some(file)) => std::fs::read(file)?,
        (None, None) => return Err(CoreError::Invalid("give the content, or the path of a file holding it".into())),
    };
    if bytes.len() as u64 > ARTIFACT_BYTES {
        return Err(CoreError::Invalid("the content is too large: an artifact is at most 10 MiB".into()));
    }
    if kind == "image" {
        let media_type =
            image_media_type(&bytes).ok_or_else(|| CoreError::Invalid("an image must be PNG, JPEG, GIF, WebP or SVG".into()))?;
        return Ok((bytes, media_type));
    }
    let media_type = text_media_type(kind)
        .ok_or_else(|| CoreError::Invalid(format!("an artifact's kind is html, svg, markdown, mermaid or image, not {kind}")))?;
    if std::str::from_utf8(&bytes).is_err() {
        return Err(CoreError::Invalid(format!("a {kind} artifact is text, and this isn't UTF-8")));
    }
    if kind == "svg" && !looks_like_svg(std::str::from_utf8(&bytes).unwrap_or_default()) {
        return Err(CoreError::Invalid("this isn't SVG".into()));
    }
    Ok((bytes, media_type))
}

/// The highest version of artifact `id` among `entries`, or 0.
fn latest_version<'a>(entries: impl IntoIterator<Item = &'a ArtifactView>, id: &str) -> u32 {
    entries.into_iter().filter(|artifact| artifact.id == id).map(|artifact| artifact.version).max().unwrap_or(0)
}

impl Core {
    /// Keeps what a run asks to show, for the thread its token was made for,
    /// and only while the daemon runs that thread's agent for the web UI.
    pub(crate) async fn put_artifact(&mut self, token: &str, new: NewArtifact) -> Result<ArtifactPut, CoreError> {
        let hash = blake3::hash(token.as_bytes());
        let (thread, workspace) = self
            .store
            .run_token(hash.as_bytes())
            .await?
            .ok_or_else(|| CoreError::Invalid("this run's token isn't known here".into()))?;
        let (session, checkout) = self
            .acp_run_place(thread)
            .ok_or_else(|| CoreError::Invalid("only an agent the web UI runs can show the person something".into()))?;
        let reference = self
            .session_mut(&session)
            .map(|session| session.reference())
            .ok_or_else(|| CoreError::Invalid("the run's session isn't recorded yet".into()))?;

        let title = new.title.trim().to_owned();
        if title.is_empty() || title.chars().count() > TITLE_CHARS {
            return Err(CoreError::Invalid(format!("a title is 1 to {TITLE_CHARS} characters")));
        }
        let caption = new.caption.as_deref().map(str::trim).filter(|caption| !caption.is_empty()).map(str::to_owned);
        if caption.as_ref().is_some_and(|caption| caption.chars().count() > CAPTION_CHARS) {
            return Err(CoreError::Invalid(format!("a caption is at most {CAPTION_CHARS} characters")));
        }
        let kind = if new.inline { "image".to_owned() } else { new.kind.trim().to_owned() };
        let file = match &new.path {
            Some(path) => Some(readable_file(path, &checkout, &self.artifact_denies(&workspace))?),
            None => None,
        };
        let (bytes, media_type) = content_of(&new, &kind, file.as_deref())?;

        let entries = self.history_entries(thread);
        let artifacts = entries.iter().filter_map(|(_, entry)| entry.artifact.as_ref());
        let (id, version) = match new.id.as_deref() {
            Some(id) if !valid_id(id) => return Err(CoreError::Invalid(format!("{id} isn't an artifact id"))),
            Some(id) => match latest_version(artifacts, id) {
                0 => return Err(CoreError::Invalid(format!("this thread has no artifact {id}"))),
                latest => (id.to_owned(), latest + 1),
            },
            None => (new_id(), 1),
        };

        let bytes = match std::str::from_utf8(&bytes) {
            Ok(text) => {
                let (redacted, found) = self.redactor.replace_found(text);
                self.learn_secrets(found);
                redacted.into_bytes()
            }
            Err(_) => bytes,
        };
        let size = bytes.len() as u64;
        let blob = self.store.blobs().put(&bytes).await?;
        let family = self.threads[&thread].family;
        let name = format!("artifact/{id}/v{version}/{}", file_name(&kind, media_type));
        self.append_conv(family, thread, Author::User, ConvOp::Attachment { session: reference, name, blob }).await?;
        let view = ArtifactView {
            id: id.clone(),
            version,
            title,
            kind,
            media_type: media_type.to_owned(),
            blob: blob.to_string(),
            size,
            inline: new.inline,
            caption,
        };
        self.record_acp(thread, &silt_agents::acp::artifact(&view)).await?;
        Ok(ArtifactPut { id, version })
    }

    /// An artifact's content, if an artifact of `thread`'s history names
    /// `blob`; no other blob is served this way.
    pub(crate) async fn artifact_content(&self, thread: &str, blob: &str) -> Result<ArtifactContent, CoreError> {
        let thread = self.find_thread(thread)?;
        let media_type = self
            .artifact_media_type(thread, blob)
            .ok_or_else(|| CoreError::Invalid(format!("this thread has no artifact {blob}")))?;
        let hash: BlobHash = blob.parse().map_err(|_| CoreError::Invalid(format!("{blob} isn't a blob")))?;
        let bytes = self.blob(self.threads[&thread].family, hash).await?;
        Ok(ArtifactContent { media_type, data: BASE64.encode(bytes) })
    }

    fn artifact_media_type(&self, thread: ThreadId, blob: &str) -> Option<String> {
        self.history_entries(thread)
            .iter()
            .filter_map(|(_, entry)| entry.artifact.as_ref())
            .find(|artifact| artifact.blob == blob)
            .map(|artifact| artifact.media_type.clone())
    }
}

/// The name a version is kept under beside the session.
fn file_name(kind: &str, media_type: &str) -> &'static str {
    match (kind, media_type) {
        ("html", _) => "index.html",
        ("svg", _) | (_, "image/svg+xml") => "image.svg",
        ("markdown", _) => "index.md",
        ("mermaid", _) => "diagram.mmd",
        (_, "image/png") => "image.png",
        (_, "image/jpeg") => "image.jpg",
        (_, "image/gif") => "image.gif",
        (_, "image/webp") => "image.webp",
        _ => "content",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new(kind: &str, content: Option<&str>) -> NewArtifact {
        NewArtifact {
            title: "t".into(),
            kind: kind.into(),
            content: content.map(str::to_owned),
            path: None,
            id: None,
            caption: None,
            inline: false,
        }
    }

    #[test]
    fn ids_are_twelve_hex_characters() {
        assert!(valid_id(&new_id()));
        assert!(valid_id("0123456789ab"));
        for bad in ["0123456789AB", "0123456789a", "../etc/pass", "0123456789abc", ""] {
            assert!(!valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn content_is_checked_against_its_kind() {
        assert_eq!(content_of(&new("html", Some("<p>hi</p>")), "html", None).expect("html").1, "text/html");
        assert_eq!(content_of(&new("svg", Some("<svg></svg>")), "svg", None).expect("svg").1, "image/svg+xml");
        assert!(content_of(&new("svg", Some("<p>no</p>")), "svg", None).is_err());
        assert!(content_of(&new("pdf", Some("x")), "pdf", None).is_err());
        assert!(content_of(&new("image", Some("not an image")), "image", None).is_err());
        assert_eq!(content_of(&new("image", Some("<svg/>")), "image", None).expect("svg image").1, "image/svg+xml");
        assert!(content_of(&new("html", None), "html", None).is_err(), "nothing to keep");
    }

    #[test]
    fn images_are_known_by_their_first_bytes() {
        assert_eq!(image_media_type(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(image_media_type(&[0xff, 0xd8, 0xff, 0xe0]), Some("image/jpeg"));
        assert_eq!(image_media_type(b"GIF89a.."), Some("image/gif"));
        assert_eq!(image_media_type(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(image_media_type(b"\0\0\0"), None);
    }

    #[test]
    fn a_path_must_be_a_small_file_outside_denied_folders() {
        let checkout = tempfile::tempdir().expect("temp dir");
        let denied = tempfile::tempdir().expect("temp dir");
        std::fs::write(checkout.path().join("chart.svg"), "<svg/>").expect("write");
        std::fs::write(denied.path().join("secret.html"), "<p>no</p>").expect("write");
        std::fs::create_dir(checkout.path().join("folder")).expect("dir");
        let denies = [denied.path().to_owned()];
        assert!(readable_file("chart.svg", checkout.path(), &denies).is_ok());
        let secret = denied.path().join("secret.html");
        assert!(readable_file(&secret.to_string_lossy(), checkout.path(), &denies).is_err());
        assert!(readable_file("folder", checkout.path(), &denies).is_err());
        assert!(readable_file("missing.png", checkout.path(), &denies).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&secret, checkout.path().join("link.html")).expect("link");
            assert!(readable_file("link.html", checkout.path(), &denies).is_err(), "a link into a denied folder");
        }
    }

    #[test]
    fn versions_count_up_per_id() {
        let view = |id: &str, version| ArtifactView {
            id: id.into(),
            version,
            title: "t".into(),
            kind: "html".into(),
            media_type: "text/html".into(),
            blob: String::new(),
            size: 0,
            inline: false,
            caption: None,
        };
        let all = [view("aaaaaaaaaaaa", 1), view("aaaaaaaaaaaa", 2), view("bbbbbbbbbbbb", 1)];
        assert_eq!(latest_version(&all, "aaaaaaaaaaaa"), 2);
        assert_eq!(latest_version(&all, "cccccccccccc"), 0);
    }
}
