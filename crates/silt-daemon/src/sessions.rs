//! Agent sessions (ADR 0007, ADR 0012): finding each agent's session once
//! it is written, recording its transcripts as they grow, and turn marks
//! from hooks or from the transcript itself.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{Read as _, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use silt_agents::{Agent, Found, Mark, Reader, complete_lines_ending};
use silt_checkout::{ImportFile, import};
use silt_git::{FileMode, Repo};
use silt_log::{Author, ConvOp, Family, Op, SessionFile, SessionRef, TurnMark};
use silt_store::{SessionFileOffset, SessionRead, SessionRecord, ThreadRecord};
use silt_types::{ActorId, FamilyId, ThreadId};
use tokio::sync::oneshot;
use tracing::warn;

use crate::core::{Core, CoreError, Event, now_ms, to_response};
use crate::protocol::{HookReply, Response, SessionStarted};

/// How long after a tool finishes its file changes still count as its own;
/// covers the watcher's debounce.
const TOOL_SETTLE: Duration = Duration::from_millis(250);
/// How long edits wait for the transcript of an agent without Silt's
/// hooks: agents flush their transcripts late, and some only write a tool
/// call once it finished.
const TRANSCRIPT_LAG: Duration = Duration::from_millis(400);
/// How long a tool or turn read as finished from a transcript still owns
/// the edits that arrive; longer than the lag plus a tick of tailing.
const MARK_SETTLE: Duration = Duration::from_millis(1000);
/// How often a session kept in a store rather than a file is read again.
const SNAPSHOT_INTERVAL: Duration = Duration::from_secs(3);
const DISCOVER_INTERVAL: Duration = Duration::from_secs(1);
/// Most transcript bytes read per session and tick.
const TAIL_CHUNK: u64 = 1 << 20;
/// Agents stamp a new session's file a little after starting.
const DISCOVER_SLACK_MS: i64 = 5_000;

pub(crate) struct Session {
    pub(crate) record: SessionRecord,
    pub(crate) family: FamilyId,
    pub(crate) agent: &'static dyn Agent,
    pub(crate) agent_cwd: PathBuf,
    /// `None` until the agent has written it.
    pub(crate) transcript: Option<PathBuf>,
    pub(crate) offsets: HashMap<String, SessionFileOffset>,
    pub(crate) readers: HashMap<String, Box<dyn Reader>>,
    pub(crate) open_tool: Option<String>,
    pub(crate) in_turn: bool,
    /// The last tool and turn read as finished from the transcript, and
    /// when: edits the watcher reports just after still belong to them.
    pub(crate) closed_tool: Option<(String, Instant)>,
    pub(crate) turn_closed: Option<Instant>,
    /// Between the agent starting and exiting, as far as Silt knows.
    pub(crate) live: bool,
    /// Size and mtime of each side file when it was last attached.
    pub(crate) attached: HashMap<String, (u64, SystemTime)>,
    /// Keys of the records already stored, for agents read by snapshot.
    pub(crate) recorded: HashSet<String>,
    pub(crate) snapshot_running: bool,
    pub(crate) last_snapshot: Option<Instant>,
}

impl Session {
    pub(crate) fn reference(&self) -> SessionRef {
        SessionRef {
            agent: self.record.agent.clone(),
            id: self.record.session_id.clone(),
        }
    }
}

/// A session whose agent picks its id; it is found once written.
pub(crate) struct PendingSession {
    pub(crate) token: u64,
    pub(crate) agent: &'static dyn Agent,
    pub(crate) thread: ThreadId,
    pub(crate) family: FamilyId,
    pub(crate) checkout_id: i64,
    pub(crate) agent_cwd: PathBuf,
    pub(crate) checkout_root: PathBuf,
    pub(crate) since_ms: i64,
    pub(crate) searching: bool,
    pub(crate) last_search: Option<Instant>,
}

/// A thread just made for a clone.
pub(crate) struct CreatedThread {
    pub(crate) thread: ThreadId,
    pub(crate) family: FamilyId,
    pub(crate) checkout_id: i64,
    pub(crate) checkout: PathBuf,
    pub(crate) agent_cwd: PathBuf,
    pub(crate) repo: Repo,
    pub(crate) head: Option<silt_types::GitOid>,
}

/// Where a session to register came from.
pub(crate) struct NewSession {
    pub(crate) agent: &'static dyn Agent,
    pub(crate) id: String,
    pub(crate) thread: ThreadId,
    pub(crate) family: FamilyId,
    pub(crate) checkout_id: i64,
    pub(crate) agent_cwd: PathBuf,
    pub(crate) checkout_root: PathBuf,
    pub(crate) transcript: Option<PathBuf>,
    /// Lines already in the transcript that were recorded before, which the
    /// readers see but aren't stored again.
    pub(crate) restored: Vec<String>,
    pub(crate) restored_side: Vec<(String, Vec<String>)>,
}

pub(crate) fn file_key(file: &SessionFile) -> String {
    match file {
        SessionFile::Main => "main".to_owned(),
        SessionFile::Subagent(name) => format!("subagent:{name}"),
    }
}

fn source_of(key: &str) -> SessionFile {
    match key.strip_prefix("subagent:") {
        Some(name) => SessionFile::Subagent(name.to_owned()),
        None => SessionFile::Main,
    }
}

pub(crate) fn agent_author(session: &SessionRef) -> Author {
    Author::Agent {
        agent: session.agent.clone(),
        session: session.id.clone(),
        tool_use_id: None,
    }
}

pub(crate) fn resolve_agent(name: &str) -> Result<&'static dyn Agent, CoreError> {
    silt_agents::agent(name).ok_or_else(|| {
        let known: Vec<&str> = silt_agents::AGENTS.iter().map(|agent| agent.command()).collect();
        CoreError::Invalid(format!("unknown agent {name}; Silt knows {}", known.join(", ")))
    })
}

impl Core {
    pub(crate) async fn start_session(
        &mut self,
        cwd: &Path,
        agent: &str,
        session_id: Option<&str>,
        isolated: bool,
        already_ran: bool,
    ) -> Result<SessionStarted, CoreError> {
        let agent = resolve_agent(agent)?;
        let created = self.create_thread(cwd, isolated, already_ran).await?;
        match session_id {
            Some(id) => {
                let new = NewSession {
                    agent,
                    id: id.to_owned(),
                    thread: created.thread,
                    family: created.family,
                    checkout_id: created.checkout_id,
                    transcript: agent.transcript(&created.agent_cwd, id),
                    agent_cwd: created.agent_cwd.clone(),
                    checkout_root: created.checkout.clone(),
                    restored: Vec::new(),
                    restored_side: Vec::new(),
                };
                self.register_session(new).await?;
            }
            None => self.await_session(agent, created.thread, created.family, created.checkout_id, &created.agent_cwd, &created.checkout),
        }
        if let Some(commit) = created.head {
            self.record_bundle(created.family, created.thread, &created.repo, commit).await?;
        }
        Ok(SessionStarted {
            thread: created.thread.to_string(),
            session_id: session_id.map(str::to_owned),
            mount: isolated.then(|| self.mount(&created.repo)),
            checkout: created.checkout,
            agent_cwd: created.agent_cwd,
        })
    }

    /// A new thread for the clone at `cwd`: its family imported from the
    /// clone's HEAD and a checkout, managed when `isolated`, being watched.
    pub(crate) async fn create_thread(&mut self, cwd: &Path, isolated: bool, already_ran: bool) -> Result<CreatedThread, CoreError> {
        let cwd = cwd.canonicalize()?;
        let repo = Repo::discover(&cwd)?;
        self.remember_clone(&cwd).await;
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
                silt_store::CheckoutRecord {
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
        self.importing = !already_ran;
        let scanned = self.rescan(checkout_id).await;
        self.importing = false;
        scanned?;

        // `join("")` would add a trailing slash, which changes Claude's slug.
        let agent_cwd = if relative.as_os_str().is_empty() {
            checkout_path.clone()
        } else {
            checkout_path.join(&relative)
        };
        Ok(CreatedThread {
            thread,
            family: family_id,
            checkout_id,
            checkout: checkout_path,
            agent_cwd,
            repo,
            head,
        })
    }

    /// Waits for the agent to write a session it named itself.
    pub(crate) fn await_session(
        &mut self,
        agent: &'static dyn Agent,
        thread: ThreadId,
        family: FamilyId,
        checkout_id: i64,
        agent_cwd: &Path,
        checkout_root: &Path,
    ) {
        self.next_token += 1;
        self.pending.push(PendingSession {
            token: self.next_token,
            agent,
            thread,
            family,
            checkout_id,
            agent_cwd: agent_cwd.to_owned(),
            checkout_root: checkout_root.to_owned(),
            since_ms: now_ms() - DISCOVER_SLACK_MS,
            searching: false,
            last_search: None,
        });
    }

    /// Starts recording a session: stores it, notes where it started and
    /// begins tailing its transcript after anything restored into it.
    pub(crate) async fn register_session(&mut self, new: NewSession) -> Result<(), CoreError> {
        let record = SessionRecord {
            agent: new.agent.id().to_owned(),
            session_id: new.id.clone(),
            thread: new.thread,
            checkout_id: new.checkout_id,
        };
        self.store.add_session(&record, now_ms()).await?;
        let mut readers: HashMap<String, Box<dyn Reader>> = HashMap::new();
        let mut main_reader = new.agent.reader();
        for line in &new.restored {
            main_reader.push(line);
        }
        readers.insert("main".to_owned(), main_reader);
        for (name, lines) in &new.restored_side {
            let mut reader = new.agent.reader();
            for line in lines {
                reader.push(line);
            }
            readers.insert(format!("subagent:{name}"), reader);
        }
        let mut offsets = HashMap::new();
        if let Some(transcript) = &new.transcript {
            let mut files = vec![("main".to_owned(), transcript.clone())];
            if !new.restored.is_empty() {
                files.extend(new.agent.side_transcripts(transcript));
            }
            for (key, path) in files {
                let offset = SessionFileOffset {
                    file: key.clone(),
                    offset: if new.restored.is_empty() {
                        0
                    } else {
                        fs::metadata(&path).map_or(0, |metadata| metadata.len())
                    },
                    path,
                };
                self.store.set_session_offset(&record.agent, &new.id, &offset).await?;
                offsets.insert(key, offset);
            }
        }
        let recorded = new.restored.iter().filter_map(|line| new.agent.record_key(line)).collect();
        self.sessions.retain(|existing| existing.record.session_id != new.id);
        let reference = SessionRef {
            agent: record.agent.clone(),
            id: new.id.clone(),
        };
        self.sessions.push(Session {
            record,
            family: new.family,
            agent: new.agent,
            agent_cwd: new.agent_cwd.clone(),
            transcript: new.transcript.clone(),
            offsets,
            readers,
            open_tool: None,
            in_turn: false,
            closed_tool: None,
            turn_closed: None,
            live: true,
            attached: HashMap::new(),
            recorded,
            snapshot_running: false,
            last_snapshot: None,
        });
        self.append_session_started(new.family, new.thread, reference, &new.agent_cwd, &new.checkout_root)
            .await?;
        if new.restored.is_empty() {
            self.tail_session(&new.id).await?;
        }
        Ok(())
    }

    pub(crate) async fn append_session_started(
        &mut self,
        family: FamilyId,
        thread: ThreadId,
        session: SessionRef,
        cwd: &Path,
        checkout_root: &Path,
    ) -> Result<(), CoreError> {
        let author = agent_author(&session);
        let dir = self
            .session(&session.id)
            .and_then(|found| found.agent.session_dir(found.transcript.as_ref()?));
        let op = ConvOp::SessionStarted {
            session: session.clone(),
            cwd: cwd.to_string_lossy().into_owned(),
            checkout_root: checkout_root.to_string_lossy().into_owned(),
        };
        self.append_conv(family, thread, author.clone(), op).await?;
        if let Some(dir) = dir {
            let op = ConvOp::SessionDir {
                session,
                dir: dir.to_string_lossy().into_owned(),
            };
            self.append_conv(family, thread, author, op).await?;
        }
        Ok(())
    }

    /// Loads the sessions recorded before a restart, with readers that have
    /// seen what was already recorded.
    pub(crate) async fn load_sessions(&mut self) -> Result<(), CoreError> {
        for record in self.store.sessions().await? {
            let Some(family) = self.threads.get(&record.thread).map(|thread| thread.family) else {
                continue;
            };
            let Some(agent) = silt_agents::agent(&record.agent) else {
                continue;
            };
            let offsets: HashMap<String, SessionFileOffset> = self
                .store
                .session_offsets(&record.agent, &record.session_id)
                .await?
                .into_iter()
                .map(|offset| (offset.file.clone(), offset))
                .collect();
            let reference = SessionRef {
                agent: record.agent.clone(),
                id: record.session_id.clone(),
            };
            let mut readers: HashMap<String, Box<dyn Reader>> = HashMap::new();
            let mut recorded = HashSet::new();
            let mut agent_cwd = PathBuf::new();
            for delta in self.local_log(family) {
                match &delta.op {
                    Op::Conv(ConvOp::Native { session, file, line }) if *session == reference => {
                        readers.entry(file_key(file)).or_insert_with(|| agent.reader()).push(line);
                        if let Some(key) = agent.record_key(line) {
                            recorded.insert(key);
                        }
                    }
                    Op::Conv(ConvOp::SessionStarted { session, cwd, .. }) if *session == reference => {
                        agent_cwd = PathBuf::from(cwd);
                    }
                    _ => {}
                }
            }
            self.sessions.push(Session {
                transcript: offsets.get("main").map(|offset| offset.path.clone()),
                record,
                family,
                agent,
                agent_cwd,
                offsets,
                readers,
                open_tool: None,
                in_turn: false,
                closed_tool: None,
                turn_closed: None,
                live: false,
                attached: HashMap::new(),
                recorded,
                snapshot_running: false,
                last_snapshot: None,
            });
        }
        Ok(())
    }

    /// Finds sessions waiting to be discovered and reads sessions kept in a
    /// store, both off the event loop.
    pub(crate) fn poll_sessions(&mut self) {
        let known: Vec<String> = self.sessions.iter().map(|session| session.record.session_id.clone()).collect();
        for pending in &mut self.pending {
            if pending.searching || pending.last_search.is_some_and(|last| last.elapsed() < DISCOVER_INTERVAL) {
                continue;
            }
            pending.searching = true;
            pending.last_search = Some(Instant::now());
            let (agent, cwd, since, known, token) = (pending.agent, pending.agent_cwd.clone(), pending.since_ms, known.clone(), pending.token);
            let events = self.events.clone();
            tokio::task::spawn_blocking(move || {
                let found = agent.discover(&cwd, since, &known);
                let _ = events.send(Event::Discovered { token, found });
            });
        }
        for session in &mut self.sessions {
            let Some(transcript) = &session.transcript else {
                continue;
            };
            let due = session.last_snapshot.is_none_or(|last| last.elapsed() >= SNAPSHOT_INTERVAL);
            if !session.live || session.snapshot_running || !due {
                continue;
            }
            let found = Found {
                id: session.record.session_id.clone(),
                transcript: transcript.clone(),
            };
            let agent = session.agent;
            if !agent.reads_by_snapshot() {
                continue;
            }
            session.snapshot_running = true;
            let events = self.events.clone();
            tokio::task::spawn_blocking(move || {
                let result = agent.snapshot(&found).map_err(|error| error.to_string());
                let _ = events.send(Event::Snapshotted {
                    session_id: found.id,
                    result,
                });
            });
        }
    }

    /// Looks for the sessions still unknown on `checkout` right away, so
    /// their edits are credited to their tool calls. Agents found through
    /// their own CLI are left to the background search.
    async fn discover_now(&mut self, checkout: i64) {
        let known: Vec<String> = self.sessions.iter().map(|session| session.record.session_id.clone()).collect();
        let tokens: Vec<u64> = self
            .pending
            .iter()
            .filter(|pending| pending.checkout_id == checkout && !pending.searching && !pending.agent.reads_by_snapshot())
            .map(|pending| pending.token)
            .collect();
        for token in tokens {
            let pending = self.pending.iter().find(|pending| pending.token == token).expect("listed above");
            let found = pending.agent.discover(&pending.agent_cwd, pending.since_ms, &known);
            if found.is_some() {
                self.discovered(token, found).await;
            }
        }
    }

    pub(crate) async fn discovered(&mut self, token: u64, found: Option<Found>) {
        let Some(index) = self.pending.iter().position(|pending| pending.token == token) else {
            return;
        };
        let Some(found) = found else {
            self.pending[index].searching = false;
            return;
        };
        let pending = self.pending.remove(index);
        if let Err(error) = self.register_found(pending, found).await {
            warn!(%error, "could not record the agent's session");
        }
    }

    async fn register_found(&mut self, pending: PendingSession, found: Found) -> Result<(), CoreError> {
        let new = NewSession {
            agent: pending.agent,
            id: found.id,
            thread: pending.thread,
            family: pending.family,
            checkout_id: pending.checkout_id,
            agent_cwd: pending.agent_cwd,
            checkout_root: pending.checkout_root,
            transcript: Some(found.transcript),
            restored: Vec::new(),
            restored_side: Vec::new(),
        };
        self.register_session(new).await
    }

    pub(crate) async fn snapshotted(&mut self, session_id: &str, result: Result<Vec<String>, String>) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.snapshot_running = false;
        session.last_snapshot = Some(Instant::now());
        match result {
            Ok(lines) => {
                if let Err(error) = self.record_snapshot(session_id, lines).await {
                    warn!(%error, session = session_id, "could not record the session");
                }
            }
            Err(error) => warn!(%error, session = session_id, "could not read the session"),
        }
    }

    async fn record_snapshot(&mut self, session_id: &str, lines: Vec<String>) -> Result<(), CoreError> {
        let session = self.session(session_id).expect("caller checked");
        let fresh: Vec<String> = lines
            .into_iter()
            .filter(|line| session.agent.record_key(line).is_none_or(|key| !session.recorded.contains(&key)))
            .collect();
        for line in fresh {
            if let Some(key) = self.session(session_id).and_then(|session| session.agent.record_key(&line))
                && let Some(session) = self.session_mut(session_id)
            {
                session.recorded.insert(key);
            }
            self.record_line(session_id, "main", line, None).await?;
        }
        Ok(())
    }

    /// The agent in `thread` exited: find its session if it is still
    /// unknown, and record what it wrote last.
    pub(crate) async fn end_session(&mut self, thread: ThreadId) -> Result<&'static str, CoreError> {
        let known: Vec<String> = self.sessions.iter().map(|session| session.record.session_id.clone()).collect();
        let waiting: Vec<PendingSession> = {
            let (waiting, rest) = std::mem::take(&mut self.pending).into_iter().partition(|pending| pending.thread == thread);
            self.pending = rest;
            waiting
        };
        for pending in waiting {
            let (agent, cwd, since) = (pending.agent, pending.agent_cwd.clone(), pending.since_ms);
            let known = known.clone();
            let found = tokio::task::spawn_blocking(move || agent.discover(&cwd, since, &known))
                .await
                .map_err(|error| CoreError::Invalid(error.to_string()))?;
            match found {
                Some(found) => self.register_found(pending, found).await?,
                None => warn!("the agent's session was never written"),
            }
        }
        let ids: Vec<String> = self
            .sessions
            .iter()
            .filter(|session| session.record.thread == thread && session.live)
            .map(|session| session.record.session_id.clone())
            .collect();
        // Edits still waiting for the transcript are taken in now, while
        // the marks just read still own them.
        let checkouts: HashSet<i64> = self
            .sessions
            .iter()
            .filter(|session| session.record.thread == thread)
            .map(|session| session.record.checkout_id)
            .collect();
        for id in ids {
            let snapshot = self.session(&id).and_then(|session| {
                let found = Found {
                    id: id.clone(),
                    transcript: session.transcript.clone()?,
                };
                let agent = session.agent;
                agent.reads_by_snapshot().then_some((agent, found))
            });
            match snapshot {
                Some((agent, found)) => {
                    let lines = tokio::task::spawn_blocking(move || agent.snapshot(&found).map_err(|error| error.to_string()))
                        .await
                        .map_err(|error| CoreError::Invalid(error.to_string()))?;
                    match lines {
                        Ok(lines) => self.record_snapshot(&id, lines).await?,
                        Err(error) => warn!(%error, "could not read the session"),
                    }
                }
                None => self.tail_session_up_to(&id, u64::MAX).await?,
            }
            for checkout in &checkouts {
                if self.checkouts.contains_key(checkout) {
                    self.rescan(*checkout).await?;
                }
            }
            if let Some(session) = self.session_mut(&id) {
                session.live = false;
                session.in_turn = false;
                session.open_tool = None;
            }
        }
        Ok("ended")
    }

    /// Records new lines of a session's transcripts, a chunk at a time so a
    /// long backlog doesn't hold up the daemon; the next tick goes on.
    pub(crate) async fn tail_session(&mut self, session_id: &str) -> Result<(), CoreError> {
        self.tail_session_up_to(session_id, TAIL_CHUNK).await
    }

    async fn tail_session_up_to(&mut self, session_id: &str, limit: u64) -> Result<(), CoreError> {
        let Some(session) = self.session(session_id) else {
            return Ok(());
        };
        let agent = session.agent;
        let transcript = match &session.transcript {
            Some(transcript) => transcript.clone(),
            None => {
                let Some(found) = agent.transcript(&session.agent_cwd, session_id) else {
                    return Ok(());
                };
                let offset = SessionFileOffset {
                    file: "main".to_owned(),
                    path: found.clone(),
                    offset: 0,
                };
                self.store.set_session_offset(agent.id(), session_id, &offset).await?;
                let session = self.session_mut(session_id).expect("found above");
                session.offsets.insert("main".to_owned(), offset);
                session.transcript = Some(found.clone());
                let (family, thread, reference) = (session.family, session.record.thread, session.reference());
                if let Some(dir) = agent.session_dir(&found) {
                    let op = ConvOp::SessionDir {
                        session: reference.clone(),
                        dir: dir.to_string_lossy().into_owned(),
                    };
                    self.append_conv(family, thread, agent_author(&reference), op).await?;
                }
                found
            }
        };
        if agent.reads_by_snapshot() {
            return Ok(());
        }
        let mut files = vec![("main".to_owned(), transcript.clone())];
        files.extend(
            agent
                .side_transcripts(&transcript)
                .into_iter()
                .map(|(name, path)| (format!("subagent:{name}"), path)),
        );
        for (key, path) in files {
            self.tail_file(session_id, &key, &path, limit).await?;
        }
        self.attach_side_files(session_id, &transcript).await
    }

    /// Records the transcript's new complete lines, at most about `limit`
    /// bytes of them. The offset moves past each line as it is recorded, so
    /// a failure part way never records a line twice.
    async fn tail_file(&mut self, session_id: &str, key: &str, path: &Path, limit: u64) -> Result<(), CoreError> {
        let session = self.session(session_id).expect("caller checked");
        let mut offset = session.offsets.get(key).map_or(0, |offset| offset.offset);
        let Ok(mut file) = fs::File::open(path) else {
            return Ok(());
        };
        let length = file.metadata()?.len();
        if length < offset {
            // The agent rewound its transcript; what it dropped stays recorded.
            offset = length;
        }
        if length <= offset {
            return Ok(());
        }
        file.seek(SeekFrom::Start(offset))?;
        let mut buffer = Vec::new();
        (&mut file).take(limit).read_to_end(&mut buffer)?;
        if !buffer.contains(&b'\n') {
            // One line longer than the chunk.
            file.read_to_end(&mut buffer)?;
        }
        let (lines, consumed) = complete_lines_ending(&buffer);
        if consumed == 0 {
            return Ok(());
        }
        let read_to = |end: usize| SessionFileOffset {
            file: key.to_owned(),
            path: path.to_owned(),
            offset: offset + end as u64,
        };
        for (line, end) in lines {
            self.record_line(session_id, key, line, Some(read_to(end))).await?;
        }
        // Lines that recorded nothing still count as read.
        let read = read_to(consumed);
        if self.session(session_id).and_then(|session| session.offsets.get(key)) != Some(&read) {
            let agent = self.session(session_id).expect("caller checked").record.agent.clone();
            self.store.set_session_offset(&agent, session_id, &read).await?;
            if let Some(session) = self.session_mut(session_id) {
                session.offsets.insert(key.to_owned(), read);
            }
        }
        Ok(())
    }

    /// Stores one transcript record, indexes its text, and applies the turn
    /// marks it carries for agents without Silt's hooks. `read` is how far
    /// its file has been read with this line; it is saved with the record,
    /// so nothing that fails afterwards records the line again.
    pub(crate) async fn record_line(&mut self, session_id: &str, key: &str, line: String, read: Option<SessionFileOffset>) -> Result<(), CoreError> {
        let session = self.session_mut(session_id).expect("caller checked");
        let agent = session.agent;
        let parsed = session.readers.entry(key.to_owned()).or_insert_with(|| agent.reader()).push(&line);
        let (family, thread, reference, agent_name) = (session.family, session.record.thread, session.reference(), session.record.agent.clone());
        let op = ConvOp::Native {
            session: reference.clone(),
            file: source_of(key),
            line,
        };
        let texts: Vec<(String, String)> = parsed
            .entries
            .iter()
            .filter_map(|entry| {
                let text = entry.searchable_text().filter(|text| !text.trim().is_empty())?;
                Some((entry.kind().to_owned(), text))
            })
            .collect();
        let read_to = read.as_ref().map(|offset| SessionRead {
            agent: &agent_name,
            session_id,
            offset,
        });
        self.append_conv_with(family, thread, agent_author(&reference), op, texts, read_to).await?;
        if let (Some(read), Some(session)) = (read, self.session_mut(session_id)) {
            session.offsets.insert(key.to_owned(), read);
        }
        if !agent.hooks() && key == "main" {
            for mark in parsed.marks {
                self.apply_mark(session_id, mark).await?;
            }
        }
        Ok(())
    }

    async fn apply_mark(&mut self, session_id: &str, mark: Mark) -> Result<(), CoreError> {
        let session = self.session_mut(session_id).expect("caller checked");
        let mark = match mark {
            Mark::PromptSubmitted => {
                session.in_turn = true;
                TurnMark::PromptSubmitted
            }
            Mark::ToolStarted { tool_use_id, tool } => {
                session.open_tool = Some(tool_use_id.clone());
                TurnMark::ToolStarted { tool_use_id, tool }
            }
            Mark::ToolFinished { tool_use_id } => {
                if session.open_tool.as_deref() == Some(&tool_use_id) {
                    session.open_tool = None;
                }
                session.closed_tool = Some((tool_use_id.clone(), Instant::now()));
                TurnMark::ToolFinished { tool_use_id }
            }
            Mark::Stopped => {
                session.in_turn = false;
                if let Some(tool) = session.open_tool.take() {
                    session.closed_tool = Some((tool, Instant::now()));
                }
                session.turn_closed = Some(Instant::now());
                TurnMark::Stopped
            }
        };
        self.record_turn(session_id, mark).await
    }

    /// Records the session's side files as attachments when they are new or
    /// changed, so a restored session is complete.
    async fn attach_side_files(&mut self, session_id: &str, transcript: &Path) -> Result<(), CoreError> {
        let session = self.session(session_id).expect("caller checked");
        let mut changed = Vec::new();
        for (name, path) in session.agent.attachments(transcript) {
            let Ok(metadata) = fs::metadata(&path) else {
                continue;
            };
            let stamp = (metadata.len(), metadata.modified()?);
            if session.attached.get(&name) != Some(&stamp) {
                changed.push((name, path, stamp));
            }
        }
        if changed.is_empty() {
            return Ok(());
        }
        let reference = session.reference();
        let (family, thread) = (session.family, session.record.thread);
        for (name, path, stamp) in changed {
            let bytes = fs::read(&path)?;
            let bytes = match String::from_utf8(bytes) {
                Ok(text) => {
                    let (redacted, found) = self.redactor.replace_found(&text);
                    self.learn_secrets(found);
                    redacted.into_bytes()
                }
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

    /// Reads the transcripts of agents that mark turns in them before the
    /// edits of `checkout` are attributed.
    pub(crate) async fn tail_marked_sessions(&mut self, checkout: i64) {
        self.discover_now(checkout).await;
        let ids: Vec<String> = self
            .sessions
            .iter()
            .filter(|session| session.record.checkout_id == checkout && session.live && !session.agent.hooks())
            .map(|session| session.record.session_id.clone())
            .collect();
        for id in ids {
            if let Err(error) = self.tail_session(&id).await {
                warn!(%error, session = id, "could not read the session transcript");
            }
        }
    }

    /// Whether a running agent on `checkout` marks its turns only in its
    /// transcript.
    pub(crate) fn marks_from_transcripts(&self, checkout: i64) -> bool {
        self.pending.iter().any(|pending| pending.checkout_id == checkout)
            || self
                .sessions
                .iter()
                .any(|session| session.record.checkout_id == checkout && session.live && !session.agent.hooks())
    }

    /// Takes the changes in again once the transcripts have caught up.
    pub(crate) fn settle_changes(&mut self, checkout: i64, paths: Vec<PathBuf>, noticed: Instant) {
        let events = self.events.clone();
        tokio::spawn(async move {
            tokio::time::sleep(TRANSCRIPT_LAG).await;
            let _ = events.send(Event::Changed {
                checkout,
                paths,
                noticed,
                settled: true,
            });
        });
    }

    pub(crate) fn author_for(&self, checkout: i64) -> Author {
        if self.importing {
            return Author::Import;
        }
        let sessions = || self.sessions.iter().filter(|session| session.record.checkout_id == checkout);
        // A mark read close to when the edit was noticed, before or after,
        // was written around the edit: transcripts trail the edits.
        let noticed = self.change_noticed.unwrap_or_else(Instant::now);
        let settling = |at: &Instant| {
            let apart = if *at > noticed { *at - noticed } else { noticed - *at };
            apart < MARK_SETTLE
        };
        if let Some(author) = sessions().find_map(|session| {
            let closed = session.closed_tool.as_ref().filter(|(_, at)| settling(at)).map(|(tool, _)| tool);
            session.open_tool.as_ref().or(closed).map(|tool| Author::Agent {
                agent: session.record.agent.clone(),
                session: session.record.session_id.clone(),
                tool_use_id: Some(tool.clone()),
            })
        }) {
            return author;
        }
        // Without hooks, edits are only known to fall inside a turn; a store
        // read by snapshot learns even that late, so its whole run counts.
        sessions()
            .find(|session| {
                !session.agent.hooks()
                    && (session.in_turn
                        || session.turn_closed.as_ref().is_some_and(settling)
                        || (session.live && session.agent.reads_by_snapshot()))
            })
            .map(|session| agent_author(&session.reference()))
            .or_else(|| {
                // The agent hasn't written its session yet, but it is running.
                self.pending.iter().find(|pending| pending.checkout_id == checkout).map(|pending| Author::Agent {
                    agent: pending.agent.id().to_owned(),
                    session: String::new(),
                    tool_use_id: None,
                })
            })
            .unwrap_or(Author::User)
    }

    /// Returns a response to send now, or `None` when the reply is deferred.
    pub(crate) async fn hook(
        &mut self,
        event: &str,
        input: serde_json::Value,
        reply: oneshot::Sender<Response>,
    ) -> Option<(Response, oneshot::Sender<Response>)> {
        let input: silt_agents::claude::HookInput = match serde_json::from_value(input) {
            Ok(input) => input,
            Err(error) => return Some((Response::error(error), reply)),
        };
        let session_id = input.session_id.clone();
        let Some((thread, hooked)) = self
            .session(&session_id)
            .map(|session| (session.record.thread, session.agent.hooks()))
        else {
            return Some((Response::ok(HookReply::default()), reply));
        };
        if !hooked {
            let tailed = self.tail_session(&session_id).await;
            let context = if event == "UserPromptSubmit" { self.prompt_context(thread).await } else { None };
            let reply_data = HookReply {
                additional_context: context,
            };
            return Some((to_response(tailed.map(|()| reply_data)), reply));
        }
        let result = match event {
            "UserPromptSubmit" => {
                let tailed = self.tail_session(&session_id).await;
                let recorded = match tailed {
                    Ok(()) => self.record_turn(&session_id, TurnMark::PromptSubmitted).await,
                    Err(error) => Err(error),
                };
                let reply_data = HookReply {
                    additional_context: self.prompt_context(thread).await,
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

    /// What the agent hears with a prompt: Silt's notices and teammates'
    /// live drafts.
    async fn prompt_context(&mut self, thread: ThreadId) -> Option<String> {
        let parts: Vec<String> = [self.take_notices(thread), self.teammates_drafts(thread).await].into_iter().flatten().collect();
        (!parts.is_empty()).then(|| parts.join("\n\n"))
    }

    pub(crate) async fn close_tool(&mut self, session_id: &str, reply: oneshot::Sender<Response>) {
        let tool = self.session_mut(session_id).and_then(|session| session.open_tool.take());
        if let Some(tool_use_id) = tool {
            let _ = self
                .record_turn(session_id, TurnMark::ToolFinished { tool_use_id })
                .await;
        }
        let _ = reply.send(Response::ok("ok"));
    }

    pub(crate) async fn record_turn(&mut self, session_id: &str, mark: TurnMark) -> Result<(), CoreError> {
        let Some(session) = self.session(session_id) else {
            return Ok(());
        };
        let reference = session.reference();
        let (family, thread) = (session.family, session.record.thread);
        let author = agent_author(&reference);
        self.append_conv(family, thread, author, ConvOp::Turn { session: reference, mark }).await?;
        Ok(())
    }

    pub(crate) fn session(&self, session_id: &str) -> Option<&Session> {
        self.sessions.iter().find(|session| session.record.session_id == session_id)
    }

    pub(crate) fn session_mut(&mut self, session_id: &str) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|session| session.record.session_id == session_id)
    }
}
