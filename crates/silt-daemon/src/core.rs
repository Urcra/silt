use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use silt_checkout::{Bridge, DiskAction, DiskContent, DiskEntry, DiskFile};
use silt_crdt::InodeId;
use silt_git::Repo;
use silt_log::{Author, BaseText, ConvOp, Delta, Family, GitOp, Op};
use silt_store::{CheckoutRecord, Store, ThreadRecord};
use silt_types::{ActorId, FamilyId, Frontier, GitOid, ThreadId};
use silt_proto::Listing;
use silt_redact::Redactor;
use silt_sync::{ClientSync, Outcome};
use tokio::sync::{mpsc, oneshot};
use tracing::warn;

use crate::disk::{self, RawFile};
use crate::lifecycle::Lifecycle;
use crate::protocol::{Mount, Request, Response};
use crate::sessions::{PendingSession, Session};
use crate::remote::{self, Remote};
use crate::watch::{self, Watcher};

pub enum Event {
    Request(Request, oneshot::Sender<Response>),
    /// Paths in a checkout may have changed, as the watcher noticed at
    /// `noticed`; `settled` once the agents' transcripts have had time to
    /// catch up.
    Changed { checkout: i64, paths: Vec<PathBuf>, noticed: Instant, settled: bool },
    Tick,
    CloseTool { session_id: String, reply: oneshot::Sender<Response> },
    Pushed { family: FamilyId, generation: Option<u64>, outcome: Outcome, error: Option<String> },
    Pulled { family: FamilyId, generation: Option<u64>, outcome: Outcome, error: Option<String> },
    /// A listing, and when it was asked for.
    Listed(Result<Listing, String>, Instant),
    Discovered { token: u64, found: Option<silt_agents::Found> },
    Snapshotted { session_id: String, result: Result<Vec<String>, String> },
    Acp { thread: ThreadId, run: u64, event: crate::acp::AcpEvent },
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
    pub(crate) pending: Vec<PendingSession>,
    /// Readers for indexing what other machines record.
    pub(crate) index_transcripts: silt_agents::Transcripts,
    pub(crate) next_token: u64,
    /// When the watcher noticed the changes being taken in, for crediting
    /// them to the tool that was running then.
    pub(crate) change_noticed: Option<Instant>,
    /// Set while a new thread takes in its clone's uncommitted changes,
    /// which are part of where it starts, not edits made in it.
    pub(crate) importing: bool,
    /// Threads in the trash, hidden until restored or purged.
    pub(crate) deleted: HashSet<ThreadId>,
    /// When this machine last moved each thread into or out of the trash;
    /// listings asked for before that are out of date for it.
    pub(crate) trash_changed: HashMap<ThreadId, Instant>,
    /// Each thread's collaborators, from the server's listing.
    pub(crate) collaborators: HashMap<ThreadId, Vec<String>>,
    /// Clones this machine knows by repository, for projecting families
    /// that started elsewhere.
    pub(crate) clones: HashMap<String, PathBuf>,
    /// Agents the web UI started, by thread.
    pub(crate) acp: HashMap<ThreadId, crate::acp::AcpRun>,
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
            pending: Vec::new(),
            index_transcripts: silt_agents::Transcripts::default(),
            next_token: 0,
            change_noticed: None,
            importing: false,
            deleted: HashSet::new(),
            trash_changed: HashMap::new(),
            collaborators: HashMap::new(),
            clones: HashMap::new(),
            acp: HashMap::new(),
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
        core.deleted = core.store.deleted_threads().await?.into_iter().collect();
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
        core.load_sessions().await?;
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
                existing,
            } => to_response(self.start_session(&cwd, &agent, session_id.as_deref(), isolated, existing).await),
            Request::Hook { event, input } => match self.hook(&event, input, reply).await {
                Some((response, reply)) => {
                    let _ = reply.send(response);
                    return;
                }
                None => return,
            },
            Request::EndSession { thread } => match self.find_thread(&thread) {
                Ok(thread) => to_response(self.end_session(thread).await),
                Err(error) => Response::error(error),
            },
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
            Request::Resume { thread, cwd, here, agent } => to_response(self.resume(&thread, &cwd, here, agent.as_deref()).await),
            Request::Fork { thread, cwd, here, agent } => to_response(self.fork(&thread, &cwd, here, agent.as_deref()).await),
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
            Request::OpenWeb { cwd } => {
                if let Some(cwd) = cwd {
                    self.remember_clone(&cwd);
                }
                to_response(self.open_web().await)
            }
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
            Request::Inbox { all } => to_response(self.inbox(all).await),
            Request::InboxRead { read } => to_response(self.inbox_read(&read).await.map(|()| "read")),
            Request::ThreadDelete { thread } => to_response(self.thread_lifecycle(&thread, Lifecycle::Delete).await),
            Request::ThreadRestore { thread } => to_response(self.thread_lifecycle(&thread, Lifecycle::Restore).await),
            Request::ThreadPurge { thread } => to_response(self.thread_lifecycle(&thread, Lifecycle::Purge).await),
            Request::Trash => Response::ok(self.trash()),
            Request::ThreadShare { thread, user, remove } => to_response(self.share(&thread, &user, remove).await),
            Request::Replica { thread, after } => to_response(self.replica_deltas(&thread, after).await),
            Request::GitText { thread, oid } => to_response(self.git_text(&thread, &oid)),
            Request::BrowserDeltas { thread, deltas } => to_response(self.receive_from_browser(&thread, &deltas).await),
            Request::Live { thread } => to_response(self.live_state(&thread)),
            Request::Presence {
                thread,
                file,
                draft,
                leaving,
            } => to_response(self.presence(&thread, file, draft, leaving).await),
            Request::AcpAgents => Response::ok(self.acp_agents()),
            Request::AcpStart { thread, clone, agent } => {
                to_response(self.acp_start(thread.as_deref(), clone.as_deref(), &agent).await)
            }
            Request::AcpPrompt { thread, text } => to_response(self.acp_prompt(&thread, &text).await),
            Request::AcpCancel { thread } => to_response(self.acp_command(&thread, silt_acp::Command::Cancel)),
            Request::AcpAnswer { thread, id, option } => {
                to_response(self.acp_command(&thread, silt_acp::Command::Answer { id, option }))
            }
            Request::AcpStop { thread } => to_response(self.acp_command(&thread, silt_acp::Command::Stop)),
            Request::Shutdown => Response::ok("shutting down"),
        };
        let _ = reply.send(response);
    }

    pub async fn changed(&mut self, checkout: i64, paths: Vec<PathBuf>, noticed: Instant, settled: bool) {
        if !settled && self.marks_from_transcripts(checkout) {
            self.settle_changes(checkout, paths, noticed);
            return;
        }
        self.tail_marked_sessions(checkout).await;
        self.change_noticed = Some(noticed);
        if let Err(error) = self.ingest_paths(checkout, paths).await {
            warn!(%error, checkout, "could not record changes");
        }
        self.change_noticed = None;
    }

    pub async fn tick(&mut self) {
        self.drive_sync();
        if let Err(error) = self.check_commits().await {
            warn!(%error, "could not record new commits");
        }
        self.poll_sessions();
        let ids: Vec<String> = self.sessions.iter().map(|session| session.record.session_id.clone()).collect();
        for id in ids {
            if let Err(error) = self.tail_session(&id).await {
                warn!(%error, session = id, "could not read the session transcript");
            }
        }
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
            let _ = events.send(Event::Changed {
                checkout: id,
                paths,
                noticed: Instant::now(),
                settled: false,
            });
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

    pub(crate) async fn append_conv(&mut self, family_id: FamilyId, thread: ThreadId, author: Author, op: ConvOp) -> Result<Delta, CoreError> {
        let actor = self.thread_actor(thread);
        let family = self.families.get_mut(&family_id).expect("session's family is loaded");
        let delta = family.delta(actor, thread, author, now_ms(), family.thread_head(thread), Op::Conv(op));
        family.apply(delta.clone(), &mut HashMap::new())?;
        self.store.append_deltas(family_id, std::slice::from_ref(&delta)).await?;
        Ok(delta)
    }

    fn checkout(&self, id: i64) -> Result<&Checkout, CoreError> {
        self.checkouts
            .get(&id)
            .ok_or_else(|| CoreError::Invalid(format!("unknown checkout {id}")))
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
    async fn inbox(&self, all: bool) -> Result<Vec<crate::protocol::InboxEntry>, CoreError> {
        let remote = self.remote.as_ref().ok_or_else(|| CoreError::Invalid("not connected to a server; use `silt auth login`".into()))?;
        let reply = remote.inbox(all).await.map_err(|error| CoreError::Invalid(error.to_string()))?;
        Ok(reply
            .notifications
            .into_iter()
            .map(|notification| crate::protocol::InboxEntry {
                id: notification.id,
                kind: notification.kind,
                thread: notification.thread.to_string(),
                actor: notification.actor,
                text: notification.text,
                created_ms: notification.created_ms,
                read: notification.read,
            })
            .collect())
    }

    async fn inbox_read(&self, read: &silt_proto::InboxRead) -> Result<(), CoreError> {
        let remote = self.remote.as_ref().ok_or_else(|| CoreError::Invalid("not connected to a server; use `silt auth login`".into()))?;
        remote.inbox_read(read).await.map_err(|error| CoreError::Invalid(error.to_string()))
    }

    async fn presence(&self, selector: &str, file: Option<String>, draft: String, leaving: bool) -> Result<silt_proto::PresenceReply, CoreError> {
        let thread = self.find_thread(selector)?;
        let Some(remote) = &self.remote else {
            return Ok(silt_proto::PresenceReply::default());
        };
        let update = silt_proto::PresenceUpdate {
            thread,
            file,
            draft,
            leaving,
            agent: self.agent_status(thread).filter(|status| status.state != "exited").map(|status| silt_proto::AgentActivity {
                name: status.agent,
                state: status.state,
                streaming: status.streaming,
            }),
        };
        remote.presence(&update).await.map_err(|error| CoreError::Invalid(error.to_string()))
    }

    /// Teammates' current drafts for the thread's next prompt, for its
    /// agent (ADR 0014).
    pub(crate) async fn teammates_drafts(&self, thread: ThreadId) -> Option<String> {
        let drafts: Vec<String> = self
            .teammates_draft_list(thread)
            .await
            .into_iter()
            .map(|(user, draft)| format!("- {user}: {draft}"))
            .collect();
        (!drafts.is_empty()).then(|| format!("Teammates watching this thread are drafting too; consider their drafts with this prompt:\n{}", drafts.join("\n")))
    }

    /// Who else is drafting a prompt for the thread, and what.
    pub(crate) async fn teammates_draft_list(&self, thread: ThreadId) -> Vec<(String, String)> {
        let Some(remote) = self.remote.as_ref() else {
            return Vec::new();
        };
        let Ok(reply) = remote.present(thread).await else {
            return Vec::new();
        };
        reply
            .people
            .into_iter()
            .filter(|person| !person.draft.trim().is_empty())
            .map(|person| (person.user, person.draft.trim().to_owned()))
            .collect()
    }

    fn remember_clone(&mut self, cwd: &Path) {
        let Ok(repo) = Repo::discover(cwd) else {
            return;
        };
        if let Ok(Some(url)) = repo.remote_url("origin") {
            self.clones.insert(silt_git::normalize_remote(&url), repo.root().to_owned());
        }
    }

    /// Whether this machine's user may write into `thread`: its owner or a
    /// collaborator, or anyone when there is no server.
    pub(crate) fn may_write(&self, thread: ThreadId) -> bool {
        let user = self.own_user();
        let Some(record) = self.threads.get(&thread) else {
            return false;
        };
        user.is_empty() || record.owner.is_empty() || record.owner == user
            || self.collaborators.get(&thread).is_some_and(|names| names.contains(&user))
    }

    async fn share(&mut self, selector: &str, user: &str, remove: bool) -> Result<String, CoreError> {
        let thread = self.find_thread(selector)?;
        let remote = self.remote.as_ref().ok_or_else(|| CoreError::Invalid("sharing needs a server; use `silt auth login`".into()))?;
        remote.share(thread, user, remove).await.map_err(|error| CoreError::Invalid(error.to_string()))?;
        let names = self.collaborators.entry(thread).or_default();
        names.retain(|name| name != user);
        if !remove {
            names.push(user.to_owned());
        }
        let short = crate::forks::short(thread);
        Ok(if remove {
            format!("{user} no longer writes into thread {short}")
        } else {
            format!("{user} can now edit thread {short} with you")
        })
    }

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

pub(crate) fn to_response<T: serde::Serialize>(result: Result<T, CoreError>) -> Response {
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
