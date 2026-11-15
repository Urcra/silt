//! What an agent the web UI runs shows the person: an image in the
//! conversation, or a versioned artifact beside it (ADR 0068).

use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use silt_log::{Author, ConvOp};
use silt_proto::view::{ArtifactContent, ArtifactView};
use silt_types::{BlobHash, ThreadId};

use crate::core::{Core, CoreError};
use crate::design;
use crate::protocol::{ArtifactGuide, ArtifactPut, ArtifactRead, DesignSystemView, NewArtifact};

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
        let silt_store::RunToken { thread, workspace, .. } = self
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
        let artifacts: Vec<&ArtifactView> = entries.iter().filter_map(|(_, entry)| entry.artifact.as_ref()).collect();
        let (id, version) = match new.id.as_deref() {
            Some(id) if !valid_id(id) => return Err(CoreError::Invalid(format!("{id} isn't an artifact id"))),
            Some(id) => match latest_version(artifacts.iter().copied(), id) {
                0 => return Err(CoreError::Invalid(format!("this thread has no artifact {id}"))),
                latest => (id.to_owned(), latest + 1),
            },
            None => (new_id(), 1),
        };
        let previous = artifacts
            .iter()
            .filter(|artifact| artifact.id == id)
            .max_by_key(|artifact| artifact.version)
            .map(|artifact| artifact.design_system.clone().unwrap_or_else(|| design::NONE.to_owned()));
        let styled = if kind == "html" {
            let workspace_default = self.workspace_design_system(&workspace);
            let repository = design::project_default(&checkout);
            let picked = design::project_choices(&self.data_dir).remove(&self.threads[&thread].repo);
            let defaults = design::Defaults {
                picked: picked.as_deref(),
                repository: repository.as_deref(),
                workspace: workspace_default.as_deref(),
            };
            match design::chosen(new.design_system.as_deref(), previous.as_deref(), defaults) {
                Some(name) => {
                    let system = design::find(&design::systems(Some(&checkout), &self.data_dir), name)?;
                    Some((system.name.to_string(), system.compiled()?))
                }
                None => None,
            }
        } else {
            None
        };

        // Not redacted: a data: URI or an SRI hash reads as a secret, and
        // redacting it breaks the page (ADR 0071).
        let size = bytes.len() as u64;
        let blob = self.store.blobs().put(&bytes).await?;
        let family = self.threads[&thread].family;
        let name = format!("artifact/{id}/v{version}/{}", file_name(&kind, media_type));
        self.append_conv(family, thread, Author::User, ConvOp::Attachment { session: reference.clone(), name, blob }).await?;
        let (design_system, style, dropped) = match styled {
            Some((system, compiled)) => {
                let style = self.store.blobs().put(compiled.css.as_bytes()).await?;
                let name = format!("artifact/{id}/v{version}/design-system.css");
                self.append_conv(family, thread, Author::User, ConvOp::Attachment { session: reference, name, blob: style })
                    .await?;
                (Some(system), Some(style.to_string()), compiled.dropped)
            }
            None => (None, None, Vec::new()),
        };
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
            design_system: design_system.clone(),
            style,
        };
        self.record_acp(thread, &silt_agents::acp::artifact(&view)).await?;
        Ok(ArtifactPut { id, version, design_system, dropped })
    }

    /// The design system a run reads before making an artifact: the one
    /// named, else its project's default, else its workspace's (ADR 0071).
    pub(crate) async fn artifact_guide(&self, token: &str, asked: Option<String>) -> Result<ArtifactGuide, CoreError> {
        let hash = blake3::hash(token.as_bytes());
        let silt_store::RunToken { thread, workspace, .. } = self
            .store
            .run_token(hash.as_bytes())
            .await?
            .ok_or_else(|| CoreError::Invalid("this run's token isn't known here".into()))?;
        let checkout = self.acp_run_place(thread).map(|(_, checkout)| checkout);
        let systems = design::systems(checkout.as_deref(), &self.data_dir);
        let repository = checkout.as_deref().and_then(design::project_default);
        let workspace_default = self.workspace_design_system(&workspace);
        let picked = design::project_choices(&self.data_dir).remove(&self.threads[&thread].repo);
        let defaults = design::Defaults {
            picked: picked.as_deref(),
            repository: repository.as_deref(),
            workspace: workspace_default.as_deref(),
        };
        let (name, why) = match asked.as_deref() {
            Some(asked) => (design::chosen(Some(asked), None, defaults), "asked for"),
            None => match defaults.first() {
                Some((name, why)) => (design::chosen(Some(name), None, defaults), why),
                None => (None, ""),
            },
        };
        let system = name.map(|name| design::find(&systems, name)).transpose()?;
        Ok(ArtifactGuide {
            system: system.as_ref().map(|system| system.view(true)),
            why: system.is_some().then(|| why.to_owned()),
            systems: systems.iter().map(|system| system.view(false)).collect(),
        })
    }

    /// A version of an artifact of the thread `token` was made for, the
    /// latest when `version` is none, as the agent revises it after a
    /// handoff or a fork.
    pub(crate) async fn artifact_read(&self, token: &str, id: &str, version: Option<u32>) -> Result<ArtifactRead, CoreError> {
        if !valid_id(id) {
            return Err(CoreError::Invalid(format!("{id} isn't an artifact id")));
        }
        let hash = blake3::hash(token.as_bytes());
        let silt_store::RunToken { thread, .. } = self
            .store
            .run_token(hash.as_bytes())
            .await?
            .ok_or_else(|| CoreError::Invalid("this run's token isn't known here".into()))?;
        let entries = self.history_entries(thread);
        let versions: Vec<&ArtifactView> =
            entries.iter().filter_map(|(_, entry)| entry.artifact.as_ref()).filter(|artifact| artifact.id == id).collect();
        let latest = latest_version(versions.iter().copied(), id);
        if latest == 0 {
            return Err(CoreError::Invalid(format!("this thread has no artifact {id}")));
        }
        let wanted = version.unwrap_or(latest);
        let artifact = versions
            .iter()
            .find(|artifact| artifact.version == wanted)
            .ok_or_else(|| CoreError::Invalid(format!("artifact {id} has versions 1 to {latest}, not {wanted}")))?;
        let blob: BlobHash = artifact.blob.parse().map_err(|_| CoreError::Invalid("the version's blob doesn't read".into()))?;
        let family = self.threads[&thread].family;
        let bytes = self.blob(family, blob).await?;
        let style = match artifact.style.as_deref().and_then(|style| style.parse::<BlobHash>().ok()) {
            Some(style) => Some(String::from_utf8_lossy(&self.blob(family, style).await?).into_owned()),
            None => None,
        };
        Ok(ArtifactRead {
            id: id.to_owned(),
            version: wanted,
            latest,
            title: artifact.title.clone(),
            kind: artifact.kind.clone(),
            media_type: artifact.media_type.clone(),
            design_system: artifact.design_system.clone(),
            data: BASE64.encode(bytes),
            style,
        })
    }

    /// The design system picked for each project on this machine.
    pub(crate) fn project_design_systems(&self) -> std::collections::BTreeMap<String, String> {
        design::project_choices(&self.data_dir)
    }

    /// Picks the design system a project's artifacts take on this machine,
    /// above the repository's default and the workspace's; `None` drops it.
    pub(crate) fn set_project_design_system(&self, repo: &str, name: Option<&str>) -> Result<&'static str, CoreError> {
        design::set_project_choice(&self.data_dir, repo, name)?;
        Ok("design system saved")
    }

    /// The person's design systems and the built-in ones, and a clone's own.
    pub(crate) fn design_systems(&self, clone: Option<&std::path::Path>) -> Vec<DesignSystemView> {
        design::systems(clone, &self.data_dir).iter().map(|system| system.view(false)).collect()
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
        self.history_entries(thread).iter().filter_map(|(_, entry)| entry.artifact.as_ref()).find_map(|artifact| {
            if artifact.blob == blob {
                Some(artifact.media_type.clone())
            } else {
                (artifact.style.as_deref() == Some(blob)).then(|| "text/css".to_owned())
            }
        })
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
            design_system: None,
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
            design_system: None,
            style: None,
        };
        let all = [view("aaaaaaaaaaaa", 1), view("aaaaaaaaaaaa", 2), view("bbbbbbbbbbbb", 1)];
        assert_eq!(latest_version(&all, "aaaaaaaaaaaa"), 2);
        assert_eq!(latest_version(&all, "cccccccccccc"), 0);
    }
}
