//! What an agent the web UI runs shows the person: an image in the
//! conversation, or a versioned artifact beside it (ADR 0068).

use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use silt_log::{Author, ConvOp, Op, SessionFile, SessionRef};
use silt_proto::view::{ArtifactContent, ArtifactView};
use silt_types::{BlobHash, ThreadId};

use crate::core::{Core, CoreError};
use crate::design;
use crate::protocol::{ArtifactGuide, ArtifactPut, ArtifactRead, DesignSystemView, NewArtifact};

/// The largest artifact or image kept.
pub(crate) const ARTIFACT_BYTES: u64 = silt_proto::artifact::ARTIFACT_BYTES;
const TITLE_CHARS: usize = 200;
const CAPTION_CHARS: usize = 500;

/// What an artifact's kind is shown as, and whether its content is text.
fn text_media_type(kind: &str) -> Option<&'static str> {
    match kind {
        "html" => Some("text/html"),
        "svg" => Some("image/svg+xml"),
        "markdown" => Some("text/markdown"),
        "mermaid" => Some("text/vnd.mermaid"),
        kind if silt_proto::artifact_types::is_type(kind) => Some("application/json"),
        "doc" => Some("text/markdown"),
        "sheet" | "whiteboard" => Some("application/json"),
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
    let media_type = text_media_type(kind).ok_or_else(|| {
        CoreError::Invalid(format!(
            "an artifact's kind is html, svg, markdown, mermaid, image, slides, canvas or data, not {kind}"
        ))
    })?;
    if std::str::from_utf8(&bytes).is_err() {
        return Err(CoreError::Invalid(format!("a {kind} artifact is text, and this isn't UTF-8")));
    }
    if kind == "svg" && !looks_like_svg(std::str::from_utf8(&bytes).unwrap_or_default()) {
        return Err(CoreError::Invalid("this isn't SVG".into()));
    }
    silt_proto::artifact_types::validate(kind, std::str::from_utf8(&bytes).unwrap_or_default()).map_err(CoreError::Invalid)?;
    Ok((bytes, media_type))
}

/// Who a version is recorded for: the run that made it, or the person a
/// page saved itself for.
enum Recorded {
    ByRun,
    ByPage(String),
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
        self.put_version(thread, &workspace, reference, Some(checkout), new, Recorded::ByRun).await
    }

    /// A version a page saved itself as, for this machine's person (ADR 0072),
    /// recorded in the session that recorded the artifact's latest version.
    pub(crate) async fn publish_artifact(
        &mut self,
        thread: &str,
        id: &str,
        html: String,
        base: Option<u32>,
    ) -> Result<ArtifactPut, CoreError> {
        let thread = self.find_thread(thread)?;
        if !valid_id(id) {
            return Err(CoreError::Invalid(format!("{id} isn't an artifact id")));
        }
        let entries = self.history_entries(thread);
        let latest = entries
            .iter()
            .filter_map(|(_, entry)| entry.artifact.as_ref())
            .filter(|artifact| artifact.id == id)
            .max_by_key(|artifact| artifact.version)
            .cloned()
            .ok_or_else(|| CoreError::Invalid(format!("this thread has no artifact {id}")))?;
        if latest.kind != "html" {
            return Err(CoreError::Invalid("only an HTML artifact's page saves itself".into()));
        }
        if !latest.capabilities.as_ref().is_some_and(|declared| declared.contains_key("artifact")) {
            return Err(CoreError::Invalid(format!("artifact {id} doesn't declare artifact")));
        }
        if base.is_some_and(|base| base != latest.version) {
            return Err(CoreError::Invalid(format!("conflict: version {} is the latest", latest.version)));
        }
        if !self.may_write(thread) {
            return Err(CoreError::Invalid("only the thread's owner and collaborators save its artifacts' pages".into()));
        }
        let reference = self
            .artifact_session(thread, id)
            .ok_or_else(|| CoreError::Invalid(format!("no session recorded artifact {id} here")))?;
        let by = match self.own_user(thread) {
            user if user.is_empty() => "you".to_owned(),
            user => user,
        };
        let new = NewArtifact {
            title: latest.title.clone(),
            kind: "html".into(),
            content: Some(html),
            path: None,
            id: Some(id.to_owned()),
            caption: None,
            inline: false,
            design_system: None,
            edits: Vec::new(),
            note: Some(format!("Saved by the page, for {by}")),
            ..NewArtifact::default()
        };
        let checkout = Some(self.threads[&thread].repo_root.clone()).filter(|root| !root.as_os_str().is_empty());
        self.put_version(thread, "", reference, checkout, new, Recorded::ByPage(by)).await
    }

    /// Keeps a doc, a sheet or a whiteboard as it stands as its next
    /// version, from the person's Save version, with their note (ADR 0077).
    pub(crate) async fn save_edited_version(
        &mut self,
        thread: &str,
        id: &str,
        note: Option<String>,
    ) -> Result<ArtifactPut, CoreError> {
        let thread = self.find_thread(thread)?;
        if !valid_id(id) {
            return Err(CoreError::Invalid(format!("{id} isn't an artifact id")));
        }
        let latest = self
            .history_entries(thread)
            .iter()
            .filter_map(|(_, entry)| entry.artifact.as_ref())
            .filter(|artifact| artifact.id == id)
            .max_by_key(|artifact| artifact.version)
            .cloned()
            .ok_or_else(|| CoreError::Invalid(format!("this thread has no artifact {id}")))?;
        if !silt_proto::edited::is_edited(&latest.kind) {
            return Err(CoreError::Invalid("only a doc, a sheet or a whiteboard is saved as it stands".into()));
        }
        if !self.may_write(thread) {
            return Err(CoreError::Invalid("only the thread's owner and collaborators save its artifacts' versions".into()));
        }
        let note = note.map(|note| note.trim().chars().take(silt_proto::artifact::NOTE_CHARS).collect::<String>());
        let reference = self
            .artifact_session(thread, id)
            .ok_or_else(|| CoreError::Invalid(format!("no session recorded artifact {id} here")))?;
        let by = match self.own_user(thread) {
            user if user.is_empty() => "you".to_owned(),
            user => user,
        };
        let new = NewArtifact {
            title: latest.title.clone(),
            kind: latest.kind.clone(),
            id: Some(id.to_owned()),
            note: Some(note.filter(|note| !note.is_empty()).unwrap_or_else(|| format!("Saved by {by}"))),
            ..NewArtifact::default()
        };
        self.put_version(thread, "", reference, None, new, Recorded::ByPage(by)).await
    }

    /// A version's files: the last version's, with `changes` and `edited`
    /// made, each new one kept as a blob and marked as added. Only an HTML
    /// page has files.
    #[expect(clippy::too_many_arguments, reason = "the parts of one version")]
    async fn version_files(
        &mut self,
        mut kept: std::collections::BTreeMap<String, silt_proto::artifact::ArtifactFile>,
        changes: std::collections::BTreeMap<String, Option<crate::protocol::NewFile>>,
        edited: std::collections::BTreeMap<String, String>,
        kind: &str,
        checkout: Option<&Path>,
        workspace: &str,
        page_bytes: u64,
    ) -> Result<Vec<(String, silt_proto::artifact::ArtifactFile, bool)>, CoreError> {
        use silt_proto::artifact::{ArtifactFile, FILE_BYTES, FILES, VERSION_BYTES, file_media_type, file_path_ok};
        if (!changes.is_empty() || !edited.is_empty()) && !matches!(kind, "html" | "slides" | "canvas") {
            return Err(CoreError::Invalid("only an HTML page, a deck or a canvas has files beside it".into()));
        }
        let mut added = std::collections::BTreeSet::new();
        let put = |bytes: Vec<u8>, path: &str| -> Result<(Vec<u8>, &'static str), CoreError> {
            if !file_path_ok(path) {
                return Err(CoreError::Invalid(format!("{path} isn't a path a version's file may have")));
            }
            let media_type =
                file_media_type(path).ok_or_else(|| CoreError::Invalid(format!("{path}: a frame has no use for that type")))?;
            if bytes.len() as u64 > FILE_BYTES {
                return Err(CoreError::Invalid(format!("{path} is over {} MiB", FILE_BYTES >> 20)));
            }
            Ok((bytes, media_type))
        };
        let mut incoming: Vec<(String, Vec<u8>)> = Vec::new();
        for (path, change) in changes {
            let Some(file) = change else {
                kept.remove(&path);
                continue;
            };
            let bytes = match (file.content, file.base64, file.path) {
                (Some(text), None, None) => text.into_bytes(),
                (None, Some(encoded), None) => BASE64
                    .decode(encoded.trim())
                    .map_err(|_| CoreError::Invalid(format!("{path}: base64 that doesn't decode")))?,
                (None, None, Some(from)) => {
                    let checkout =
                        checkout.ok_or_else(|| CoreError::Invalid(format!("{path}: no checkout to read {from} from")))?;
                    let real = readable_file(&from, checkout, &self.artifact_denies(workspace))?;
                    std::fs::read(&real).map_err(|_| CoreError::Invalid(format!("{path}: couldn't read {from}")))?
                }
                _ => return Err(CoreError::Invalid(format!("{path}: give content, base64 or a path"))),
            };
            incoming.push((path, bytes));
        }
        incoming.extend(edited.into_iter().map(|(path, text)| (path, text.into_bytes())));
        for (path, bytes) in incoming {
            let (bytes, media_type) = put(bytes, &path)?;
            let blob = self.store.blobs().put(&bytes).await?;
            kept.insert(
                path.clone(),
                ArtifactFile { blob: blob.to_string(), size: bytes.len() as u64, media_type: media_type.to_owned() },
            );
            added.insert(path);
        }
        if kept.len() > FILES {
            return Err(CoreError::Invalid(format!("a version has at most {FILES} files")));
        }
        let total: u64 = page_bytes + kept.values().map(|file| file.size).sum::<u64>();
        if total > VERSION_BYTES {
            return Err(CoreError::Invalid(format!(
                "a version's page and files are at most {} MiB together",
                VERSION_BYTES >> 20
            )));
        }
        Ok(kept
            .into_iter()
            .map(|(path, file)| {
                let new = added.contains(&path);
                (path, file, new)
            })
            .collect())
    }

    /// A file of `id`'s version (the latest when none) in `thread`: its
    /// type and its bytes.
    pub(crate) async fn artifact_file(
        &self,
        thread: &str,
        id: &str,
        version: Option<u32>,
        path: &str,
    ) -> Result<silt_proto::view::ArtifactContent, CoreError> {
        let thread = self.find_thread(thread)?;
        let entries = self.history_entries(thread);
        let view = entries
            .iter()
            .filter_map(|(_, entry)| entry.artifact.as_ref())
            .filter(|artifact| artifact.id == id && version.is_none_or(|wanted| wanted == artifact.version))
            .max_by_key(|artifact| artifact.version)
            .ok_or_else(|| CoreError::Invalid(format!("no artifact {id} here")))?;
        let file = view
            .files
            .as_ref()
            .and_then(|files| files.get(path))
            .ok_or_else(|| CoreError::Invalid(format!("version {} has no file {path}", view.version)))?;
        let hash = file.blob.parse().map_err(|_| CoreError::Invalid(format!("{path} isn't kept here")))?;
        let bytes = self.store.blobs().get(hash).await?;
        Ok(silt_proto::view::ArtifactContent { media_type: file.media_type.clone(), data: BASE64.encode(bytes) })
    }

    /// A doc's text or a sheet's typed cells as they stand, for a version.
    async fn edited_snapshot(&mut self, thread: ThreadId, id: &str, kind: &str) -> Result<String, CoreError> {
        let thread_id = thread.to_string();
        if kind == "doc" {
            let read = self.doc_data(&thread_id, id, silt_proto::edited::DocRequest::Read).await?;
            return Ok(read["text"].as_str().unwrap_or_default().to_owned());
        }
        if kind == "whiteboard" {
            let items =
                self.page_data(&thread_id, id, silt_proto::page::PageRequest::List { collection: "items".into() }, None).await?;
            let items: Vec<serde_json::Value> = items.as_array().into_iter().flatten().map(|doc| doc["data"].clone()).collect();
            return Ok(serde_json::json!({ "items": items }).to_string());
        }
        // The first sheet as `rows`, as a one-sheet artifact gives it, and every sheet under `sheets`.
        let mut sheets = Vec::new();
        let mut count = 1;
        let mut sheet = 0;
        while sheet < count {
            let read = self.sheet_data(&thread_id, id, silt_proto::edited::SheetRequest::Read { sheet, range: None }).await?;
            count = read["sheets"].as_u64().map_or(1, |count| count as u16);
            sheets.push(serde_json::json!({ "rows": read["raw"] }));
            sheet += 1;
        }
        Ok(silt_proto::edited::sheet_content(sheets))
    }

    /// Seeds a new doc's text or a new sheet's cells from its first version.
    async fn seed_edited(&mut self, thread: ThreadId, id: &str, kind: &str, content: &str) -> Result<(), CoreError> {
        let thread_id = thread.to_string();
        if kind == "doc" {
            if !content.is_empty() {
                let edit = silt_proto::edited::DocEdit::Splice { pos: 0, delete: 0, insert: content.to_owned() };
                self.doc_data(&thread_id, id, silt_proto::edited::DocRequest::Edit { edits: vec![edit], base: None }).await?;
            }
            return Ok(());
        }
        if kind == "whiteboard" {
            let board: serde_json::Value = serde_json::from_str(content).unwrap_or_default();
            let writes: Vec<serde_json::Value> = board["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|item| {
                    Some(serde_json::json!({"op": "set", "path": format!("items/{}", item["id"].as_str()?), "value": item}))
                })
                .collect();
            if !writes.is_empty() {
                let request: silt_proto::page::PageRequest =
                    serde_json::from_value(serde_json::json!({"op": "batch", "writes": writes}))
                        .map_err(|error| CoreError::Invalid(error.to_string()))?;
                self.page_data(&thread_id, id, request, None).await?;
            }
            return Ok(());
        }
        for (sheet, cells) in silt_proto::edited::seeded_sheets(content).into_iter().enumerate() {
            let sheet = u16::try_from(sheet).unwrap_or(u16::MAX);
            self.sheet_data(&thread_id, id, silt_proto::edited::SheetRequest::Set { sheet, cells, ids: Default::default() })
                .await?;
        }
        Ok(())
    }

    /// The session whose record line names the artifact's latest version.
    pub(crate) fn artifact_session(&self, thread: ThreadId, id: &str) -> Option<SessionRef> {
        let family = self.threads.get(&thread)?.family;
        let needle = format!("\"id\":\"{id}\"");
        self.local_log(family).iter().rev().find_map(|delta| match &delta.op {
            Op::Conv(ConvOp::Native { session, line, .. }) if line.contains("\"t\":\"artifact\"") && line.contains(&needle) => {
                Some(session.clone())
            }
            _ => None,
        })
    }

    async fn put_version(
        &mut self,
        thread: ThreadId,
        workspace: &str,
        reference: SessionRef,
        checkout: Option<PathBuf>,
        new: NewArtifact,
        recorded: Recorded,
    ) -> Result<ArtifactPut, CoreError> {
        // A new version may leave its title and kind as the last version's.
        let earlier = match new.id.as_deref() {
            Some(id) if valid_id(id) => self
                .history_entries(thread)
                .iter()
                .filter_map(|(_, entry)| entry.artifact.as_ref())
                .filter(|artifact| artifact.id == id)
                .max_by_key(|artifact| artifact.version)
                .map(|artifact| (artifact.title.clone(), artifact.kind.clone())),
            _ => None,
        };
        let title = match (new.title.trim(), &earlier) {
            ("", Some((title, _))) => title.clone(),
            (title, _) => title.to_owned(),
        };
        if title.is_empty() || title.chars().count() > TITLE_CHARS {
            return Err(CoreError::Invalid(format!("a title is 1 to {TITLE_CHARS} characters")));
        }
        let caption = new.caption.as_deref().map(str::trim).filter(|caption| !caption.is_empty()).map(str::to_owned);
        if caption.as_ref().is_some_and(|caption| caption.chars().count() > CAPTION_CHARS) {
            return Err(CoreError::Invalid(format!("a caption is at most {CAPTION_CHARS} characters")));
        }
        let kind = match (new.inline, new.kind.trim(), &earlier) {
            (true, _, _) => "image".to_owned(),
            (false, "", Some((_, kind))) => kind.clone(),
            (false, kind, _) => kind.to_owned(),
        };
        let note = silt_proto::artifact::short_text(new.note.as_deref(), silt_proto::artifact::NOTE_CHARS, "a note")
            .map_err(CoreError::Invalid)?;
        let described = silt_proto::artifact::short_text(
            new.description.as_deref(),
            silt_proto::artifact::DESCRIPTION_CHARS,
            "a description",
        )
        .map_err(CoreError::Invalid)?;
        let entries = self.history_entries(thread);
        let artifacts: Vec<&ArtifactView> = entries.iter().filter_map(|(_, entry)| entry.artifact.as_ref()).collect();
        let mut new = new;
        // Edits of the version's files apply to those files; the rest, to its page.
        let (file_edits, page_edits): (Vec<_>, Vec<_>) =
            std::mem::take(&mut new.edits).into_iter().partition(|edit| edit.file.is_some());
        new.edits = page_edits;
        let latest_of = |id: Option<&str>| {
            id.and_then(|id| {
                artifacts.iter().filter(|artifact| artifact.id == id).max_by_key(|artifact| artifact.version).copied()
            })
        };
        if new.edits.is_empty()
            && new.content.is_none()
            && new.path.is_none()
            && (!file_edits.is_empty() || !new.files.is_empty())
        {
            // Only its files change: the page stays as it was.
            let latest = latest_of(new.id.as_deref())
                .ok_or_else(|| CoreError::Invalid("a version's files change an artifact: give its id".into()))?;
            let hash =
                latest.blob.parse().map_err(|_| CoreError::Invalid("the latest version's content isn't kept here".into()))?;
            new.content = Some(
                String::from_utf8(self.store.blobs().get(hash).await?)
                    .map_err(|_| CoreError::Invalid("the latest version isn't text".into()))?,
            );
        }
        let mut edited_files: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
        for edit in &file_edits {
            let path = edit.file.clone().unwrap_or_default();
            let source = match edited_files.get(&path) {
                Some(source) => source.clone(),
                None => {
                    let file = latest_of(new.id.as_deref())
                        .and_then(|latest| latest.files.as_ref())
                        .and_then(|files| files.get(&path))
                        .ok_or_else(|| CoreError::Invalid(format!("the latest version has no file {path}")))?;
                    let hash = file.blob.parse().map_err(|_| CoreError::Invalid(format!("{path} isn't kept here")))?;
                    String::from_utf8(self.store.blobs().get(hash).await?)
                        .map_err(|_| CoreError::Invalid(format!("{path} isn't text, so it's replaced, not edited")))?
                }
            };
            let edited = silt_proto::artifact::apply_edits(&source, std::slice::from_ref(edit))
                .map_err(|error| CoreError::Invalid(format!("{path}: {error}")))?;
            edited_files.insert(path, edited);
        }
        if let Some(patch) = new.patch.take() {
            if new.content.is_some() || new.path.is_some() || !new.edits.is_empty() {
                return Err(CoreError::Invalid("give a patch, or content, a path or edits, not more than one".into()));
            }
            let latest = latest_of(new.id.as_deref())
                .ok_or_else(|| CoreError::Invalid("a patch changes an artifact: give its id".into()))?;
            if !silt_proto::artifact_types::is_type(&latest.kind) {
                return Err(CoreError::Invalid(format!("a {} artifact is changed by edits, not a patch", latest.kind)));
            }
            let hash =
                latest.blob.parse().map_err(|_| CoreError::Invalid("the latest version's content isn't kept here".into()))?;
            let source = String::from_utf8(self.store.blobs().get(hash).await?)
                .map_err(|_| CoreError::Invalid("the latest version isn't text".into()))?;
            new.content = Some(silt_proto::artifact_types::apply_patch(&source, &patch).map_err(CoreError::Invalid)?);
        }
        if !new.edits.is_empty() {
            if new.content.is_some() || new.path.is_some() {
                return Err(CoreError::Invalid("give edits, or content or a path, not both".into()));
            }
            let id = new.id.as_deref().ok_or_else(|| CoreError::Invalid("edits change an artifact: give its id".into()))?;
            let latest = artifacts
                .iter()
                .filter(|artifact| artifact.id == id)
                .max_by_key(|artifact| artifact.version)
                .ok_or_else(|| CoreError::Invalid(format!("this thread has no artifact {id}")))?;
            if !matches!(latest.kind.as_str(), "html" | "svg" | "markdown" | "mermaid") {
                return Err(CoreError::Invalid(format!("a {} artifact is changed by a new file, not edits", latest.kind)));
            }
            let hash =
                latest.blob.parse().map_err(|_| CoreError::Invalid("the latest version's content isn't kept here".into()))?;
            let source = String::from_utf8(self.store.blobs().get(hash).await?)
                .map_err(|_| CoreError::Invalid("the latest version isn't text".into()))?;
            let edited =
                silt_proto::artifact::apply_edits(&source, &new.edits).map_err(|error| CoreError::Invalid(error.to_string()))?;
            new.content = Some(edited);
        }
        let file = match &new.path {
            Some(path) => {
                let checkout =
                    checkout.as_deref().ok_or_else(|| CoreError::Invalid("a page saves its own source, not a file".into()))?;
                Some(readable_file(path, checkout, &self.artifact_denies(workspace))?)
            }
            None => None,
        };
        // A doc's or a sheet's later version is a snapshot of it as it
        // stands; content given for a doc replaces its text (ADR 0077).
        let edited_id = new.id.as_deref().filter(|_| silt_proto::edited::is_edited(&kind)).map(str::to_owned);
        if let Some(existing) = &edited_id {
            if kind == "doc"
                && let Some(text) = new.content.take()
            {
                let current = self.doc_data(&thread.to_string(), existing, silt_proto::edited::DocRequest::Read).await?;
                let length = current["text"].as_str().unwrap_or_default().chars().count();
                let edit = silt_proto::edited::DocEdit::Splice { pos: 0, delete: length, insert: text };
                self.doc_data(
                    &thread.to_string(),
                    existing,
                    silt_proto::edited::DocRequest::Edit { edits: vec![edit], base: None },
                )
                .await?;
            }
            if new.content.is_none() && new.path.is_none() {
                new.content = Some(self.edited_snapshot(thread, existing, &kind).await?);
            }
        }
        let (bytes, media_type) = content_of(&new, &kind, file.as_deref())?;
        let (id, version) = match new.id.as_deref() {
            Some(id) if !valid_id(id) => return Err(CoreError::Invalid(format!("{id} isn't an artifact id"))),
            Some(id) => match latest_version(artifacts.iter().copied(), id) {
                0 => return Err(CoreError::Invalid(format!("this thread has no artifact {id}"))),
                latest => (id.to_owned(), latest + 1),
            },
            None => (new_id(), 1),
        };
        let last = artifacts.iter().filter(|artifact| artifact.id == id).max_by_key(|artifact| artifact.version);
        let previous = last.map(|artifact| artifact.design_system.clone().unwrap_or_else(|| design::NONE.to_owned()));
        let description = described.or_else(|| last.and_then(|artifact| artifact.description.clone()));
        let capabilities = match &new.capabilities {
            None => last.and_then(|artifact| artifact.capabilities.clone()),
            Some(declared) if declared.is_empty() => None,
            Some(declared) => Some(silt_proto::artifact::checked_capabilities(declared).map_err(CoreError::Invalid)?),
        };
        let files = self
            .version_files(
                last.and_then(|artifact| artifact.files.clone()).unwrap_or_default(),
                std::mem::take(&mut new.files),
                edited_files,
                &kind,
                checkout.as_deref(),
                workspace,
                bytes.len() as u64,
            )
            .await?;
        // A doc or a sheet is edited by anyone who may comment unless it says
        // otherwise, which the server checks by its declaration; a whiteboard
        // made before it took images takes them from its next version.
        let capabilities = silt_proto::artifact::with_kind_defaults(&kind, capabilities);
        let mut missing = None;
        let styled = if matches!(kind.as_str(), "html" | "data") {
            let workspace_default = self.workspace_design_system(workspace);
            let repository = checkout.as_deref().and_then(design::project_default);
            let picked = design::project_choices(&self.data_dir).remove(&self.threads[&thread].repo);
            let defaults = design::Defaults {
                picked: picked.as_deref(),
                repository: repository.as_deref(),
                workspace: workspace_default.as_deref(),
            };
            let asked = new.design_system.as_deref().map(str::trim).is_some_and(|name| !name.is_empty());
            match design::chosen(new.design_system.as_deref(), previous.as_deref(), defaults) {
                Some(name) => match design::find(&design::systems(checkout.as_deref(), &self.data_dir), name) {
                    Ok(system) => Some((system.name.to_string(), system.compiled()?)),
                    // A name carried over from the last version or a default
                    // may be one this machine lacks, or one Silt no longer
                    // ships (ADR 0087): the version goes unstyled and says so.
                    Err(_) if !asked => {
                        missing = Some(name.to_owned());
                        None
                    }
                    Err(error) => return Err(error),
                },
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
        let page_session = reference.clone();
        self.append_conv(family, thread, Author::User, ConvOp::Attachment { session: reference.clone(), name, blob }).await?;
        for (path, file, added) in &files {
            if *added {
                let blob = file.blob.parse().map_err(|_| CoreError::Invalid(format!("{path} isn't kept")))?;
                let name = format!("artifact/{id}/v{version}/files/{path}");
                self.append_conv(family, thread, Author::User, ConvOp::Attachment { session: reference.clone(), name, blob })
                    .await?;
            }
        }
        let files: std::collections::BTreeMap<String, silt_proto::artifact::ArtifactFile> =
            files.into_iter().map(|(path, file, _)| (path, file)).collect();
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
        let contract = if kind == "html" {
            let kept = last.and_then(|artifact| artifact.contract.as_deref());
            Some(silt_proto::artifact::contract_for(new.contract.as_deref(), kept).map_err(CoreError::Invalid)?)
        } else {
            None
        };
        let view = ArtifactView {
            contract,
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
            note,
            description,
            capabilities,
            files: (!files.is_empty()).then_some(files),
            by: match &recorded {
                Recorded::ByRun => None,
                Recorded::ByPage(by) => Some(by.clone()),
            },
        };
        let line = silt_agents::acp::artifact(&view);
        match recorded {
            Recorded::ByRun => self.record_acp(thread, &line).await?,
            // Recorded in the session this machine keeps, where it keeps it,
            // so its readers see the line; else straight onto the log.
            Recorded::ByPage(_) if self.session_mut(&page_session.id).is_some() => {
                self.record_line(&page_session.id, "main", line, None).await?;
            }
            Recorded::ByPage(_) => {
                let op = ConvOp::Native { session: page_session, file: SessionFile::Main, line };
                self.append_conv(family, thread, Author::User, op).await?;
            }
        }
        if version == 1 && silt_proto::edited::is_edited(&view.kind) {
            self.seed_edited(thread, &id, &view.kind, std::str::from_utf8(&bytes).unwrap_or_default()).await?;
        }
        Ok(ArtifactPut { id, version, design_system, dropped, missing })
    }

    /// The design system a run reads before making an artifact: the one
    /// named, else its project's default, else its workspace's (ADR 0071).
    pub(crate) async fn artifact_guide(&self, token: &str, asked: Option<String>) -> Result<ArtifactGuide, CoreError> {
        let hash = blake3::hash(token.as_bytes());
        let (thread, workspace) = self
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
        // Only a name asked for must exist; a default naming one this
        // machine lacks leaves none applying (ADR 0087).
        let (system, missing) = match name.map(|name| (name, design::find(&systems, name))) {
            None => (None, None),
            Some((_, Ok(system))) => (Some(system), None),
            Some((_, Err(error))) if asked.is_some() => return Err(error),
            Some((name, Err(_))) => (None, Some((name.to_owned(), why.to_owned()))),
        };
        Ok(ArtifactGuide {
            system: system.as_ref().map(|system| system.view(true)),
            why: system.is_some().then(|| why.to_owned()),
            systems: systems.iter().map(|system| system.view(false)).collect(),
            missing,
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
        let (thread, _) = self
            .store
            .run_token(hash.as_bytes())
            .await?
            .ok_or_else(|| CoreError::Invalid("this run's token isn't known here".into()))?;
        self.artifact_version(thread, id, version).await
    }

    /// A version of one of `thread`'s artifacts as the web UI's frame route
    /// composes it (ADR 0074), the latest when `version` is none.
    pub(crate) async fn artifact_version_of(
        &self,
        thread: &str,
        id: &str,
        version: Option<u32>,
    ) -> Result<ArtifactRead, CoreError> {
        if !valid_id(id) {
            return Err(CoreError::Invalid(format!("{id} isn't an artifact id")));
        }
        let thread = self.find_thread(thread)?;
        self.artifact_version(thread, id, version).await
    }

    async fn artifact_version(&self, thread: ThreadId, id: &str, version: Option<u32>) -> Result<ArtifactRead, CoreError> {
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
        self.history_entries(thread)
            .iter()
            .filter_map(|(_, entry)| entry.artifact.as_ref())
            .find_map(|artifact| artifact.media_type_of(blob))
    }
}

/// The name a version is kept under beside the session.
fn file_name(kind: &str, media_type: &str) -> &'static str {
    match (kind, media_type) {
        ("html", _) => "index.html",
        ("svg", _) | (_, "image/svg+xml") => "image.svg",
        ("markdown", _) => "index.md",
        ("mermaid", _) => "diagram.mmd",
        (_, "application/json") => "content.json",
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
            contract: None,
            title: "t".into(),
            kind: kind.into(),
            content: content.map(str::to_owned),
            path: None,
            id: None,
            caption: None,
            inline: false,
            design_system: None,
            edits: Vec::new(),
            note: None,
            description: None,
            capabilities: None,
            files: Default::default(),
            patch: None,
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
            contract: None,
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
            note: None,
            description: None,
            capabilities: None,
            by: None,
            files: None,
        };
        let all = [view("aaaaaaaaaaaa", 1), view("aaaaaaaaaaaa", 2), view("bbbbbbbbbbbb", 1)];
        assert_eq!(latest_version(&all, "aaaaaaaaaaaa"), 2);
        assert_eq!(latest_version(&all, "cccccccccccc"), 0);
    }
}
