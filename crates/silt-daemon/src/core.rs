use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use silt_checkout::{Bridge, DiskAction, DiskContent, DiskEntry, DiskFile};
use silt_crdt::InodeId;
use silt_git::Repo;
use silt_log::{Author, BaseText, ConvOp, Delta, Family, GitOp, Op, ThreadState};
use silt_proto::{Listing, Visibility};
use silt_redact::Redactor;
use silt_store::{CheckoutId, CheckoutRecord, SearchRow, SessionRead, Store, ThreadRecord};
use silt_sync::{ClientSync, Outcome};
use silt_types::{ActorId, BlobHash, DeviceId, FamilyId, Frontier, GitOid, ThreadId};
use tokio::sync::{mpsc, oneshot};
use tracing::warn;

use crate::disk::{self, RawFile};
use crate::lifecycle::Lifecycle;
use crate::load::{self, Loaded};
use crate::protocol::{Mount, Request, Response};
use crate::remote::{self, Remote};
use crate::sessions::{self, PendingSession, Session};
use crate::watch::{self, Watcher};

pub enum Event {
    Request(Request, oneshot::Sender<Response>),
    /// Paths in a checkout may have changed, as the watcher noticed at
    /// `noticed`; `settled` once the agents' transcripts have had time to
    /// catch up.
    Changed {
        checkout: CheckoutId,
        paths: Vec<PathBuf>,
        noticed: Instant,
        settled: bool,
    },
    Tick,
    CloseTool {
        session_id: String,
        reply: oneshot::Sender<Response>,
    },
    /// A push's answer, from sync epoch `epoch`.
    Pushed {
        family: FamilyId,
        epoch: u64,
        generation: Option<u64>,
        outcome: Outcome,
        error: Option<String>,
    },
    Pulled {
        family: FamilyId,
        epoch: u64,
        generation: Option<u64>,
        outcome: Outcome,
        error: Option<String>,
    },
    /// A listing, and when it was asked for.
    Listed(Result<Listing, String>, Instant),
    Discovered {
        token: u64,
        found: Option<silt_agents::Found>,
    },
    Snapshotted {
        session_id: String,
        result: Result<Vec<String>, String>,
    },
    Acp {
        thread: ThreadId,
        run: u64,
        event: crate::acp::AcpEvent,
    },
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
    #[error("could not {doing} on the server")]
    Server {
        doing: &'static str,
        #[source]
        source: crate::remote::RemoteError,
    },
    #[error("a background task stopped")]
    Background(#[from] tokio::task::JoinError),
    #[error(transparent)]
    Text(#[from] silt_crdt::TextError),
    #[error("could not encode a snapshot of a family")]
    Encode(#[source] postcard::Error),
    #[error("could not {doing}")]
    Agent {
        doing: &'static str,
        #[source]
        source: silt_agents::AgentError,
    },
    #[error("{0}")]
    Invalid(String),
}

/// A working directory with its symlinks resolved, as checkout paths are
/// recorded, so the two compare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CanonicalCwd(PathBuf);

impl CanonicalCwd {
    pub(crate) fn new(cwd: &Path) -> std::io::Result<Self> {
        cwd.canonicalize().map(Self)
    }
}

impl std::ops::Deref for CanonicalCwd {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
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
    pub(crate) checkouts: BTreeMap<CheckoutId, Checkout>,
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
    /// Families whose next pull waits until then, after a delta that
    /// couldn't apply yet.
    pub(crate) pull_paused: HashMap<FamilyId, Instant>,
    /// Agents the web UI started, by thread.
    pub(crate) acp: HashMap<ThreadId, crate::acp::AcpRun>,
    pub(crate) events: mpsc::UnboundedSender<Event>,
    pub(crate) data_dir: PathBuf,
    pub(crate) remote: Option<Remote>,
    pub(crate) syncs: HashMap<FamilyId, ClientSync>,
    /// Bumped when a family's sync state is replaced, so answers to requests
    /// made before aren't taken as answers to new ones.
    pub(crate) sync_epochs: HashMap<FamilyId, u64>,
    pub(crate) redactor: Redactor,
    /// Secrets already remembered in the store, which keeps redacting them
    /// after they leave the environment (ADR 0019).
    pub(crate) known_secrets: HashSet<String>,
    pub(crate) last_listing: Option<Instant>,
    pub(crate) sync_error: Option<String>,
    /// The HEAD commit each checkout was last seen at.
    pub(crate) heads: HashMap<CheckoutId, GitOid>,
    /// How much of each family's log its stored snapshot covers.
    pub(crate) snapshotted: HashMap<FamilyId, usize>,
    pub(crate) web: Option<crate::web::WebInfo>,
    /// What recovery forks this daemon made, for `silt auth status`.
    pub(crate) recoveries: Vec<String>,
    /// Blobs this daemon already sent the server, so a retried push doesn't
    /// send them again.
    pub(crate) uploaded: std::sync::Arc<std::sync::Mutex<HashSet<BlobHash>>>,
    pub(crate) registry_checked: bool,
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
            pull_paused: HashMap::new(),
            acp: HashMap::new(),
            events,
            data_dir: data_dir.to_owned(),
            remote: remote::load_config(data_dir).map(Remote::new),
            syncs: HashMap::new(),
            sync_epochs: HashMap::new(),
            redactor: redactor(data_dir),
            known_secrets: HashSet::new(),
            last_listing: None,
            sync_error: None,
            heads: HashMap::new(),
            snapshotted: HashMap::new(),
            web: None,
            recoveries: Vec::new(),
            uploaded: std::sync::Arc::default(),
            registry_checked: false,
        };
        for thread in core.store.threads().await? {
            core.threads.insert(thread.id, thread);
        }
        core.deleted = core.store.deleted_threads().await?.into_iter().collect();
        // Remembered secrets are redacted too, so deltas recorded while a
        // secret was in the environment stay redacted after it's gone.
        let stored = core.store.secrets().await?;
        core.store.learn_secrets(core.redactor.secrets()).await?;
        core.redactor = core.redactor.clone().with(stored.iter().cloned());
        core.known_secrets = stored.into_iter().chain(core.redactor.secrets().iter().cloned()).collect();
        core.clones = core.store.clones().await?.into_iter().filter(|(_, path)| path.join(".git").exists()).collect();
        let families: HashSet<FamilyId> = core.threads.values().map(|thread| thread.family).collect();
        for family_id in families {
            let repo = core
                .threads
                .values()
                .filter(|thread| thread.family == family_id && !thread.repo_root.as_os_str().is_empty())
                .find_map(|thread| Repo::discover(&thread.repo_root).ok());
            core.take_loaded(family_id, load::family(&core.store, family_id, repo.as_ref()).await?);
            core.snapshot_if_due(family_id).await;
            let state = core.store.sync_state(family_id).await?;
            core.syncs
                .insert(family_id, ClientSync::new(state.generation, state.pushed, state.pulled, state.last_pulled, state.view));
        }
        for record in core.store.checkouts().await? {
            if !record.path.exists() {
                continue;
            }
            let entries = stored_entries(&core.store, record.id).await?;
            core.start_checkout(record, entries)?;
        }
        core.load_sessions().await?;
        // Queued rather than done here, so the socket is already taking
        // requests while they run; they still come before any request.
        for checkout in core.checkouts.values() {
            let _ = core.events.send(Event::Changed {
                checkout: checkout.record.id,
                paths: vec![checkout.record.path.clone()],
                noticed: Instant::now(),
                settled: true,
            });
        }
        Ok(core)
    }

    /// Handles a request. Some answer `reply` later, off the event loop.
    pub async fn request(&mut self, request: Request, reply: oneshot::Sender<Response>) {
        let response = match request {
            Request::Ping => Response::ok("pong"),
            Request::AgentHomes => {
                let homes: Vec<(&str, Option<String>)> =
                    silt_agents::HOME_VARIABLES.iter().map(|name| (*name, std::env::var(name).ok())).collect();
                Response::ok(homes)
            }
            Request::StartSession { cwd, agent, session_id, isolated, existing } => {
                to_response(self.start_session(&cwd, &agent, session_id.as_deref(), isolated, existing).await)
            }
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
                let fresh = redactor(&self.data_dir);
                let secrets = fresh.secrets().to_vec();
                self.redactor = self.redactor.clone().with(secrets.iter().cloned());
                self.learn_secrets(secrets);
                for family in self.syncs.keys().copied().collect::<Vec<_>>() {
                    *self.sync_epochs.entry(family).or_default() += 1;
                }
                self.syncs.clear();
                for family in self.families.keys().chain(self.raw.keys()).copied().collect::<Vec<_>>() {
                    let state = self.store.sync_state(family).await.unwrap_or_default();
                    let sync = ClientSync::new(state.generation, state.pushed, state.pulled, state.last_pulled, state.view);
                    self.replace_sync(family, sync);
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
            Request::Fork { thread, cwd, here, agent, private } => {
                to_response(self.fork(&thread, &cwd, here, agent.as_deref(), private).await)
            }
            Request::Pull { thread, cwd } => to_response(self.pull(thread.as_deref(), &cwd).await),
            Request::MrCreate { thread, cwd, title, summary } => {
                to_response(self.mr_create(thread.as_deref(), &cwd, title, summary).await)
            }
            Request::MrList { thread } => to_response(self.mr_list(thread.as_deref())),
            Request::MrShow { mr } => to_response(self.mr_show(&mr)),
            Request::MrApprove { mr } => to_response(self.mr_approve(&mr).await),
            Request::MrDecline { mr, reason } => to_response(self.mr_decline(&mr, reason).await),
            Request::Turns { thread } => to_response(self.find_thread(&thread).map(|thread| self.turns(thread))),
            Request::OpenWeb { cwd, once } => {
                if let Some(cwd) = cwd {
                    self.remember_clone(&cwd).await;
                }
                to_response(self.open_web(once).await)
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
            Request::ResolveComment { thread, comment, cwd } => to_response(self.resolve_comment(&thread, &comment, &cwd).await),
            Request::SetVisibility { cwd, thread, visibility } => {
                to_response(self.set_visibility(&cwd, thread.as_deref(), visibility).await)
            }
            Request::Inbox { all } => return answer_later(reply, self.inbox(all)),
            Request::InboxRead { read } => return answer_later(reply, self.inbox_read(read)),
            Request::ThreadDelete { thread } => to_response(self.thread_lifecycle(&thread, Lifecycle::Delete).await),
            Request::ThreadRestore { thread } => to_response(self.thread_lifecycle(&thread, Lifecycle::Restore).await),
            Request::ThreadPurge { thread } => to_response(self.thread_lifecycle(&thread, Lifecycle::Purge).await),
            Request::Trash => Response::ok(self.trash()),
            Request::ThreadShare { thread, user, remove } => to_response(self.share(&thread, &user, remove).await),
            Request::Replica { thread, after } => to_response(self.replica_deltas(&thread, after).await),
            Request::GitText { thread, oid } => to_response(self.git_text(&thread, &oid)),
            Request::BrowserDeltas { thread, deltas } => to_response(self.receive_from_browser(&thread, &deltas).await),
            Request::Live { thread } => to_response(self.live_state(&thread)),
            Request::Presence { thread, file, line, draft, leaving } => {
                return answer_later(reply, self.presence(&thread, file, line, draft, leaving));
            }
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

    pub async fn changed(&mut self, checkout: CheckoutId, paths: Vec<PathBuf>, noticed: Instant, settled: bool) {
        if !settled && self.marks_from_transcripts(checkout) {
            self.settle_changes(checkout, paths, noticed);
            return;
        }
        self.tail_marked_sessions(checkout).await;
        self.change_noticed = Some(noticed);
        if let Err(error) = self.ingest_paths(checkout, paths).await {
            warn!(%error, %checkout, "could not record changes");
        }
        self.change_noticed = None;
    }

    pub async fn tick(&mut self) {
        self.drive_sync();
        if let Err(error) = self.check_commits().await {
            warn!(%error, "could not record new commits");
        }
        self.poll_sessions();
        let ids: Vec<String> = self
            .sessions
            .iter()
            .filter(|session| session.live || sessions::transcript_grew(session))
            .map(|session| session.record.session_id.clone())
            .collect();
        for id in ids {
            if let Err(error) = self.tail_session(&id).await {
                warn!(%error, session = id, "could not read the session transcript");
            }
        }
    }

    pub(crate) fn start_checkout(
        &mut self,
        record: CheckoutRecord,
        entries: BTreeMap<String, DiskEntry>,
    ) -> Result<(), CoreError> {
        let family = self
            .threads
            .get(&record.thread)
            .map(|thread| thread.family)
            .ok_or_else(|| CoreError::Invalid("checkout of an unknown thread".into()))?;
        let repo = Repo::discover(&record.path)?;
        let events = self.events.clone();
        let id = record.id;
        let watcher = watch::watch(&record.path, move |paths| {
            let _ = events.send(Event::Changed { checkout: id, paths, noticed: Instant::now(), settled: false });
        })?;
        let bridge = Bridge::new(record.actor, record.thread, entries);
        self.checkouts.insert(record.id, Checkout { record, family, repo, bridge, _watcher: Some(watcher) });
        Ok(())
    }

    pub(crate) async fn rescan(&mut self, checkout: CheckoutId) -> Result<(), CoreError> {
        let root = self.checkout(checkout)?.record.path.clone();
        self.ingest_paths(checkout, vec![root]).await
    }

    /// Records changes under `paths` (absolute; directories are scanned).
    async fn ingest_paths(&mut self, checkout_id: CheckoutId, paths: Vec<PathBuf>) -> Result<(), CoreError> {
        let checkout = self.checkout(checkout_id)?;
        let root = checkout.record.path.clone();
        if paths.iter().any(|path| *path == root || path.file_name() == Some(".gitattributes".as_ref())) {
            checkout.repo.attributes_changed();
        }
        let mut probes = Vec::new();
        for path in paths {
            let Ok(relative) = path.strip_prefix(&root) else {
                continue;
            };
            let relative = relative.to_string_lossy().replace('\\', "/");
            probes.push(if path.is_dir() {
                let files = self.files_under(checkout_id, &relative)?;
                if relative.is_empty() { Probe::Root(files) } else { Probe::Dir(files) }
            } else if relative.is_empty() {
                continue;
            } else if path.exists() {
                Probe::File(relative)
            } else {
                Probe::Gone(relative)
            });
        }
        let thread_paths =
            if probes.iter().any(|probe| matches!(probe, Probe::Root(_))) { self.family_paths(checkout_id) } else { Vec::new() };
        let recorded = self.checkout(checkout_id)?.bridge.entries().keys();
        let mut worklist = self.trackable(checkout_id, candidates(probes, recorded, &thread_paths))?;
        for path in self.ingest_renames(checkout_id, &worklist).await? {
            worklist.retain(|candidate| *candidate != path);
        }
        let mut rounds = 0;
        while !worklist.is_empty() && rounds < 8 {
            rounds += 1;
            let mut retry = Vec::new();
            for path in std::mem::take(&mut worklist) {
                // One file that can't be recorded must not keep the rest
                // of the batch from being recorded.
                match self.ingest_file(checkout_id, &path).await {
                    Ok(again) => retry.extend(again),
                    Err(error) => warn!(%error, path, "could not record a change to a file"),
                }
            }
            worklist = self.trackable(checkout_id, retry)?;
        }
        Ok(())
    }

    /// Records each vanished file whose exact bytes reappeared at a new path
    /// in the same batch as a move. Returns the paths it handled.
    async fn ingest_renames(&mut self, checkout_id: CheckoutId, paths: &[String]) -> Result<Vec<String>, CoreError> {
        let root = self.checkout(checkout_id)?.record.path.clone();
        let entries = self.checkout(checkout_id)?.bridge.entries().clone();
        let mut vanished: BTreeMap<BlobHash, Vec<&String>> = BTreeMap::new();
        let mut appeared = Vec::new();
        for path in paths {
            // An unreadable file is left to `ingest_file`, which reports it.
            match (entries.get(path), disk::read(&root, path)) {
                (Some(entry), Ok(None)) => vanished.entry(entry.hash).or_default().push(path),
                (None, Ok(Some(raw))) => appeared.push((path, raw)),
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
            // A move that can't be recorded is recorded as a delete and a
            // new file instead, by `ingest_file`.
            let renamed = checkout.bridge.rename(family, &mut RepoBases(&checkout.repo), from, to, &disk_file, &author, now_ms());
            let ingested = match renamed {
                Ok(Some(ingested)) => ingested,
                Ok(None) => continue,
                Err(error) => {
                    warn!(%error, from, to, "could not record a move");
                    continue;
                }
            };
            self.persist(family_id, &ingested.deltas).await?;
            self.persist_entry(checkout_id, from).await?;
            self.persist_entry(checkout_id, to).await?;
            self.propagate(checkout_id, &ingested.deltas).await?;
            handled.extend([from.clone(), to.clone()]);
        }
        Ok(handled)
    }

    fn files_under(&self, checkout: CheckoutId, relative: &str) -> Result<Vec<String>, CoreError> {
        let files = self.checkout(checkout)?.repo.worktree_files_under(relative)?;
        Ok(files
            .into_iter()
            .filter(|path| relative.is_empty() || path.starts_with(&format!("{relative}/")) || path == relative)
            .collect())
    }

    fn family_paths(&self, checkout: CheckoutId) -> Vec<String> {
        let Ok(checkout) = self.checkout(checkout) else {
            return Vec::new();
        };
        self.families
            .get(&checkout.family)
            .map(|family| {
                let view = family.thread_view(checkout.record.thread);
                view.paths()
                    .iter()
                    .filter(|(_, inode)| view.tree().kind(**inode) != Some(silt_crdt::InodeKind::Dir))
                    .map(|(path, _)| path.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Records one file's disk state. Returns paths that changed again
    /// meanwhile and need another pass.
    pub(crate) async fn ingest_file(&mut self, checkout_id: CheckoutId, path: &str) -> Result<Vec<String>, CoreError> {
        let root = self.checkout(checkout_id)?.record.path.clone();
        let entry = self.checkout(checkout_id)?.bridge.entries().get(path).cloned();
        // Same size and modification time as recorded: unchanged, without
        // reading and hashing it, which a rescan would do for every file.
        // File times tick coarsely, so two quick writes can share one; a
        // recent time isn't trusted, as git doesn't.
        let settled = |mtime_ns: i64| mtime_ns != 0 && mtime_ns < (now_ms() - 2_000).saturating_mul(1_000_000);
        if let (Some(entry), Ok(Some((size, mtime_ns, _)))) = (&entry, disk::stat(&root, path))
            && entry.size == size
            && entry.mtime_ns == mtime_ns
            && settled(mtime_ns)
        {
            return Ok(Vec::new());
        }
        let raw = disk::read(&root, path)?;
        let entry_hash = entry.map(|entry| entry.hash);
        let family_id = self.checkout(checkout_id)?.family;
        let thread = self.checkout(checkout_id)?.record.thread;
        let in_family =
            || self.families.get(&family_id).is_some_and(|family| family.thread_view(thread).paths().contains_key(path));
        match (&raw, entry_hash) {
            (Some(raw), Some(hash)) if raw.hash == hash => return Ok(Vec::new()),
            (None, None) if !in_family() => return Ok(Vec::new()),
            _ => {}
        }
        let disk_file = match raw {
            Some(raw) => Some(self.disk_file(checkout_id, path, raw).await?),
            None => None,
        };
        let author = self.author_for(checkout_id);
        let checkout = self.checkouts.get_mut(&checkout_id).expect("checked above");
        let family = self.families.get_mut(&family_id).expect("checkout's family is loaded");
        let ingested = checkout.bridge.ingest(family, &mut RepoBases(&checkout.repo), path, disk_file, &author, now_ms())?;
        self.persist(family_id, &ingested.deltas).await?;
        self.persist_entry(checkout_id, path).await?;
        let again = self.perform(checkout_id, ingested.actions).await?;
        self.propagate(checkout_id, &ingested.deltas).await?;
        Ok(again)
    }

    async fn disk_file(&self, checkout: CheckoutId, path: &str, raw: RawFile) -> Result<DiskFile, CoreError> {
        let content = if raw.is_text() {
            let clean = self.checkout(checkout)?.repo.filters()?.to_git(path, &raw.bytes)?;
            match String::from_utf8(clean) {
                Ok(text) => DiskContent::Text(text),
                Err(error) => DiskContent::Blob(self.store.blobs().put(error.as_bytes()).await?),
            }
        } else {
            DiskContent::Blob(self.store.blobs().put(&raw.bytes).await?)
        };
        let checkout = self.checkout(checkout)?;
        let executable = if checkout.repo.tracks_file_mode() {
            raw.executable
        } else {
            // The filesystem can't keep the bit, so what the thread says
            // stands.
            self.families.get(&checkout.family).is_some_and(|family| {
                let view = family.thread_view(checkout.record.thread);
                view.paths().get(path).is_some_and(|inode| view.is_executable(*inode))
            })
        };
        Ok(DiskFile { content, executable, hash: raw.hash, size: raw.size(), mtime_ns: raw.mtime_ns })
    }

    /// Brings other checkouts of the family up to date with `deltas`, taking
    /// in what changed in them meanwhile.
    async fn propagate(&mut self, from: CheckoutId, deltas: &[Delta]) -> Result<(), CoreError> {
        let family_id = self.checkout(from)?.family;
        let others: Vec<CheckoutId> = self
            .checkouts
            .iter()
            .filter(|(id, checkout)| **id != from && checkout.family == family_id)
            .map(|(id, _)| *id)
            .collect();
        for other in others {
            let checkout = self.checkout(other)?;
            let inodes: Vec<InodeId> =
                deltas.iter().filter(|delta| !checkout.bridge.is_own(delta)).filter_map(touched_inode).collect();
            let family = &self.families[&family_id];
            let actions = checkout.bridge.reconcile(family, &mut RepoBases(&checkout.repo), inodes);
            let again = self.perform(other, actions).await?;
            self.ingest_changed_meanwhile(other, again).await?;
        }
        Ok(())
    }

    /// Applies disk actions, skipping files that changed on disk since they
    /// were last recorded; those are returned to be ingested first.
    pub(crate) async fn perform(&mut self, checkout_id: CheckoutId, actions: Vec<DiskAction>) -> Result<Vec<String>, CoreError> {
        self.perform_with(checkout_id, actions, false).await
    }

    /// Where the filesystem folds case, paths of the thread that differ only
    /// in case would be one file, each write replacing the other. Only the
    /// first of them, in path order, is written.
    fn without_case_clashes(&self, checkout_id: CheckoutId, mut actions: Vec<DiskAction>) -> Result<Vec<DiskAction>, CoreError> {
        let checkout = self.checkout(checkout_id)?;
        if !checkout.repo.ignores_case() {
            return Ok(actions);
        }
        let Some(family) = self.families.get(&checkout.family) else {
            return Ok(actions);
        };
        let mut first: HashMap<String, String> = HashMap::new();
        for path in family.thread_view(checkout.record.thread).paths().keys() {
            first.entry(path.to_lowercase()).or_insert_with(|| path.clone());
        }
        actions.retain(|action| match action {
            DiskAction::Write { path, .. } => {
                let kept = first.get(&path.to_lowercase()).is_none_or(|winner| winner == path);
                if !kept {
                    warn!(
                        path,
                        "not writing a file whose name differs only in case from another's; this filesystem would merge them"
                    );
                }
                kept
            }
            DiskAction::Remove { .. } => true,
        });
        Ok(actions)
    }

    /// Like [`Core::perform`]; with `force`, files on disk are overwritten
    /// even when the checkout never recorded them (materializing a thread).
    pub(crate) async fn perform_with(
        &mut self,
        checkout_id: CheckoutId,
        actions: Vec<DiskAction>,
        force: bool,
    ) -> Result<Vec<String>, CoreError> {
        let actions = self.without_case_clashes(checkout_id, actions)?;
        let mut changed_meanwhile = Vec::new();
        for action in actions {
            match self.perform_action(checkout_id, action, force).await {
                Ok(Some(path)) => changed_meanwhile.push(path),
                Ok(None) => {}
                // One file that can't be written must not keep the others
                // from reaching the disk.
                Err(error) => warn!(%error, "could not update a file of the checkout"),
            }
        }
        Ok(changed_meanwhile)
    }

    /// Applies one disk action; returns the path when the file changed on
    /// disk since it was last recorded.
    async fn perform_action(
        &mut self,
        checkout_id: CheckoutId,
        action: DiskAction,
        force: bool,
    ) -> Result<Option<String>, CoreError> {
        let root = self.checkout(checkout_id)?.record.path.clone();
        match action {
            DiskAction::Write { path, inode, content, executable, version, moved_from } => {
                // The bytes come first: fetching them may wait, and the disk
                // must be checked after any wait, right before writing.
                let bytes = match content {
                    DiskContent::Text(text) => self.checkout(checkout_id)?.repo.filters()?.to_worktree(&path, text.as_bytes())?,
                    DiskContent::Blob(hash) => self.blob(hash).await?,
                    DiskContent::GitBlob(oid) => {
                        let repo = &self.checkout(checkout_id)?.repo;
                        repo.filters()?.to_worktree(&path, &repo.blob(oid)?)?
                    }
                };
                let recorded = self.checkout(checkout_id)?.bridge.entries().get(&path).map(|entry| entry.hash);
                let unchanged =
                    |current: Option<BlobHash>| current.is_none() || current == recorded || (force && recorded.is_none());
                let current = disk::read(&root, &path)?;
                if !unchanged(current.as_ref().map(|current| current.hash)) {
                    return Ok(Some(path));
                }
                let modes_count = self.checkout(checkout_id)?.repo.tracks_file_mode();
                let written = match current {
                    Some(current)
                        if current.hash == silt_store::BlobStore::hash(&bytes)
                            && (current.executable == executable || !modes_count) =>
                    {
                        current
                    }
                    _ => match disk::replace(&root, &path, &bytes, executable, unchanged)? {
                        Some(written) => written,
                        None => return Ok(Some(path)),
                    },
                };
                let entry = DiskEntry { inode, hash: written.hash, size: written.size(), mtime_ns: written.mtime_ns, version };
                self.checkouts.get_mut(&checkout_id).expect("checked above").bridge.record_written(&path, entry);
                self.persist_entry(checkout_id, &path).await?;
                if let Some(old) = moved_from {
                    return self.remove_from_disk(checkout_id, &root, old).await;
                }
            }
            DiskAction::Remove { path } => return self.remove_from_disk(checkout_id, &root, path).await,
        }
        Ok(None)
    }

    /// Removes a file the thread no longer has there, unless it changed on
    /// disk since it was recorded; then returns its path.
    async fn remove_from_disk(
        &mut self,
        checkout_id: CheckoutId,
        root: &Path,
        path: String,
    ) -> Result<Option<String>, CoreError> {
        let current = disk::read(root, &path)?;
        let recorded = self.checkout(checkout_id)?.bridge.entries().get(&path).map(|entry| entry.hash);
        if current.as_ref().is_some_and(|current| Some(current.hash) != recorded) {
            return Ok(Some(path));
        }
        disk::remove(root, &path)?;
        self.checkouts.get_mut(&checkout_id).expect("checked above").bridge.record_removed(&path);
        self.store.remove_disk_file(checkout_id, &path).await?;
        Ok(None)
    }

    /// Takes in files that changed on disk while the thread's version was
    /// being written there. Ignored files stay out: a peer's file of the
    /// same name must not make this machine record its own `.env`.
    pub(crate) async fn ingest_changed_meanwhile(
        &mut self,
        checkout_id: CheckoutId,
        paths: Vec<String>,
    ) -> Result<(), CoreError> {
        for path in self.trackable(checkout_id, paths)? {
            Box::pin(self.ingest_file(checkout_id, &path)).await?;
        }
        Ok(())
    }

    /// `paths` without Silt's and git's own data and ignored files.
    fn trackable(&self, checkout_id: CheckoutId, mut paths: Vec<String>) -> Result<Vec<String>, CoreError> {
        paths.retain(|path| !disk::is_internal(path));
        if paths.is_empty() {
            return Ok(paths);
        }
        let repo = &self.checkout(checkout_id)?.repo;
        match repo.ignored(&paths) {
            Ok(ignored) => paths.retain(|path| !ignored.contains(path)),
            // git refuses some paths, such as those inside a submodule;
            // asking one at a time leaves out only those.
            Err(_) => paths.retain(|path| match repo.ignored(std::slice::from_ref(path)) {
                Ok(ignored) => ignored.is_empty(),
                Err(error) => {
                    warn!(%error, path, "leaving out a path git won't check");
                    false
                }
            }),
        }
        Ok(paths)
    }

    async fn persist_entry(&self, checkout: CheckoutId, path: &str) -> Result<(), CoreError> {
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

    pub(crate) async fn append_conv(
        &mut self,
        family_id: FamilyId,
        thread: ThreadId,
        author: Author,
        op: ConvOp,
    ) -> Result<Delta, CoreError> {
        self.append_conv_with(family_id, thread, author, op, Vec::new(), None).await
    }

    /// Like `append_conv`, storing with the delta its searchable texts, as
    /// `(kind, body)`, and how far the session file it came from was read.
    pub(crate) async fn append_conv_with(
        &mut self,
        family_id: FamilyId,
        thread: ThreadId,
        author: Author,
        op: ConvOp,
        texts: Vec<(String, String)>,
        read: Option<SessionRead<'_>>,
    ) -> Result<Delta, CoreError> {
        let actor = self.thread_actor(thread);
        let family = self.families.get_mut(&family_id).expect("session's family is loaded");
        let delta = family.delta(actor, thread, author, now_ms(), family.thread_head(thread), Op::Conv(op));
        family.apply(delta.clone(), &mut HashMap::new())?;
        let rows: Vec<SearchRow> =
            texts.into_iter().map(|(kind, body)| SearchRow { thread, delta: delta.span.first(), kind, body }).collect();
        self.persist_with(family_id, std::slice::from_ref(&delta), &rows, read).await?;
        Ok(delta)
    }

    /// Stores deltas just applied to the family in memory. If the store
    /// fails, the family and its checkouts' records of the disk are reloaded
    /// from the store, so memory never holds what a restart would lose and
    /// nothing builds on it; a later pull or rescan records it again.
    pub(crate) async fn persist(&mut self, family_id: FamilyId, deltas: &[Delta]) -> Result<(), CoreError> {
        self.persist_with(family_id, deltas, &[], None).await
    }

    /// `persist`, with the deltas' search rows and session progress.
    pub(crate) async fn persist_with(
        &mut self,
        family_id: FamilyId,
        deltas: &[Delta],
        rows: &[SearchRow],
        read: Option<SessionRead<'_>>,
    ) -> Result<(), CoreError> {
        let Err(error) = self.store.append_deltas_with(family_id, deltas, rows, read).await else {
            self.snapshot_if_due(family_id).await;
            return Ok(());
        };
        if let Err(reload) = self.reload_family(family_id).await {
            warn!(error = %reload, family = %family_id, "could not reload the family after a failed write");
        }
        Err(error.into())
    }

    async fn reload_family(&mut self, family_id: FamilyId) -> Result<(), CoreError> {
        let repo = if self.families.contains_key(&family_id) { Some(self.family_repo(family_id)?) } else { None };
        let loaded = load::family(&self.store, family_id, repo.as_ref()).await?;
        self.take_loaded(family_id, loaded);
        let ids: Vec<CheckoutId> =
            self.checkouts.iter().filter(|(_, checkout)| checkout.family == family_id).map(|(id, _)| *id).collect();
        for id in ids {
            let entries = stored_entries(&self.store, id).await?;
            let checkout = self.checkouts.get_mut(&id).expect("listed above");
            checkout.bridge = Bridge::new(checkout.record.actor, checkout.record.thread, entries);
        }
        Ok(())
    }

    fn take_loaded(&mut self, family_id: FamilyId, loaded: Loaded) {
        match loaded {
            Loaded::Projected { family, covered } => {
                self.raw.remove(&family_id);
                self.families.insert(family_id, *family);
                self.snapshotted.insert(family_id, covered);
            }
            Loaded::Raw(deltas) => {
                self.families.remove(&family_id);
                self.raw.insert(family_id, deltas);
            }
        }
    }

    /// Saves the family's projection once enough was recorded since its last
    /// snapshot. A snapshot that can't be saved only makes the next load
    /// slower.
    async fn snapshot_if_due(&mut self, family_id: FamilyId) {
        let Some(family) = self.families.get(&family_id) else {
            return;
        };
        let covered = self.snapshotted.get(&family_id).copied().unwrap_or_default();
        if family.log().len() < covered + load::SNAPSHOT_EVERY {
            return;
        }
        match load::save_snapshot(&self.store, family_id, family).await {
            Ok(covers) => {
                self.snapshotted.insert(family_id, covers);
            }
            Err(error) => warn!(%error, family = %family_id, "could not save a snapshot of the family"),
        }
    }

    fn checkout(&self, id: CheckoutId) -> Result<&Checkout, CoreError> {
        self.checkouts.get(&id).ok_or_else(|| CoreError::Invalid(format!("unknown checkout {id}")))
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
    /// Stores a bundle of `commit` and its history that origin lacks and
    /// that `known` (commits bundled before) doesn't reach, so another
    /// machine can get it from the server (ADR 0009).
    pub(crate) async fn record_bundle(
        &mut self,
        family_id: FamilyId,
        thread: ThreadId,
        repo: &Repo,
        commit: GitOid,
        known: &[GitOid],
    ) -> Result<(), CoreError> {
        // Bundling a long history takes git a while; off the event loop.
        let (root, known_commits) = (repo.root().to_owned(), known.to_vec());
        let bundled = tokio::task::spawn_blocking(move || Repo::discover(&root)?.bundle(commit, &known_commits)).await?;
        let Some(bytes) = bundled? else {
            return Ok(());
        };
        self.push_to_mirror(family_id, thread, repo, commit);
        let blob = self.store.blobs().put(&bytes).await?;
        let actor = self.thread_actor(thread);
        let family = self.families.get_mut(&family_id).expect("thread's family is loaded");
        let op = Op::Git(GitOp::Bundle { commits: vec![commit], blob });
        let delta = family.delta(actor, thread, Author::Import, now_ms(), family.thread_head(thread), op);
        family.apply(delta.clone(), &mut HashMap::new())?;
        self.persist(family_id, &[delta]).await?;
        Ok(())
    }

    /// Records commits made in checkouts since the last look, with bundles.
    pub(crate) async fn check_commits(&mut self) -> Result<(), CoreError> {
        let ids: Vec<CheckoutId> = self.checkouts.keys().copied().collect();
        for id in ids {
            if let Err(error) = self.check_commit(id).await {
                warn!(%error, checkout = %id, "could not record a new commit");
            }
        }
        Ok(())
    }

    /// Records the checkout's HEAD when it differs from the last commit its
    /// thread has, which also catches commits made while the daemon was
    /// down. The head is remembered only once it is recorded, so a failure
    /// is retried next time.
    async fn check_commit(&mut self, id: CheckoutId) -> Result<(), CoreError> {
        let checkout = &self.checkouts[&id];
        let Some(head) = checkout.repo.head_commit()? else {
            return Ok(());
        };
        let (family_id, thread) = (checkout.family, checkout.record.thread);
        let known = match self.heads.get(&id) {
            Some(known) => Some(*known),
            None => self.recorded_commit(family_id, thread),
        };
        if known == Some(head) {
            self.heads.insert(id, head);
            return Ok(());
        }
        let branch = checkout.repo.head_branch()?;
        let repo = Repo::discover(&checkout.record.path)?;
        let actor = self.thread_actor(thread);
        let family = self.families.get_mut(&family_id).expect("checkout's family is loaded");
        let op = Op::Git(GitOp::Committed { commit: head, branch });
        let delta = family.delta(actor, thread, Author::User, now_ms(), family.thread_head(thread), op);
        family.apply(delta.clone(), &mut HashMap::new())?;
        self.persist(family_id, &[delta]).await?;
        repo.protect(&thread.to_string(), head)?;
        self.record_bundle(family_id, thread, &repo, head, known.as_slice()).await?;
        self.heads.insert(id, head);
        Ok(())
    }

    /// The last commit the thread has recorded, its own or inherited.
    fn recorded_commit(&self, family_id: FamilyId, thread: ThreadId) -> Option<GitOid> {
        let family = self.families.get(&family_id)?;
        let seen = family.graph().version_vector(&family.thread_head(thread));
        family.log().iter().rev().find_map(|delta| match &delta.op {
            Op::Git(GitOp::Genesis { commit, .. } | GitOp::Committed { commit, .. }) if seen.contains(delta.span.first()) => {
                Some(*commit)
            }
            _ => None,
        })
    }

    /// Threads whose lease moved to another device stop taking writes from
    /// this one. `before` holds the heads the moved threads had here before
    /// `deltas` arrived, which is where this device's work continues.
    pub(crate) async fn lease_changes(
        &mut self,
        deltas: &[Delta],
        before: &HashMap<ThreadId, Frontier>,
    ) -> Result<(), CoreError> {
        let Some(device) = self.remote.as_ref().map(|remote| remote.config.device) else {
            return Ok(());
        };
        let holder = |thread: ThreadId| {
            self.threads
                .get(&thread)
                .and_then(|record| self.families.get(&record.family))
                .and_then(|family| family.thread(thread))
                .and_then(ThreadState::lease)
                .map(|(holder, _)| holder)
        };
        for thread in moved_threads(deltas, device, holder) {
            self.thread_moved(thread, before.get(&thread).cloned()).await?;
        }
        Ok(())
    }

    /// Another device took `thread`'s lease: checkouts of it here continue in
    /// a fork from `at` (by default the thread's head), so nothing written
    /// here lands in the moved thread and nothing from there lands here.
    pub(crate) async fn thread_moved(&mut self, thread: ThreadId, at: Option<Frontier>) -> Result<(), CoreError> {
        let ids: Vec<CheckoutId> =
            self.checkouts.iter().filter(|(_, checkout)| checkout.record.thread == thread).map(|(id, _)| *id).collect();
        if ids.is_empty() {
            return Ok(());
        }
        let fork = self.fork_thread_at(thread, at, None).await?;
        for id in ids {
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
            let pushed = Repo::discover(&root).map_err(|error| error.to_string()).and_then(|repo| {
                repo.push_to_mirror(&url, &headers, commit, &thread.to_string()).map_err(|error| error.to_string())
            });
            if let Err(error) = pushed {
                warn!(%error, "could not push to the server's git mirror");
            }
        });
    }

    /// The web UI's URL, starting it first if needed; with `once`, one that
    /// works a single time, to hand to another program.
    async fn open_web(&mut self, once: bool) -> Result<String, CoreError> {
        if self.web.is_none() {
            self.web = Some(crate::web::start(self.events.clone()).await?);
        }
        let web = self.web.as_ref().expect("started above");
        Ok(if once { web.one_time_url() } else { web.url() })
    }

    fn connected(&self) -> Result<Remote, CoreError> {
        self.remote.clone().ok_or_else(|| CoreError::Invalid("not connected to a server; use `silt auth login`".into()))
    }

    fn inbox(
        &self,
        all: bool,
    ) -> Result<impl Future<Output = Result<Vec<crate::protocol::InboxEntry>, CoreError>> + use<>, CoreError> {
        let remote = self.connected()?;
        Ok(async move {
            let reply = remote.inbox(all).await.map_err(|source| CoreError::Server { doing: "read the inbox", source })?;
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
        })
    }

    fn inbox_read(
        &self,
        read: silt_proto::InboxRead,
    ) -> Result<impl Future<Output = Result<&'static str, CoreError>> + use<>, CoreError> {
        let remote = self.connected()?;
        Ok(async move {
            remote.inbox_read(&read).await.map_err(|source| CoreError::Server { doing: "mark the inbox read", source })?;
            Ok("read")
        })
    }

    fn presence(
        &self,
        selector: &str,
        file: Option<String>,
        line: Option<u32>,
        draft: String,
        leaving: bool,
    ) -> Result<impl Future<Output = Result<silt_proto::PresenceReply, CoreError>> + use<>, CoreError> {
        let thread = self.find_thread(selector)?;
        let remote = self.remote.clone();
        let update = silt_proto::PresenceUpdate {
            thread,
            file,
            line,
            draft: self.redactor.replace(&draft),
            leaving,
            agent: self.agent_status(thread).filter(|status| status.state != silt_proto::AgentState::Exited).map(|status| {
                silt_proto::AgentActivity {
                    name: status.agent,
                    state: status.state,
                    streaming: self.redactor.replace(&status.streaming),
                }
            }),
        };
        Ok(async move {
            let Some(remote) = remote else {
                return Ok(silt_proto::PresenceReply::default());
            };
            remote.presence(&update).await.map_err(|source| CoreError::Server { doing: "share presence", source })
        })
    }

    /// Teammates' current drafts for the thread's next prompt, for its
    /// agent (ADR 0014).
    pub(crate) async fn teammates_drafts(&self, thread: ThreadId) -> Option<String> {
        let drafts: Vec<String> =
            self.teammates_draft_list(thread).await.into_iter().map(|(user, draft)| format!("- {user}: {draft}")).collect();
        (!drafts.is_empty()).then(|| {
            format!(
                "Teammates watching this thread are drafting too; consider their drafts with this prompt:\n{}",
                drafts.join("\n")
            )
        })
    }

    /// Remembers secrets just redacted, so they stay redacted (ADR 0019).
    pub(crate) fn learn_secrets(&mut self, found: Vec<String>) {
        let new: Vec<String> = found.into_iter().filter(|secret| self.known_secrets.insert(secret.clone())).collect();
        if new.is_empty() {
            return;
        }
        let store = self.store.clone();
        tokio::spawn(async move {
            if let Err(error) = store.learn_secrets(&new).await {
                warn!(%error, "could not remember redacted secrets");
            }
        });
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

    /// Notes the clone at `cwd`, here and in the store, so agents can start
    /// new threads in it after a restart.
    pub(crate) async fn remember_clone(&mut self, cwd: &Path) {
        let Ok(repo) = Repo::discover(cwd) else {
            return;
        };
        let Ok(Some(url)) = repo.remote_url("origin") else {
            return;
        };
        let identity = silt_git::normalize_remote(&url);
        if self.clones.get(&identity).map(PathBuf::as_path) == Some(repo.root()) {
            return;
        }
        if let Err(error) = self.store.remember_clone(&identity, repo.root()).await {
            warn!(%error, "could not remember the clone");
        }
        self.clones.insert(identity, repo.root().to_owned());
    }

    /// Whether this machine's user may write into `thread`: its owner or a
    /// collaborator, or anyone when there is no server.
    pub(crate) fn may_write(&self, thread: ThreadId) -> bool {
        let user = self.own_user();
        let Some(record) = self.threads.get(&thread) else {
            return false;
        };
        user.is_empty()
            || record.owner.is_empty()
            || record.owner == user
            || self.collaborators.get(&thread).is_some_and(|names| names.contains(&user))
    }

    async fn share(&mut self, selector: &str, user: &str, remove: bool) -> Result<String, CoreError> {
        let thread = self.find_thread(selector)?;
        let remote =
            self.remote.as_ref().ok_or_else(|| CoreError::Invalid("sharing needs a server; use `silt auth login`".into()))?;
        remote.share(thread, user, remove).await.map_err(|source| CoreError::Server { doing: "share the thread", source })?;
        let names = self.collaborators.entry(thread).or_default();
        names.retain(|name| name != user);
        if !remove {
            names.push(user.to_owned());
        }
        let short = thread.short();
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
        let project: String = project.chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == '.' { ch } else { '-' }).collect();
        Mount { source: source.root().to_owned(), shared: self.data_dir.join("linked").join(project.trim_matches('-')) }
    }

    /// The actor for this daemon's own deltas in one thread. Threads of a
    /// family grow concurrently, and an actor's deltas must form one chain.
    pub(crate) fn thread_actor(&self, thread: ThreadId) -> ActorId {
        self.actor.derive(thread.0)
    }

    /// This device's user on the server; empty without one.
    pub(crate) fn own_user(&self) -> String {
        self.remote.as_ref().map(|remote| remote.config.user.clone()).unwrap_or_default()
    }

    /// Starts a new thread in `from`'s family at `from`'s current head.
    pub(crate) async fn fork_thread(&mut self, from: ThreadId) -> Result<ThreadId, CoreError> {
        self.fork_thread_at(from, None, None).await
    }

    /// Starts a new thread in `from`'s family at `at`, by default `from`'s
    /// current head. A fork with its own `visibility` is registered with the
    /// server before it has a delta to push, so it is never visible to
    /// anyone else in between.
    pub(crate) async fn fork_thread_at(
        &mut self,
        from: ThreadId,
        at: Option<Frontier>,
        visibility: Option<Visibility>,
    ) -> Result<ThreadId, CoreError> {
        let source = self.threads[&from].clone();
        let fork = ThreadId(uuid::Uuid::now_v7().as_u128());
        let record = ThreadRecord { id: fork, created_ms: now_ms(), owner: self.own_user(), ..source.clone() };
        match (&visibility, &self.remote) {
            (Some(_), Some(remote)) => remote
                .register_thread(&crate::sync::thread_info(&record), visibility)
                .await
                .map_err(|source| CoreError::Server { doing: "register the fork", source })?,
            _ => self.register_thread(&record),
        }
        self.store.create_thread(&record).await?;
        self.threads.insert(fork, record);
        let actor = self.thread_actor(fork);
        let family = self.families.get_mut(&source.family).expect("forked thread's family is loaded");
        let parents = at.unwrap_or_else(|| family.thread_head(from));
        let delta = family.delta(actor, fork, Author::User, now_ms(), parents, Op::Fork { from });
        family.apply(delta.clone(), &mut HashMap::new())?;
        self.persist(source.family, &[delta]).await?;
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
        Op::Conv(_) | Op::Git(_) | Op::Lease { .. } | Op::Fork { .. } | Op::Merge { .. } | Op::Mr(_) | Op::Comment(_) => None,
    }
}

/// What was found at a path that changed in a checkout.
enum Probe {
    /// The checkout's root, with the files under it.
    Root(Vec<String>),
    /// Another directory, with the files under it.
    Dir(Vec<String>),
    File(String),
    Gone(String),
}

/// The files to look at again after changes at `probes`. A rescan of the
/// root also looks at every file recorded on disk or in the thread, and a
/// path that is gone takes the recorded files under it along, since either
/// may have disappeared.
fn candidates<'a>(
    probes: Vec<Probe>,
    recorded: impl Iterator<Item = &'a String> + Clone,
    thread_paths: &[String],
) -> Vec<String> {
    let mut found = Vec::new();
    for probe in probes {
        match probe {
            Probe::Root(files) => {
                found.extend(files);
                found.extend(recorded.clone().cloned());
                found.extend_from_slice(thread_paths);
            }
            Probe::Dir(files) => found.extend(files),
            Probe::File(path) => found.push(path),
            Probe::Gone(path) => {
                let prefix = format!("{path}/");
                found.extend(recorded.clone().filter(|recorded| recorded.starts_with(&prefix)).cloned());
                found.push(path);
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// Threads whose lease `deltas` moved to a device other than `device`. The
/// lease with the highest epoch holds, so an older one arriving late moves
/// nothing.
fn moved_threads(deltas: &[Delta], device: DeviceId, holder: impl Fn(ThreadId) -> Option<DeviceId>) -> Vec<ThreadId> {
    let mut leased: Vec<ThreadId> =
        deltas.iter().filter(|delta| matches!(delta.op, Op::Lease { .. })).map(|delta| delta.thread).collect();
    leased.sort();
    leased.dedup();
    leased.retain(|thread| holder(*thread).is_some_and(|holder| holder != device));
    leased
}

/// What the store says a checkout's files on disk were last recorded as.
async fn stored_entries(store: &Store, checkout: CheckoutId) -> Result<BTreeMap<String, DiskEntry>, CoreError> {
    Ok(store
        .disk_files(checkout)
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
        .collect())
}

/// Redacts the secrets in this process's environment and in the data
/// directory's `secrets` file.
fn redactor(data_dir: &Path) -> Redactor {
    let file = data_dir.join("secrets");
    let secrets = silt_redact::read_secrets_file(&file).unwrap_or_else(|error| {
        warn!(%error, path = %file.display(), "could not read the secrets file; its secrets are not redacted");
        Vec::new()
    });
    Redactor::from_env().with(secrets)
}

/// Answers `reply` once `call`, a call to the server, is done, off the event
/// loop: a slow server then holds up only this request.
fn answer_later<T, F>(reply: oneshot::Sender<Response>, call: Result<F, CoreError>)
where
    T: serde::Serialize,
    F: Future<Output = Result<T, CoreError>> + Send + 'static,
{
    match call {
        Ok(call) => {
            tokio::spawn(async move {
                let _ = reply.send(to_response(call.await));
            });
        }
        Err(error) => {
            let _ = reply.send(Response::error(error));
        }
    }
}

pub(crate) fn to_response<T: serde::Serialize>(result: Result<T, CoreError>) -> Response {
    match result {
        Ok(value) => Response::ok(value),
        Err(error) => Response::failure(&error),
    }
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use silt_types::{DeltaSpan, Lamport};

    use super::*;

    #[test]
    fn a_rescan_looks_at_what_changed_and_what_may_be_gone() {
        let recorded = ["a/one".to_owned(), "a/two".to_owned(), "ab".to_owned(), "b".to_owned()];
        let found = candidates(
            vec![Probe::Gone("a".into()), Probe::File("c".into()), Probe::Dir(vec!["d/x".into()])],
            recorded.iter(),
            &["thread-only".to_owned()],
        );
        assert_eq!(found, ["a", "a/one", "a/two", "c", "d/x"]);
        let root = candidates(vec![Probe::Root(vec!["new".into(), "b".into()])], recorded.iter(), &["thread-only".to_owned()]);
        assert_eq!(root, ["a/one", "a/two", "ab", "b", "new", "thread-only"]);
    }

    #[test]
    fn only_threads_leased_elsewhere_move() {
        let (here, there) = (DeviceId(1), DeviceId(2));
        let lease = |thread: u128, device: DeviceId| Delta {
            span: DeltaSpan::new(ActorId(1), Lamport(thread as u64), 1),
            parents: Frontier::default(),
            thread: ThreadId(thread),
            author: Author::User,
            time_ms: 0,
            op: Op::Lease { device, epoch: 1 },
        };
        let deltas = [lease(1, there), lease(1, there), lease(2, here), lease(3, there)];
        // Thread 3's newer lease is held here, so the one arriving late moves nothing.
        let holder = |thread: ThreadId| Some(if thread.0 == 1 { there } else { here });
        assert_eq!(moved_threads(&deltas, here, holder), [ThreadId(1)]);
    }
}
