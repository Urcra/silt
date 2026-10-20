use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use silt_agents::{claude, complete_lines};
use silt_checkout::{Bridge, DiskAction, DiskContent, DiskEntry, DiskFile, ImportFile, import};
use silt_crdt::InodeId;
use silt_git::{FileMode, Repo};
use silt_log::{Author, BaseText, ConvOp, Delta, Family, GitOp, Op, SessionFile, SessionRef, TurnMark};
use silt_store::{CheckoutRecord, SessionFileOffset, SessionRecord, Store, ThreadRecord};
use silt_types::{ActorId, FamilyId, Frontier, GitOid, ThreadId};
use silt_proto::Listing;
use silt_redact::Redactor;
use silt_sync::{ClientSync, Outcome};
use tokio::sync::{mpsc, oneshot};
use tracing::warn;

use crate::disk::{self, RawFile};
use crate::protocol::{HookReply, Mount, Request, Response, SessionStarted};
use crate::remote::{self, Remote};
use crate::watch::{self, Watcher};

/// How long after a tool finishes its file changes still count as its own;
/// covers the watcher's debounce.
const TOOL_SETTLE: Duration = Duration::from_millis(250);

pub enum Event {
    Request(Request, oneshot::Sender<Response>),
    Changed { checkout: i64, paths: Vec<PathBuf> },
    Tick,
    CloseTool { session_id: String, reply: oneshot::Sender<Response> },
    Pushed { family: FamilyId, generation: Option<u64>, outcome: Outcome, error: Option<String> },
    Pulled { family: FamilyId, generation: Option<u64>, outcome: Outcome, error: Option<String> },
    Listed(Result<Listing, String>),
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error(transparent)]
    Store(#[from] silt_store::StoreError),
    #[error(transparent)]
    Git(#[from] silt_git::GitError),
    #[error(transparent)]
    Log(#[from] silt_log::LogError),
    #[error(transparent)]
    Bridge(#[from] silt_checkout::BridgeError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("could not watch the checkout")]
    Watch(#[from] notify::Error),
    #[error("{0}")]
    Invalid(String),
}

pub(crate) struct Checkout {
    pub(crate) record: CheckoutRecord,
    pub(crate) family: FamilyId,
    pub(crate) repo: Repo,
    pub(crate) bridge: Bridge,
    pub(crate) _watcher: Option<Watcher>,
}

pub(crate) struct Session {
    pub(crate) record: SessionRecord,
    pub(crate) family: FamilyId,
    pub(crate) transcript: PathBuf,
    pub(crate) offsets: HashMap<String, SessionFileOffset>,
    pub(crate) open_tool: Option<String>,
    /// Size and mtime of each side file when it was last attached.
    pub(crate) attached: HashMap<String, (u64, SystemTime)>,
}

pub struct Core {
    pub(crate) store: Store,
    pub(crate) actor: ActorId,
    pub(crate) families: HashMap<FamilyId, Family>,
    /// Families only known from the server: their deltas, not yet projected
    /// because there's no local clone to read git blobs from.
    pub(crate) raw: HashMap<FamilyId, Vec<Delta>>,
    pub(crate) threads: HashMap<ThreadId, ThreadRecord>,
    pub(crate) checkouts: BTreeMap<i64, Checkout>,
    pub(crate) sessions: Vec<Session>,
    pub(crate) events: mpsc::UnboundedSender<Event>,
    pub(crate) data_dir: PathBuf,
    pub(crate) remote: Option<Remote>,
    pub(crate) syncs: HashMap<FamilyId, ClientSync>,
    pub(crate) redactor: Redactor,
    pub(crate) last_listing: Option<Instant>,
    pub(crate) sync_error: Option<String>,
    /// The HEAD commit each checkout was last seen at.
    pub(crate) heads: HashMap<i64, GitOid>,
    /// Messages for each thread's agent, handed over on its next prompt.
    pub(crate) notices: HashMap<ThreadId, Vec<String>>,
    pub(crate) web: Option<crate::web::WebInfo>,
    /// What recovery forks this daemon made, for `silt auth status`.
    pub(crate) recoveries: Vec<String>,
}

/// Reads git blobs for lazily imported files.
pub(crate) struct RepoBases<'a>(pub(crate) &'a Repo);

impl BaseText for RepoBases<'_> {
    fn text(&mut self, oid: GitOid) -> Option<String> {
        let bytes = self.0.blob(oid).ok()?;
        if bytes.contains(&0) {
            return None;
        }
        String::from_utf8(bytes).ok()
    }
}

impl Core {
    /// Loads everything from the store, restarts watchers and tails, and
    /// queues a rescan of every checkout to pick up changes made while the
    /// daemon was down.
    pub async fn load(
        store: Store,
        actor: ActorId,
        data_dir: &Path,
        events: mpsc::UnboundedSender<Event>,
    ) -> Result<Self, CoreError> {
        let mut core = Self {
            store,
            actor,
            families: HashMap::new(),
            raw: HashMap::new(),
            threads: HashMap::new(),
            checkouts: BTreeMap::new(),
            sessions: Vec::new(),
            events,
            data_dir: data_dir.to_owned(),
            remote: remote::load_config(data_dir).map(Remote::new),
            syncs: HashMap::new(),
            redactor: Redactor::from_env(),
            last_listing: None,
            sync_error: None,
            heads: HashMap::new(),
            notices: HashMap::new(),
            web: None,
            recoveries: Vec::new(),
        };
        for thread in core.store.threads().await? {
            core.threads.insert(thread.id, thread);
        }
        let families: HashSet<FamilyId> = core.threads.values().map(|thread| thread.family).collect();
        for family_id in families {
            let repo = core
                .threads
                .values()
                .filter(|thread| thread.family == family_id && !thread.repo_root.as_os_str().is_empty())
                .find_map(|thread| Repo::discover(&thread.repo_root).ok());
            let deltas = core.store.deltas(family_id).await?;
            match repo {
                Some(repo) => {
                    let mut family = Family::new();
                    for delta in deltas {
                        if let Err(error) = family.apply(delta, &mut RepoBases(&repo)) {
                            warn!(%error, "skipping a delta that no longer applies");
                        }
                    }
                    core.families.insert(family_id, family);
                }
                None => {
                    core.raw.insert(family_id, deltas);
                }
            }
            let state = core.store.sync_state(family_id).await?;
            core.syncs.insert(
                family_id,
                ClientSync::new(state.generation, state.pushed, state.pulled, state.last_pulled, state.view),
            );
        }
        for record in core.store.checkouts().await? {
            if !record.path.exists() {
                continue;
            }
            let entries = core
                .store
                .disk_files(record.id)
                .await?
                .into_iter()
                .map(|(path, file)| {
                    (
                        path,
                        DiskEntry {
                            inode: InodeId(file.inode),
                            hash: file.hash,
                            size: file.size,
                            mtime_ns: file.mtime_ns,
                            version: file.version,
                        },
                    )
                })
                .collect();
            core.start_checkout(record, entries)?;
        }
        for record in core.store.sessions().await? {
            let Some(family) = core.threads.get(&record.thread).map(|thread| thread.family) else {
                continue;
            };
            let offsets: HashMap<String, SessionFileOffset> = core
                .store
                .session_offsets(&record.agent, &record.session_id)
                .await?
                .into_iter()
                .map(|offset| (offset.file.clone(), offset))
                .collect();
            let Some(transcript) = offsets.get("main").map(|offset| offset.path.clone()) else {
                continue;
            };
            core.sessions.push(Session {
                record,
                family,
                transcript,
                offsets,
                open_tool: None,
                attached: HashMap::new(),
            });
        }
        let ids: Vec<i64> = core.checkouts.keys().copied().collect();
        for id in ids {
            core.rescan(id).await?;
        }
        Ok(core)
    }

    /// Handles a request; `None` means the reply is sent later.
    pub async fn request(&mut self, request: Request, reply: oneshot::Sender<Response>) {
        let response = match request {
            Request::Ping => Response::ok("pong"),
            Request::StartSession {
                cwd,
                agent,
                session_id,
                isolated,
            } => to_response(self.start_session(&cwd, &agent, &session_id, isolated).await),
            Request::Hook { event, input } => match self.hook(&event, input, reply).await {
                Some((response, reply)) => {
                    let _ = reply.send(response);
                    return;
                }
                None => return,
            },
            Request::EndSession { session_id, .. } => {
                to_response(self.tail_session(&session_id).await.map(|()| "ended"))
            }
            Request::Threads => Response::ok(self.thread_summaries()),
            Request::Show { thread } => to_response(self.find_thread(&thread).map(|thread| self.show(thread))),
            Request::Diff { thread, turn, base } => {
                to_response(self.find_thread(&thread).and_then(|thread| self.diff(thread, turn, base.as_deref())))
            }
            Request::Search { query, limit, filters } => to_response(self.search(&query, limit, &filters).await),
            Request::Reload => {
                self.remote = remote::load_config(&self.data_dir).map(Remote::new);
                self.syncs.clear();
                for family in self.families.keys().chain(self.raw.keys()).copied().collect::<Vec<_>>() {
                    let state = self.store.sync_state(family).await.unwrap_or_default();
                    self.syncs.insert(
                        family,
                        ClientSync::new(state.generation, state.pushed, state.pulled, state.last_pulled, state.view),
                    );
                }
                self.drive_sync();
                Response::ok("reloaded")
            }
            Request::Status => Response::ok(crate::protocol::DaemonStatus {
                server: self.remote.as_ref().map(|remote| remote.config.url.clone()),
                device: self.remote.as_ref().map(|remote| remote.config.device.to_string()),
                sync_error: self.sync_error.clone(),
                threads: self.threads.len(),
                diverged: self
                    .threads
                    .values()
                    .filter(|thread| self.syncs.get(&thread.family).is_some_and(|sync| sync.diverged().is_some()))
                    .map(|thread| thread.id.to_string())
                    .collect(),
                recovered: self.recoveries.clone(),
            }),
            Request::Resume { thread, cwd, here } => to_response(self.resume(&thread, &cwd, here).await),
            Request::Fork { thread, cwd, here } => to_response(self.fork(&thread, &cwd, here).await),
            Request::Pull { thread, cwd } => to_response(self.pull(thread.as_deref(), &cwd).await),
            Request::MrCreate {
                thread,
                cwd,
                title,
                summary,
            } => to_response(self.mr_create(thread.as_deref(), &cwd, title, summary).await),
            Request::MrList { thread } => to_response(self.mr_list(thread.as_deref())),
            Request::MrShow { mr } => to_response(self.mr_show(&mr)),
            Request::MrApprove { mr } => to_response(self.mr_approve(&mr).await),
            Request::MrDecline { mr, reason } => to_response(self.mr_decline(&mr, reason).await),
            Request::Turns { thread } => to_response(self.find_thread(&thread).map(|thread| self.turns(thread))),
            Request::OpenWeb => to_response(self.open_web().await),
            Request::HistoryRead { thread, offset, limit } => {
                to_response(self.find_thread(&thread).map(|thread| self.history_page(thread, offset, limit)))
            }
            Request::HistorySearch { thread, query, limit } => match self.find_thread(&thread) {
                Ok(thread) => to_response(self.history_search(thread, &query, limit).await),
                Err(error) => Response::error(error),
            },
            Request::FileHistory { thread, path } => {
                to_response(self.find_thread(&thread).and_then(|thread| self.file_history(thread, &path)))
            }
            Request::InFlight { cwd, path } => to_response(self.in_flight_anywhere(&cwd, &path).await),
            Request::Comment { thread, on, text, cwd } => to_response(self.add_comment(&thread, on, text, &cwd).await),
            Request::Comments { thread, cwd } => to_response(self.list_comments(&thread, &cwd).await),
            Request::ResolveComment { thread, comment, cwd } => {
                to_response(self.resolve_comment(&thread, &comment, &cwd).await)
            }
            Request::SetVisibility { cwd, thread, visibility } => {
                to_response(self.set_visibility(&cwd, thread.as_deref(), visibility).await)
            }
            Request::Shutdown => Response::ok("shutting down"),
        };
        let _ = reply.send(response);
    }

    pub async fn changed(&mut self, checkout: i64, paths: Vec<PathBuf>) {
        if let Err(error) = self.ingest_paths(checkout, paths).await {
            warn!(%error, checkout, "could not record changes");
        }
    }

    pub async fn tick(&mut self) {
        self.drive_sync();
        if let Err(error) = self.check_commits().await {
            warn!(%error, "could not record new commits");
        }
        let ids: Vec<String> = self.sessions.iter().map(|session| session.record.session_id.clone()).collect();
        for id in ids {
            if let Err(error) = self.tail_session(&id).await {
                warn!(%error, session = id, "could not read the session transcript");
            }
        }
    }

    pub async fn close_tool(&mut self, session_id: &str, reply: oneshot::Sender<Response>) {
        let tool = self.session_mut(session_id).and_then(|session| session.open_tool.take());
        if let Some(tool_use_id) = tool {
            let _ = self
                .record_turn(session_id, TurnMark::ToolFinished { tool_use_id })
                .await;
        }
        let _ = reply.send(Response::ok("ok"));
    }

    async fn start_session(
        &mut self,
        cwd: &Path,
        agent: &str,
        session_id: &str,
        isolated: bool,
    ) -> Result<SessionStarted, CoreError> {
        let cwd = cwd.canonicalize()?;
        let repo = Repo::discover(&cwd)?;
        let relative = cwd.strip_prefix(repo.root()).unwrap_or(Path::new("")).to_owned();
        let head = repo.head_commit()?;
        let files: Vec<ImportFile> = match head {
            Some(commit) => repo
                .files_at(commit)?
                .into_iter()
                .filter(|file| file.mode != FileMode::Symlink)
                .map(|file| ImportFile {
                    path: file.path,
                    oid: file.oid,
                    executable: file.mode == FileMode::Executable,
                })
                .collect(),
            None => Vec::new(),
        };
        let thread = ThreadId(uuid::Uuid::now_v7().as_u128());
        let family_id = FamilyId(uuid::Uuid::now_v7().as_u128());
        let record = ThreadRecord {
            id: thread,
            family: family_id,
            repo_root: repo.root().to_owned(),
            repo: repo
                .remote_url("origin")?
                .map(|url| silt_git::normalize_remote(&url))
                .unwrap_or_default(),
            created_ms: now_ms(),
            owner: self.own_user(),
        };
        self.store.create_thread(&record).await?;
        self.register_thread(&record);
        self.threads.insert(thread, record);

        let mut family = Family::new();
        let branch = repo.head_branch()?;
        let deltas = import(&mut family, self.thread_actor(thread), thread, head.map(|commit| (commit, branch)), &files, now_ms())?;
        self.store.append_deltas(family_id, &deltas).await?;
        self.families.insert(family_id, family);

        let checkout_path = if isolated {
            let commit = head.ok_or_else(|| CoreError::Invalid("an isolated checkout needs at least one commit".into()))?;
            repo.create_managed_checkout(&thread.to_string(), commit, None)?.path
        } else {
            repo.root().to_owned()
        };
        let checkout_actor = ActorId(uuid::Uuid::new_v4().as_u128());
        let existing = self
            .checkouts
            .values()
            .find(|checkout| checkout.record.path == checkout_path)
            .map(|checkout| checkout.record.clone());
        let checkout = match existing {
            Some(previous) => {
                self.checkouts.remove(&previous.id);
                self.store.rebind_checkout(previous.id, thread, checkout_actor).await?;
                CheckoutRecord {
                    thread,
                    actor: checkout_actor,
                    ..previous
                }
            }
            None => {
                self.store
                    .add_checkout(thread, &checkout_path, checkout_actor, isolated, now_ms())
                    .await?
            }
        };
        let checkout_id = checkout.id;
        self.start_checkout(checkout, BTreeMap::new())?;
        self.rescan(checkout_id).await?;

        // `join("")` would add a trailing slash, which changes Claude's slug.
        let agent_cwd = if relative.as_os_str().is_empty() {
            checkout_path.clone()
        } else {
            checkout_path.join(&relative)
        };
        let config_dir = claude::config_dir().ok_or_else(|| CoreError::Invalid("HOME is not set".into()))?;
        let transcript = claude::session_paths(&config_dir, &agent_cwd, session_id).transcript;
        let session = SessionRecord {
            agent: agent.to_owned(),
            session_id: session_id.to_owned(),
            thread,
            checkout_id,
        };
        self.store.add_session(&session, now_ms()).await?;
        let main = SessionFileOffset {
            file: "main".into(),
            path: transcript.clone(),
            offset: 0,
        };
        self.store.set_session_offset(agent, session_id, &main).await?;
        self.sessions.push(Session {
            record: session,
            family: family_id,
            transcript,
            offsets: HashMap::from([("main".to_owned(), main)]),
            open_tool: None,
            attached: HashMap::new(),
        });
        let reference = SessionRef {
            agent: agent.to_owned(),
            id: session_id.to_owned(),
        };
        self.append_session_started(family_id, thread, reference, &agent_cwd, &checkout_path)
            .await?;
        if let Some(commit) = head {
            self.record_bundle(family_id, thread, &repo, commit).await?;
        }
        Ok(SessionStarted {
            thread: thread.to_string(),
            mount: isolated.then(|| self.mount(&repo)),
            checkout: checkout_path,
            agent_cwd,
        })
    }

    pub(crate) fn start_checkout(&mut self, record: CheckoutRecord, entries: BTreeMap<String, DiskEntry>) -> Result<(), CoreError> {
        let family = self
            .threads
            .get(&record.thread)
            .map(|thread| thread.family)
            .ok_or_else(|| CoreError::Invalid("checkout of an unknown thread".into()))?;
        let repo = Repo::discover(&record.path)?;
        let events = self.events.clone();
        let id = record.id;
        let watcher = watch::watch(&record.path, move |paths| {
            let _ = events.send(Event::Changed { checkout: id, paths });
        })?;
        let bridge = Bridge::new(record.actor, record.thread, entries);
        self.checkouts.insert(
            record.id,
            Checkout {
                record,
                family,
                repo,
                bridge,
                _watcher: Some(watcher),
            },
        );
        Ok(())
    }

    pub(crate) async fn rescan(&mut self, checkout: i64) -> Result<(), CoreError> {
        let root = self.checkout(checkout)?.record.path.clone();
        self.ingest_paths(checkout, vec![root]).await
    }

    /// Records changes under `paths` (absolute; directories are scanned).
    async fn ingest_paths(&mut self, checkout_id: i64, paths: Vec<PathBuf>) -> Result<(), CoreError> {
        let checkout = self.checkout(checkout_id)?;
        let root = checkout.record.path.clone();
        let mut candidates: Vec<String> = Vec::new();
        for path in paths {
            let Ok(relative) = path.strip_prefix(&root) else {
                continue;
            };
            let relative = relative.to_string_lossy().replace('\\', "/");
            if path.is_dir() {
                candidates.extend(self.files_under(checkout_id, &relative)?);
                if relative.is_empty() {
                    candidates.extend(self.checkout(checkout_id)?.bridge.entries().keys().cloned());
                    candidates.extend(self.family_paths(checkout_id));
                }
            } else if !relative.is_empty() {
                if !path.exists() {
                    let prefix = format!("{relative}/");
                    let recorded = self.checkout(checkout_id)?.bridge.entries().keys();
                    candidates.extend(recorded.filter(|path| path.starts_with(&prefix)).cloned());
                }
                candidates.push(relative);
            }
        }
        candidates.retain(|path| !disk::is_internal(path));
        candidates.sort();
        candidates.dedup();
        let ignored = self.checkout(checkout_id)?.repo.ignored(&candidates)?;
        let mut worklist: Vec<String> = candidates.into_iter().filter(|path| !ignored.contains(path)).collect();
        for path in self.ingest_renames(checkout_id, &worklist).await? {
            worklist.retain(|candidate| *candidate != path);
        }
        let mut rounds = 0;
        while !worklist.is_empty() && rounds < 8 {
            rounds += 1;
            let mut retry = Vec::new();
            for path in std::mem::take(&mut worklist) {
                retry.extend(self.ingest_file(checkout_id, &path).await?);
            }
            worklist = retry;
        }
        Ok(())
    }

    /// Records each vanished file whose exact bytes reappeared at a new path
    /// in the same batch as a move. Returns the paths it handled.
    async fn ingest_renames(&mut self, checkout_id: i64, paths: &[String]) -> Result<Vec<String>, CoreError> {
        let root = self.checkout(checkout_id)?.record.path.clone();
        let entries = self.checkout(checkout_id)?.bridge.entries().clone();
        let mut vanished: BTreeMap<[u8; 32], Vec<&String>> = BTreeMap::new();
        let mut appeared = Vec::new();
        for path in paths {
            match (entries.get(path), disk::read(&root, path)?) {
                (Some(entry), None) => vanished.entry(entry.hash).or_default().push(path),
                (None, Some(raw)) => appeared.push((path, raw)),
                _ => {}
            }
        }
        let mut handled = Vec::new();
        for (to, raw) in appeared {
            let Some(from) = vanished.get_mut(&raw.hash).and_then(Vec::pop) else {
                continue;
            };
            let disk_file = self.disk_file(checkout_id, to, raw).await?;
            let author = self.author_for(checkout_id);
            let family_id = self.checkout(checkout_id)?.family;
            let checkout = self.checkouts.get_mut(&checkout_id).expect("checked above");
            let family = self.families.get_mut(&family_id).expect("checkout's family is loaded");
            let Some(ingested) = checkout.bridge.rename(
                family,
                &mut RepoBases(&checkout.repo),
                from,
                to,
                &disk_file,
                &author,
                now_ms(),
            )?
            else {
                continue;
            };
            self.store.append_deltas(family_id, &ingested.deltas).await?;
            self.persist_entry(checkout_id, from).await?;
            self.persist_entry(checkout_id, to).await?;
            self.propagate(checkout_id, &ingested.deltas).await?;
            handled.extend([from.clone(), to.clone()]);
        }
        Ok(handled)
    }

    fn files_under(&self, checkout: i64, relative: &str) -> Result<Vec<String>, CoreError> {
        let files = self.checkout(checkout)?.repo.worktree_files()?;
        Ok(files
            .into_iter()
            .filter(|path| relative.is_empty() || path.starts_with(&format!("{relative}/")) || path == relative)
            .collect())
    }

    fn family_paths(&self, checkout: i64) -> Vec<String> {
        let Ok(checkout) = self.checkout(checkout) else {
            return Vec::new();
        };
        self.families
            .get(&checkout.family)
            .map(|family| {
                let view = family.thread_view(checkout.record.thread);
                view.paths()
                    .into_iter()
                    .filter(|(_, inode)| view.tree().kind(*inode) != Some(silt_crdt::InodeKind::Dir))
                    .map(|(path, _)| path)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Records one file's disk state. Returns paths that changed again
    /// meanwhile and need another pass.
    pub(crate) async fn ingest_file(&mut self, checkout_id: i64, path: &str) -> Result<Vec<String>, CoreError> {
        let root = self.checkout(checkout_id)?.record.path.clone();
        let raw = disk::read(&root, path)?;
        let entry_hash = self.checkout(checkout_id)?.bridge.entries().get(path).map(|entry| entry.hash);
        let family_id = self.checkout(checkout_id)?.family;
        let thread = self.checkout(checkout_id)?.record.thread;
        let in_family = self
            .families
            .get(&family_id)
            .is_some_and(|family| family.thread_view(thread).paths().contains_key(path));
        match (&raw, entry_hash) {
            (Some(raw), Some(hash)) if raw.hash == hash => return Ok(Vec::new()),
            (None, None) if !in_family => return Ok(Vec::new()),
            _ => {}
        }
        let disk_file = match raw {
            Some(raw) => Some(self.disk_file(checkout_id, path, raw).await?),
            None => None,
        };
        let author = self.author_for(checkout_id);
        let checkout = self.checkouts.get_mut(&checkout_id).expect("checked above");
        let family = self.families.get_mut(&family_id).expect("checkout's family is loaded");
        let ingested = checkout.bridge.ingest(
            family,
            &mut RepoBases(&checkout.repo),
            path,
            disk_file,
            &author,
            now_ms(),
        )?;
        self.store.append_deltas(family_id, &ingested.deltas).await?;
        self.persist_entry(checkout_id, path).await?;
        let mut again = self.perform(checkout_id, ingested.actions).await?;
        again.extend(self.propagate(checkout_id, &ingested.deltas).await?);
        Ok(again)
    }

    async fn disk_file(&self, checkout: i64, path: &str, raw: RawFile) -> Result<DiskFile, CoreError> {
        let content = if raw.is_text() {
            let clean = self.checkout(checkout)?.repo.filters()?.to_git(path, &raw.bytes)?;
            match String::from_utf8(clean) {
                Ok(text) => DiskContent::Text(text),
                Err(error) => DiskContent::Blob(self.store.blobs().put(error.as_bytes()).await?),
            }
        } else {
            DiskContent::Blob(self.store.blobs().put(&raw.bytes).await?)
        };
        Ok(DiskFile {
            content,
            executable: raw.executable,
            hash: raw.hash,
            size: raw.size(),
            mtime_ns: raw.mtime_ns,
        })
    }

    /// Brings other checkouts of the family up to date with `deltas`.
    async fn propagate(&mut self, from: i64, deltas: &[Delta]) -> Result<Vec<String>, CoreError> {
        let family_id = self.checkout(from)?.family;
        let others: Vec<i64> = self
            .checkouts
            .iter()
            .filter(|(id, checkout)| **id != from && checkout.family == family_id)
            .map(|(id, _)| *id)
            .collect();
        for other in others {
            let checkout = self.checkout(other)?;
            let inodes: Vec<InodeId> = deltas
                .iter()
                .filter(|delta| !checkout.bridge.is_own(delta))
                .filter_map(touched_inode)
                .collect();
            let family = &self.families[&family_id];
            let actions = checkout.bridge.reconcile(family, &mut RepoBases(&checkout.repo), inodes);
            let again = self.perform(other, actions).await?;
            for path in again {
                Box::pin(self.ingest_file(other, &path)).await?;
            }
        }
        Ok(Vec::new())
    }

    /// Applies disk actions, skipping files that changed on disk since they
    /// were last recorded; those are returned to be ingested first.
    pub(crate) async fn perform(&mut self, checkout_id: i64, actions: Vec<DiskAction>) -> Result<Vec<String>, CoreError> {
        self.perform_with(checkout_id, actions, false).await
    }

    /// Like [`Core::perform`]; with `force`, files on disk are overwritten
    /// even when the checkout never recorded them (materializing a thread).
    pub(crate) async fn perform_with(
        &mut self,
        checkout_id: i64,
        actions: Vec<DiskAction>,
        force: bool,
    ) -> Result<Vec<String>, CoreError> {
        let mut changed_meanwhile = Vec::new();
        for action in actions {
            let root = self.checkout(checkout_id)?.record.path.clone();
            match action {
                DiskAction::Write {
                    path,
                    inode,
                    content,
                    executable,
                    version,
                } => {
                    let current = disk::read(&root, &path)?;
                    let recorded = self.checkout(checkout_id)?.bridge.entries().get(&path).map(|entry| entry.hash);
                    if let Some(current) = &current
                        && recorded != Some(current.hash)
                        && !(force && recorded.is_none())
                    {
                        changed_meanwhile.push(path);
                        continue;
                    }
                    let bytes = match content {
                        DiskContent::Text(text) => self.checkout(checkout_id)?.repo.filters()?.to_worktree(&path, text.as_bytes())?,
                        DiskContent::Blob(hash) => self.blob(hash).await?,
                    };
                    let written = match current {
                        Some(current) if current.hash == *blake3::hash(&bytes).as_bytes() && current.executable == executable => current,
                        _ => disk::write(&root, &path, &bytes, executable)?,
                    };
                    let entry = DiskEntry {
                        inode,
                        hash: written.hash,
                        size: written.size(),
                        mtime_ns: written.mtime_ns,
                        version,
                    };
                    self.checkouts
                        .get_mut(&checkout_id)
                        .expect("checked above")
                        .bridge
                        .record_written(&path, entry);
                    self.persist_entry(checkout_id, &path).await?;
                }
                DiskAction::Remove { path } => {
                    let current = disk::read(&root, &path)?;
                    let recorded = self.checkout(checkout_id)?.bridge.entries().get(&path).map(|entry| entry.hash);
                    if current.as_ref().is_some_and(|current| Some(current.hash) != recorded) {
                        changed_meanwhile.push(path);
                        continue;
                    }
                    disk::remove(&root, &path)?;
                    self.checkouts
                        .get_mut(&checkout_id)
                        .expect("checked above")
                        .bridge
                        .record_removed(&path);
                    self.store.remove_disk_file(checkout_id, &path).await?;
                }
            }
        }
        Ok(changed_meanwhile)
    }

    async fn persist_entry(&self, checkout: i64, path: &str) -> Result<(), CoreError> {
        match self.checkout(checkout)?.bridge.entries().get(path) {
            Some(entry) => {
                let file = silt_store::DiskFile {
                    inode: entry.inode.0,
                    hash: entry.hash,
                    size: entry.size,
                    mtime_ns: entry.mtime_ns,
                    version: entry.version.clone(),
                };
                self.store.put_disk_file(checkout, path, &file).await?;
            }
            None => self.store.remove_disk_file(checkout, path).await?,
        }
        Ok(())
    }

    fn author_for(&self, checkout: i64) -> Author {
        self.sessions
            .iter()
            .filter(|session| session.record.checkout_id == checkout)
            .find_map(|session| {
                session.open_tool.as_ref().map(|tool| Author::Agent {
                    agent: session.record.agent.clone(),
                    session: session.record.session_id.clone(),
                    tool_use_id: Some(tool.clone()),
                })
            })
            .unwrap_or(Author::User)
    }

    /// Returns a response to send now, or `None` when the reply is deferred.
    async fn hook(
        &mut self,
        event: &str,
        input: serde_json::Value,
        reply: oneshot::Sender<Response>,
    ) -> Option<(Response, oneshot::Sender<Response>)> {
        let input: claude::HookInput = match serde_json::from_value(input) {
            Ok(input) => input,
            Err(error) => return Some((Response::error(error), reply)),
        };
        let session_id = input.session_id.clone();
        let Some(thread) = self.session(&session_id).map(|session| session.record.thread) else {
            return Some((Response::ok(HookReply::default()), reply));
        };
        let result = match event {
            "UserPromptSubmit" => {
                let tailed = self.tail_session(&session_id).await;
                let recorded = match tailed {
                    Ok(()) => self.record_turn(&session_id, TurnMark::PromptSubmitted).await,
                    Err(error) => Err(error),
                };
                let reply_data = HookReply {
                    additional_context: self.take_notices(thread),
                };
                return Some((to_response(recorded.map(|()| reply_data)), reply));
            }
            "PreToolUse" => {
                let tool_use_id = input.tool_use_id.clone().unwrap_or_default();
                if let Some(session) = self.session_mut(&session_id) {
                    session.open_tool = Some(tool_use_id.clone());
                }
                let mark = TurnMark::ToolStarted {
                    tool_use_id,
                    tool: input.tool_name.clone().unwrap_or_default(),
                };
                self.record_turn(&session_id, mark).await
            }
            "PostToolUse" => {
                let events = self.events.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(TOOL_SETTLE).await;
                    let _ = events.send(Event::CloseTool { session_id, reply });
                });
                return None;
            }
            "SessionStart" if input.source.as_deref() == Some("compact") => {
                let tailed = self.tail_session(&session_id).await;
                let reply_data = HookReply {
                    additional_context: Some(format!(
                        "{} This session's thread is {thread}.",
                        silt_mcp::INSTRUCTIONS
                    )),
                };
                return Some((to_response(tailed.map(|()| reply_data)), reply));
            }
            "Stop" => {
                let tailed = self.tail_session(&session_id).await;
                match tailed {
                    Ok(()) => self.record_turn(&session_id, TurnMark::Stopped).await,
                    Err(error) => Err(error),
                }
            }
            _ => Ok(()),
        };
        Some((to_response(result.map(|()| HookReply::default())), reply))
    }

    async fn record_turn(&mut self, session_id: &str, mark: TurnMark) -> Result<(), CoreError> {
        let Some(session) = self.session(session_id) else {
            return Ok(());
        };
        let reference = SessionRef {
            agent: session.record.agent.clone(),
            id: session.record.session_id.clone(),
        };
        let (family, thread) = (session.family, session.record.thread);
        let author = agent_author(&reference);
        self.append_conv(family, thread, author, ConvOp::Turn { session: reference, mark }).await?;
        Ok(())
    }

    pub(crate) async fn append_conv(&mut self, family_id: FamilyId, thread: ThreadId, author: Author, op: ConvOp) -> Result<Delta, CoreError> {
        let actor = self.thread_actor(thread);
        let family = self.families.get_mut(&family_id).expect("session's family is loaded");
        let delta = family.delta(actor, thread, author, now_ms(), family.thread_head(thread), Op::Conv(op));
        family.apply(delta.clone(), &mut HashMap::new())?;
        self.store.append_deltas(family_id, std::slice::from_ref(&delta)).await?;
        Ok(delta)
    }

    /// Records new lines of a session's transcripts.
    async fn tail_session(&mut self, session_id: &str) -> Result<(), CoreError> {
        let Some(session) = self.session(session_id) else {
            return Ok(());
        };
        let mut files = vec![("main".to_owned(), session.transcript.clone())];
        let subagents = session.transcript.with_extension("").join("subagents");
        if let Ok(entries) = fs::read_dir(&subagents) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|extension| extension == "jsonl") {
                    let name = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
                    files.push((format!("subagent:{name}"), path));
                }
            }
        }
        for (key, path) in files {
            self.tail_file(session_id, &key, &path).await?;
        }
        self.attach_side_files(session_id).await
    }

    /// Records the session's tool-result files and subagent metadata as
    /// attachments when they are new or changed, so a restored session is
    /// complete.
    async fn attach_side_files(&mut self, session_id: &str) -> Result<(), CoreError> {
        let session = self.session(session_id).expect("caller checked");
        let session_dir = session.transcript.with_extension("");
        let mut changed = Vec::new();
        for (dir, keep) in [
            ("tool-results", (|_: &Path| true) as fn(&Path) -> bool),
            ("subagents", |path: &Path| path.extension().is_none_or(|extension| extension != "jsonl")),
        ] {
            let Ok(entries) = fs::read_dir(session_dir.join(dir)) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                if !metadata.is_file() || !keep(&path) {
                    continue;
                }
                let name = format!("{dir}/{}", entry.file_name().to_string_lossy());
                let stamp = (metadata.len(), metadata.modified()?);
                if session.attached.get(&name) != Some(&stamp) {
                    changed.push((name, path, stamp));
                }
            }
        }
        if changed.is_empty() {
            return Ok(());
        }
        let reference = SessionRef {
            agent: session.record.agent.clone(),
            id: session.record.session_id.clone(),
        };
        let (family, thread) = (session.family, session.record.thread);
        for (name, path, stamp) in changed {
            let bytes = fs::read(&path)?;
            let bytes = match String::from_utf8(bytes) {
                Ok(text) => self.redactor.replace(&text).into_bytes(),
                Err(error) => error.into_bytes(),
            };
            let blob = self.store.blobs().put(&bytes).await?;
            let known = self.local_log(family).iter().rev().find_map(|delta| match &delta.op {
                Op::Conv(ConvOp::Attachment {
                    session,
                    name: attached,
                    blob,
                }) if *session == reference && *attached == name => Some(*blob),
                _ => None,
            });
            if known != Some(blob) {
                let op = ConvOp::Attachment {
                    session: reference.clone(),
                    name: name.clone(),
                    blob,
                };
                self.append_conv(family, thread, agent_author(&reference), op).await?;
            }
            if let Some(session) = self.session_mut(session_id) {
                session.attached.insert(name, stamp);
            }
        }
        Ok(())
    }

    async fn tail_file(&mut self, session_id: &str, key: &str, path: &Path) -> Result<(), CoreError> {
        let session = self.session(session_id).expect("caller checked");
        let offset = session.offsets.get(key).map_or(0, |offset| offset.offset);
        let Ok(mut file) = fs::File::open(path) else {
            return Ok(());
        };
        if file.metadata()?.len() <= offset {
            return Ok(());
        }
        file.seek(SeekFrom::Start(offset))?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;
        let (lines, consumed) = complete_lines(&buffer);
        if lines.is_empty() {
            return Ok(());
        }
        let reference = SessionRef {
            agent: session.record.agent.clone(),
            id: session.record.session_id.clone(),
        };
        let (family, thread) = (session.family, session.record.thread);
        let source = match key.strip_prefix("subagent:") {
            Some(name) => SessionFile::Subagent(name.to_owned()),
            None => SessionFile::Main,
        };
        for line in lines {
            let entries = claude::normalize(&line);
            let delta = self
                .append_conv(
                    family,
                    thread,
                    agent_author(&reference),
                    ConvOp::Native {
                        session: reference.clone(),
                        file: source.clone(),
                        line,
                    },
                )
                .await?;
            for entry in entries {
                if let Some(text) = entry.searchable_text().filter(|text| !text.trim().is_empty()) {
                    self.store.index(thread, delta.span.first(), entry.kind(), &text).await?;
                }
            }
        }
        let advanced = SessionFileOffset {
            file: key.to_owned(),
            path: path.to_owned(),
            offset: offset + consumed as u64,
        };
        self.store.set_session_offset(&reference.agent, &reference.id, &advanced).await?;
        if let Some(session) = self.session_mut(session_id) {
            session.offsets.insert(key.to_owned(), advanced);
        }
        Ok(())
    }

    fn checkout(&self, id: i64) -> Result<&Checkout, CoreError> {
        self.checkouts
            .get(&id)
            .ok_or_else(|| CoreError::Invalid(format!("unknown checkout {id}")))
    }

    fn session(&self, session_id: &str) -> Option<&Session> {
        self.sessions.iter().find(|session| session.record.session_id == session_id)
    }

    fn session_mut(&mut self, session_id: &str) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|session| session.record.session_id == session_id)
    }

    pub(crate) fn families(&self) -> &HashMap<FamilyId, Family> {
        &self.families
    }

    pub(crate) fn thread_records(&self) -> &HashMap<ThreadId, ThreadRecord> {
        &self.threads
    }

    pub(crate) fn store(&self) -> &Store {
        &self.store
    }

    /// The thread whose id starts or ends with `prefix` (ids are shown by
    /// their last digits).
    pub(crate) fn find_thread(&self, prefix: &str) -> Result<ThreadId, CoreError> {
        let matches: Vec<ThreadId> = self
            .threads
            .keys()
            .filter(|thread| {
                let id = thread.to_string();
                id.starts_with(prefix) || id.ends_with(prefix)
            })
            .copied()
            .collect();
        match matches.as_slice() {
            [thread] => Ok(*thread),
            [] => Err(CoreError::Invalid(format!("no thread matches {prefix}"))),
            _ => Err(CoreError::Invalid(format!("{prefix} matches several threads"))),
        }
    }
}

impl Core {
    pub(crate) async fn append_session_started(
        &mut self,
        family: FamilyId,
        thread: ThreadId,
        session: SessionRef,
        cwd: &Path,
        checkout_root: &Path,
    ) -> Result<(), CoreError> {
        let author = agent_author(&session);
        let config_dir = claude::config_dir().ok_or_else(|| CoreError::Invalid("HOME is not set".into()))?;
        let dir = claude::session_paths(&config_dir, cwd, &session.id).session_dir;
        let op = ConvOp::SessionStarted {
            session: session.clone(),
            cwd: cwd.to_string_lossy().into_owned(),
            checkout_root: checkout_root.to_string_lossy().into_owned(),
        };
        self.append_conv(family, thread, author.clone(), op).await?;
        let op = ConvOp::SessionDir {
            session,
            dir: dir.to_string_lossy().into_owned(),
        };
        self.append_conv(family, thread, author, op).await?;
        Ok(())
    }

    /// Stores a bundle of `commit` and its history that origin lacks, so
    /// another machine can get it from the server (ADR 0009).
    pub(crate) async fn record_bundle(
        &mut self,
        family_id: FamilyId,
        thread: ThreadId,
        repo: &Repo,
        commit: GitOid,
    ) -> Result<(), CoreError> {
        let Some(bytes) = repo.bundle(commit)? else {
            return Ok(());
        };
        self.push_to_mirror(family_id, thread, repo, commit);
        let blob = self.store.blobs().put(&bytes).await?;
        let actor = self.thread_actor(thread);
        let family = self.families.get_mut(&family_id).expect("thread's family is loaded");
        let op = Op::Git(GitOp::Bundle {
            commits: vec![commit],
            blob,
        });
        let delta = family.delta(actor, thread, Author::Import, now_ms(), family.thread_head(thread), op);
        family.apply(delta.clone(), &mut HashMap::new())?;
        self.store.append_deltas(family_id, &[delta]).await?;
        Ok(())
    }

    /// Records commits made in checkouts since the last look, with bundles.
    pub(crate) async fn check_commits(&mut self) -> Result<(), CoreError> {
        let ids: Vec<i64> = self.checkouts.keys().copied().collect();
        for id in ids {
            let checkout = &self.checkouts[&id];
            let Some(head) = checkout.repo.head_commit()? else {
                continue;
            };
            let previous = self.heads.insert(id, head);
            if previous.is_none() || previous == Some(head) {
                continue;
            }
            let (family_id, thread) = (checkout.family, checkout.record.thread);
            let branch = checkout.repo.head_branch()?;
            let repo = Repo::discover(&checkout.record.path)?;
            let actor = self.thread_actor(thread);
            let family = self.families.get_mut(&family_id).expect("checkout's family is loaded");
            let op = Op::Git(GitOp::Committed { commit: head, branch });
            let delta = family.delta(actor, thread, Author::User, now_ms(), family.thread_head(thread), op);
            family.apply(delta.clone(), &mut HashMap::new())?;
            self.store.append_deltas(family_id, &[delta]).await?;
            repo.protect(&thread.to_string(), head)?;
            self.record_bundle(family_id, thread, &repo, head).await?;
        }
        Ok(())
    }

    /// Threads whose lease moved to another device stop taking writes from
    /// this one.
    /// Threads whose lease moved to another device stop taking writes from
    /// this one. `before` holds the heads the moved threads had here before
    /// `deltas` arrived, which is where this device's work continues.
    pub(crate) async fn lease_changes(&mut self, deltas: &[Delta], before: &HashMap<ThreadId, Frontier>) -> Result<(), CoreError> {
        let Some(device) = self.remote.as_ref().map(|remote| remote.config.device) else {
            return Ok(());
        };
        for delta in deltas {
            if let Op::Lease { device: holder, .. } = delta.op
                && holder != device
            {
                self.thread_moved(delta.thread, before.get(&delta.thread).cloned()).await?;
            }
        }
        Ok(())
    }

    /// Another device took `thread`'s lease: checkouts of it here continue in
    /// a fork from `at` (by default the thread's head), so nothing written
    /// here lands in the moved thread and nothing from there lands here.
    pub(crate) async fn thread_moved(&mut self, thread: ThreadId, at: Option<Frontier>) -> Result<(), CoreError> {
        let ids: Vec<i64> = self
            .checkouts
            .iter()
            .filter(|(_, checkout)| checkout.record.thread == thread)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            let fork = self.fork_thread_at(thread, at.clone()).await?;
            self.store.move_checkout(id, fork).await?;
            let checkout = self.checkouts.get_mut(&id).expect("listed above");
            checkout.record.thread = fork;
            checkout.bridge = Bridge::new(checkout.record.actor, fork, checkout.bridge.entries().clone());
            for session in self.sessions.iter_mut().filter(|session| session.record.checkout_id == id) {
                session.record.thread = fork;
            }
            warn!(%thread, %fork, "the thread moved to another device; continuing here in a fork");
        }
        Ok(())
    }

    /// Pushes a commit origin lacks to the server's git mirror, in the
    /// background; the bundle recorded next to it covers a failure.
    fn push_to_mirror(&self, family: FamilyId, thread: ThreadId, repo: &Repo, commit: GitOid) {
        let Some(remote) = &self.remote else {
            return;
        };
        let (url, headers) = remote.git_mirror(family);
        let root = repo.root().to_owned();
        tokio::task::spawn_blocking(move || {
            let pushed = Repo::discover(&root)
                .map_err(|error| error.to_string())
                .and_then(|repo| {
                    repo.push_to_mirror(&url, &headers, commit, &thread.to_string())
                        .map_err(|error| error.to_string())
                });
            if let Err(error) = pushed {
                warn!(%error, "could not push to the server's git mirror");
            }
        });
    }

    /// The web UI's URL, starting it first if needed.
    async fn open_web(&mut self) -> Result<String, CoreError> {
        if self.web.is_none() {
            self.web = Some(crate::web::start(self.events.clone()).await?);
        }
        Ok(self.web.as_ref().expect("started above").url())
    }

    /// The actor for this daemon's own deltas in one thread. Threads of a
    /// family grow concurrently, and an actor's deltas must form one chain.
    pub(crate) fn mount(&self, source: &Repo) -> Mount {
        let project = match source.remote_url("origin").ok().flatten() {
            Some(url) => silt_git::normalize_remote(&url),
            None => source.root().to_string_lossy().into_owned(),
        };
        let project: String = project
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() || ch == '.' { ch } else { '-' })
            .collect();
        Mount {
            source: source.root().to_owned(),
            shared: self.data_dir.join("linked").join(project.trim_matches('-')),
        }
    }

    pub(crate) fn thread_actor(&self, thread: ThreadId) -> ActorId {
        self.actor.derive(thread.0)
    }

    /// This device's user on the server; empty without one.
    pub(crate) fn own_user(&self) -> String {
        self.remote.as_ref().map(|remote| remote.config.user.clone()).unwrap_or_default()
    }

    /// Starts a new thread in `from`'s family at `from`'s current head.
    pub(crate) async fn fork_thread(&mut self, from: ThreadId) -> Result<ThreadId, CoreError> {
        self.fork_thread_at(from, None).await
    }

    /// Starts a new thread in `from`'s family at `at`, by default `from`'s
    /// current head.
    pub(crate) async fn fork_thread_at(&mut self, from: ThreadId, at: Option<Frontier>) -> Result<ThreadId, CoreError> {
        let source = self.threads[&from].clone();
        let fork = ThreadId(uuid::Uuid::now_v7().as_u128());
        let record = ThreadRecord {
            id: fork,
            created_ms: now_ms(),
            owner: self.own_user(),
            ..source.clone()
        };
        self.store.create_thread(&record).await?;
        self.register_thread(&record);
        self.threads.insert(fork, record);
        let actor = self.thread_actor(fork);
        let family = self.families.get_mut(&source.family).expect("forked thread's family is loaded");
        let parents = at.unwrap_or_else(|| family.thread_head(from));
        let delta = family.delta(actor, fork, Author::User, now_ms(), parents, Op::Fork { from });
        family.apply(delta.clone(), &mut HashMap::new())?;
        self.store.append_deltas(source.family, &[delta]).await?;
        Ok(fork)
    }
}

fn agent_author(session: &SessionRef) -> Author {
    Author::Agent {
        agent: session.agent.clone(),
        session: session.id.clone(),
        tool_use_id: None,
    }
}

pub(crate) fn touched_inode(delta: &Delta) -> Option<InodeId> {
    match &delta.op {
        Op::Create { .. } => Some(InodeId(delta.span.first())),
        Op::Move { inode, .. }
        | Op::Delete { inode }
        | Op::Text { inode, .. }
        | Op::SetContent { inode, .. }
        | Op::SetExecutable { inode, .. } => Some(*inode),
        Op::Conv(_)
        | Op::Git(_)
        | Op::Lease { .. }
        | Op::Fork { .. }
        | Op::Merge { .. }
        | Op::Mr(_)
        | Op::Comment(_) => None,
    }
}

fn to_response<T: serde::Serialize>(result: Result<T, CoreError>) -> Response {
    match result {
        Ok(value) => Response::ok(value),
        Err(error) => Response::error(error),
    }
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
}
