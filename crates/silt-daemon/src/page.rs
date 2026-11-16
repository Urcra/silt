//! An artifact's page data on the daemon (ADR 0072): reads fold from the
//! family along the thread's lineage; writes become `Op::Page` deltas,
//! checked first against the rules the artifact's latest version declares.
//! The server checks them again when they're pushed.

use silt_crdt::CausalGraph;
use silt_log::{Author, Delta, Op, PageOp};
use silt_proto::page::{Level, PageRequest, Rules};
use silt_types::{ArtifactId, DeltaSpan, FamilyId, ThreadId};

use crate::core::{Core, CoreError};

impl Core {
    /// Answers a read or makes a write of `artifact`'s page data in `thread`,
    /// for this machine's person.
    pub(crate) async fn page_data(
        &mut self,
        thread: &str,
        artifact: &str,
        request: PageRequest,
    ) -> Result<serde_json::Value, CoreError> {
        let thread = self.find_thread(thread)?;
        let artifact: ArtifactId =
            artifact.parse().map_err(|_| CoreError::Invalid(format!("{artifact} isn't an artifact id")))?;
        let rules = self.page_rules(thread, artifact)?;
        // The server checks each delta again against who wrote it, so a
        // write it would refuse is refused here first.
        let (me, level) = (self.page_me(thread), self.page_level(thread));
        let family_id = self.threads[&thread].family;
        // A family known only from the server is folded from its log.
        let docs = self.page_docs(family_id, thread, artifact);
        if let Some(read) = silt_proto::page::read(&docs, &request, &rules, &me, level) {
            return Ok(read);
        }
        let writes = request.writes(&docs).map_err(CoreError::Invalid)?;
        let ops = silt_proto::page::page_ops(artifact, &docs, writes, &rules, &me, level).map_err(CoreError::Invalid)?;
        let written = ops.len();
        for op in ops {
            self.append_page(family_id, thread, op).await?;
        }
        Ok(serde_json::json!({ "written": written }))
    }

    /// Adds, lists or deletes the files people added to `artifact`'s page
    /// (ADR 0074), under the level its `assets` declaration asks.
    pub(crate) async fn page_assets(
        &mut self,
        thread: &str,
        artifact: &str,
        asked: silt_proto::page::AssetRequest,
    ) -> Result<serde_json::Value, CoreError> {
        use base64::Engine as _;
        use silt_proto::page::AssetRequest;
        let thread = self.find_thread(thread)?;
        let artifact: ArtifactId =
            artifact.parse().map_err(|_| CoreError::Invalid(format!("{artifact} isn't an artifact id")))?;
        let config = self.declared(thread, artifact, "assets")?;
        let family_id = self.threads[&thread].family;
        let docs = self.page_docs(family_id, thread, artifact);
        let level = self.page_level(thread);
        match asked {
            AssetRequest::List => Ok(serde_json::to_value(silt_proto::page::assets(&docs)).unwrap_or_default()),
            AssetRequest::Add { name, media_type, data } => {
                if level < silt_proto::page::asset_level(&config) {
                    return Err(CoreError::Invalid("you may not add files to this page".into()));
                }
                if !silt_proto::artifact::asset_media_type_ok(&media_type) {
                    return Err(CoreError::Invalid(format!("a page's files may not be {media_type}")));
                }
                let name: String = name.chars().filter(|char| !char.is_control()).take(200).collect();
                let mut bytes = base64::engine::general_purpose::STANDARD
                    .decode(data.trim())
                    .map_err(|_| CoreError::Invalid("the file's base64 doesn't decode".into()))?;
                // An SVG is kept only as a picture, whatever it held.
                if media_type.starts_with("image/svg+xml") {
                    bytes = silt_proto::svg::sanitized_svg(&bytes).map_err(CoreError::Invalid)?;
                }
                if bytes.len() as u64 > silt_proto::artifact::ASSET_BYTES {
                    return Err(CoreError::Invalid(format!("a file is at most {} MiB", silt_proto::artifact::ASSET_BYTES >> 20)));
                }
                let taken: u64 = silt_proto::page::assets(&docs).iter().map(|kept| kept.size).sum();
                if taken.saturating_add(bytes.len() as u64) > silt_proto::artifact::PAGE_ASSETS_BYTES {
                    return Err(CoreError::Invalid(format!(
                        "this page's files would take more than {} MiB",
                        silt_proto::artifact::PAGE_ASSETS_BYTES >> 20
                    )));
                }
                let blob = self.store.blobs().put(&bytes).await?;
                let asset = format!("{:032x}", uuid::Uuid::new_v4().as_u128())[..12].to_owned();
                let size = bytes.len() as u64;
                let op = PageOp::Asset {
                    artifact,
                    asset: asset.clone(),
                    blob,
                    name: name.clone(),
                    media_type: media_type.clone(),
                    size,
                };
                self.append_page(family_id, thread, op).await?;
                Ok(serde_json::to_value(silt_proto::page::AssetView {
                    url: format!("_assets/{asset}"),
                    id: asset,
                    name,
                    media_type,
                    size,
                })
                .unwrap_or_default())
            }
            AssetRequest::Delete { id } => {
                if level < Level::Collaborator {
                    return Err(CoreError::Invalid("only the thread's owner and collaborators delete a page's files".into()));
                }
                let path = format!("_assets/{id}");
                if !silt_proto::page::asset_id_ok(&id) || !docs.contains_key(&path) {
                    return Err(CoreError::Invalid(format!("this page has no file {id}")));
                }
                self.append_page(family_id, thread, PageOp::Deleted { artifact, path }).await?;
                Ok(serde_json::json!({ "deleted": id }))
            }
        }
    }

    /// Lists or reads the thread's files for `artifact`'s page (ADR 0078):
    /// from its log, never a checkout, within the globs the version
    /// declares and outside the folders the thread's workspace denies.
    pub(crate) fn page_files(
        &self,
        thread: &str,
        artifact: &str,
        asked: silt_proto::page::FilesRequest,
    ) -> Result<serde_json::Value, CoreError> {
        use silt_proto::page::{FilesRequest, files_allowed, list_files};
        let thread = self.find_thread(thread)?;
        let id: ArtifactId = artifact.parse().map_err(|_| CoreError::Invalid(format!("{artifact} isn't an artifact id")))?;
        let config = self.declared(thread, id, "files")?;
        let record = &self.threads[&thread];
        let workspace = self.family_workspace(record.family).id.to_string();
        let denied = self.artifact_denies(&workspace);
        let readable = |path: &str| {
            files_allowed(&config, path)
                && (record.repo_root.as_os_str().is_empty()
                    || !denied.iter().any(|deny| record.repo_root.join(path).starts_with(deny)))
        };
        let family = self
            .families
            .get(&record.family)
            .ok_or_else(|| CoreError::Invalid("the thread's files aren't on this machine yet; resume or fork it first".into()))?;
        match asked {
            FilesRequest::List { path, recursive, cursor } => {
                let view = family.thread_view(thread);
                let paths = view.paths();
                let shown = paths
                    .iter()
                    .filter(|(_, inode)| view.tree().kind(**inode) == Some(silt_crdt::InodeKind::File))
                    .map(|(path, _)| path.as_str())
                    .filter(|path| readable(path));
                let (entries, next) = list_files(shown, &path, recursive, cursor.as_deref());
                Ok(serde_json::json!({ "entries": entries, "cursor": next }))
            }
            FilesRequest::Read { path, at } => {
                if !readable(&path) {
                    return Err(CoreError::Invalid(format!("not_found: this page may not read {path}")));
                }
                let file = self.file_for_link(thread, &silt_proto::view::FileQuery { path: path.clone(), at })?;
                let text = file.text.ok_or_else(|| CoreError::Invalid(format!("{path} isn't text")))?;
                if text.len() > 4 << 20 {
                    return Err(CoreError::Invalid(format!("too_large: {path} is over 4 MiB")));
                }
                Ok(serde_json::json!({ "path": path, "text": text, "size": text.len(), "revision": file.at }))
            }
        }
    }

    /// The bytes and type of an asset of `artifact`'s page, for its frame.
    pub(crate) async fn page_asset(
        &self,
        thread: &str,
        artifact: &str,
        asset: &str,
    ) -> Result<silt_proto::view::ArtifactContent, CoreError> {
        use base64::Engine as _;
        let thread = self.find_thread(thread)?;
        let artifact: ArtifactId =
            artifact.parse().map_err(|_| CoreError::Invalid(format!("{artifact} isn't an artifact id")))?;
        let family_id = self.threads[&thread].family;
        let docs = self.page_docs(family_id, thread, artifact);
        let doc = docs.get(&format!("_assets/{asset}")).ok_or_else(|| CoreError::Invalid(format!("no file {asset}")))?;
        let blob = doc["blob"].as_str().and_then(|blob| blob.parse().ok()).ok_or_else(|| CoreError::Invalid("no blob".into()))?;
        let media_type = doc["media_type"].as_str().unwrap_or("application/octet-stream").to_owned();
        let bytes = self.store.blobs().get(blob).await?;
        Ok(silt_proto::view::ArtifactContent { media_type, data: base64::engine::general_purpose::STANDARD.encode(bytes) })
    }

    pub(crate) fn page_docs(&self, family_id: FamilyId, thread: ThreadId, artifact: ArtifactId) -> silt_log::page::Documents {
        match self.families.get(&family_id) {
            Some(family) => family.page_documents(thread, artifact),
            None => {
                let log = self.local_log(family_id);
                silt_log::page_documents_in(log, &graph_of(log), thread, artifact)
            }
        }
    }

    /// What version `version` of the artifact declares, or its latest.
    pub(crate) fn declared_by(
        &self,
        thread: ThreadId,
        artifact: &str,
        version: Option<u32>,
    ) -> Option<silt_proto::artifact::Capabilities> {
        self.version_view(thread, artifact, version).and_then(|view| view.capabilities)
    }

    /// Version `version` of the artifact, or its latest.
    pub(crate) fn version_view(
        &self,
        thread: ThreadId,
        artifact: &str,
        version: Option<u32>,
    ) -> Option<silt_proto::view::ArtifactView> {
        let views = self.history_entries(thread);
        let mut versions = views.iter().filter_map(|(_, entry)| entry.artifact.as_ref()).filter(|view| view.id == artifact);
        let view = match version {
            Some(version) => versions.find(|view| view.version == version),
            None => versions.max_by_key(|view| view.version),
        };
        view.cloned()
    }

    /// The config the artifact's latest version declares for `capability`.
    pub(crate) fn declared(
        &self,
        thread: ThreadId,
        artifact: ArtifactId,
        capability: &str,
    ) -> Result<serde_json::Value, CoreError> {
        let id = artifact.to_string();
        self.declared_by(thread, &id, None)
            .and_then(|mut declared| declared.remove(capability))
            .ok_or_else(|| CoreError::Invalid(format!("artifact {id} doesn't declare {capability}")))
    }

    /// Writes `op` on `thread`: through the family's projection where this
    /// machine has one, else onto the log it keeps from the server, as page
    /// data needs no file's text.
    pub(crate) async fn append_page(&mut self, family_id: FamilyId, thread: ThreadId, op: PageOp) -> Result<(), CoreError> {
        if let Some(family) = self.families.get(&family_id) {
            let head = family.thread_head(thread);
            self.append_op(family_id, thread, head, Op::Page(op)).await?;
            return Ok(());
        }
        let log = self.local_log(family_id);
        let graph = graph_of(log);
        let parents = silt_log::thread_head_in(log, &graph, thread);
        let op = Op::Page(op);
        let span = DeltaSpan::new(self.thread_actor(thread), graph.next_lamport(), op.units() as u64);
        let delta = Delta { span, parents, thread, author: Author::User, time_ms: crate::core::now_ms(), op };
        delta.check_shape().map_err(|error| CoreError::Invalid(error.to_string()))?;
        self.raw.entry(family_id).or_default().push(delta.clone());
        self.persist(family_id, std::slice::from_ref(&delta)).await
    }

    /// The thread a run's token was made for.
    pub(crate) async fn run_thread(&self, token: &str) -> Result<String, CoreError> {
        let hash = blake3::hash(token.as_bytes());
        let (thread, _) = self
            .store
            .run_token(hash.as_bytes())
            .await?
            .ok_or_else(|| CoreError::Invalid("this run's token isn't known here".into()))?;
        Ok(thread.to_string())
    }

    /// [`Self::page_data`] for the agent of a run, in the thread its token
    /// was made for (ADR 0072).
    pub(crate) async fn run_page_data(
        &mut self,
        token: &str,
        artifact: &str,
        request: PageRequest,
    ) -> Result<serde_json::Value, CoreError> {
        let hash = blake3::hash(token.as_bytes());
        let (thread, _) = self
            .store
            .run_token(hash.as_bytes())
            .await?
            .ok_or_else(|| CoreError::Invalid("this run's token isn't known here".into()))?;
        self.page_data(&thread.to_string(), artifact, request).await
    }

    /// The rules the artifact's latest version declares; refused when it
    /// declares no `db`.
    fn page_rules(&self, thread: ThreadId, artifact: ArtifactId) -> Result<Rules, CoreError> {
        let id = artifact.to_string();
        let entries = self.history_entries(thread);
        let latest = entries
            .iter()
            .filter_map(|(_, entry)| entry.artifact.as_ref())
            .filter(|view| view.id == id)
            .max_by_key(|view| view.version)
            .ok_or_else(|| CoreError::Invalid(format!("this thread has no artifact {id}")))?;
        let config = latest
            .capabilities
            .as_ref()
            .and_then(|capabilities| capabilities.get("db"))
            .ok_or_else(|| CoreError::Invalid(format!("artifact {id} doesn't declare db")))?;
        Rules::of(Some(config)).map_err(CoreError::Invalid)
    }

    /// What this machine's person may do in `thread`: as `may_write`
    /// decides, else comment, as anyone who sees a thread may.
    pub(crate) fn page_level(&self, thread: ThreadId) -> Level {
        let user = self.own_user(thread);
        let Some(record) = self.threads.get(&thread) else {
            return Level::Comment;
        };
        if user.is_empty() || record.owner.is_empty() || record.owner == user {
            Level::Owner
        } else if self.collaborators.get(&thread).is_some_and(|names| names.contains(&user)) {
            Level::Collaborator
        } else {
            Level::Comment
        }
    }

    /// Who views `thread`'s pages here, for their `user` capability.
    pub(crate) fn page_viewer(&self, thread: &str) -> Result<silt_proto::page::Viewer, CoreError> {
        let thread = self.find_thread(thread)?;
        let me = self.remote_for(thread).map(|remote| remote.config.user.clone()).filter(|user| !user.is_empty());
        let owner = self.threads.get(&thread).map(|record| record.owner.clone()).unwrap_or_default();
        let collaborators = self.collaborators.get(&thread).cloned().unwrap_or_default();
        let people = std::iter::once(owner.as_str()).chain(collaborators.iter().map(String::as_str));
        Ok(silt_proto::page::Viewer::of(me.as_deref(), self.page_level(thread), people))
    }

    /// This machine's person's id in page data: the one their home server
    /// gave them, once known, else this machine's own (ADR 0072).
    pub(crate) fn page_me(&self, thread: ThreadId) -> String {
        match self.remote_for(thread) {
            Some(remote) if !remote.config.user.is_empty() => silt_proto::page::person_id(&remote.config.user),
            _ => "local".to_owned(),
        }
    }
}

/// The causal graph of a log the daemon keeps without a projection.
pub(crate) fn graph_of(log: &[Delta]) -> CausalGraph {
    let mut graph = CausalGraph::new();
    for delta in log {
        // A delta whose parents the log lacks has no place in a lineage.
        let _ = graph.add(delta.span, &delta.parents);
    }
    graph
}
