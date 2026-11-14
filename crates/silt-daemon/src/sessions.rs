//! Agent sessions (ADR 0007, ADR 0012): finding each agent's session once
//! it is written, recording its transcripts as they grow, and turn marks
//! from hooks or from the transcript itself.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{Read as _, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use silt_agents::{Agent, Capture, Found, Mark, Reader, complete_lines_ending};
use silt_checkout::{ImportFile, import};
use silt_git::{FileMode, Repo};
use silt_log::{Author, ConvOp, Delta, Family, GitOp, Op, SessionFile, SessionRef, TurnMark};
use silt_store::{CheckoutId, SearchRow, SessionFileOffset, SessionRead, SessionRecord, ThreadRecord};
use silt_types::{ActorId, CloudSessionId, FamilyId, ThreadId};
use tracing::warn;

use crate::attribution;
use crate::core::{CanonicalCwd, Core, CoreError, Event, now_ms, to_response};
use crate::protocol::{Response, SessionStarted};

/// How long edits wait for the transcript of an agent without Silt's
/// hooks: agents flush their transcripts late, and some only write a tool
/// call once it finished.
const TRANSCRIPT_LAG: Duration = Duration::from_millis(400);
/// How often a session kept in a store rather than a file is read again.
const SNAPSHOT_INTERVAL: Duration = Duration::from_secs(3);
const DISCOVER_INTERVAL: Duration = Duration::from_secs(1);
/// How long a started agent may take to write its session before Silt stops
/// looking. Its CLI normally says when it exits; this covers one that was
/// killed, which would otherwise be searched for, and credited with the
/// person's edits, for good.
const PENDING_TTL: Duration = Duration::from_secs(30 * 60);
/// Most transcript bytes read per session and tick.
const TAIL_CHUNK: u64 = 1 << 20;
/// Most transcript bytes of a backlog read at once: big enough that a long
/// session imports in a few writes, small enough that requests waiting
/// behind it are answered soon.
const BACKLOG_CHUNK: u64 = 8 << 20;
/// Agents stamp a new session's file a little after starting.
const DISCOVER_SLACK_MS: i64 = 5_000;

/// A transcript record applied to its family in memory, not stored yet.
pub(crate) struct StagedLine {
    pub(crate) family: FamilyId,
    pub(crate) delta: Delta,
    pub(crate) rows: Vec<SearchRow>,
    pub(crate) marks: Vec<Mark>,
}

/// Records staged from one session file, to store in one write.
#[derive(Default)]
struct Batch {
    family: Option<FamilyId>,
    deltas: Vec<Delta>,
    rows: Vec<SearchRow>,
    /// Where in the chunk read the last record staged ends.
    read: usize,
}

impl Batch {
    fn push(&mut self, family: FamilyId, delta: Delta, rows: Vec<SearchRow>, end: usize) {
        self.family = Some(family);
        self.deltas.push(delta);
        self.rows.extend(rows);
        self.read = end;
    }
}

pub(crate) struct Session {
    pub(crate) record: SessionRecord,
    pub(crate) family: FamilyId,
    pub(crate) agent: &'static dyn Agent,
    pub(crate) agent_cwd: PathBuf,
    /// `None` until the agent has written it.
    pub(crate) transcript: Option<PathBuf>,
    pub(crate) offsets: HashMap<String, SessionFileOffset>,
    pub(crate) readers: HashMap<String, Box<dyn Reader>>,
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
        SessionRef { agent: self.record.agent.clone(), id: self.record.session_id.clone() }
    }
}

/// A session whose agent picks its id; it is found once written.
pub(crate) struct PendingSession {
    pub(crate) token: u64,
    pub(crate) agent: &'static dyn Agent,
    pub(crate) thread: ThreadId,
    pub(crate) family: FamilyId,
    pub(crate) checkout_id: CheckoutId,
    pub(crate) agent_cwd: PathBuf,
    pub(crate) checkout_root: PathBuf,
    pub(crate) since_ms: i64,
    pub(crate) searching: bool,
    pub(crate) last_search: Option<Instant>,
    pub(crate) started: Instant,
}

/// What becomes of a session still not found when it is looked for.
#[derive(Debug, Clone, Copy)]
enum Waiting {
    Keep,
    /// The agent exited, so it never wrote one.
    GiveUp,
}

/// A thread just made for a clone.
pub(crate) struct CreatedThread {
    pub(crate) thread: ThreadId,
    pub(crate) family: FamilyId,
    pub(crate) checkout_id: CheckoutId,
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
    pub(crate) checkout_id: CheckoutId,
    pub(crate) agent_cwd: PathBuf,
    pub(crate) checkout_root: PathBuf,
    pub(crate) transcript: Option<PathBuf>,
    /// Lines already in the transcript that were recorded before, which the
    /// readers see but aren't stored again.
    pub(crate) restored: Vec<String>,
    pub(crate) restored_side: Vec<(String, Vec<String>)>,
}

/// Whether the session's main transcript has more than was read: a session
/// Silt doesn't know to be running, such as one recorded before a restart,
/// is read again only then, at the cost of one `stat`.
pub(crate) fn transcript_grew(session: &Session) -> bool {
    let (Some(transcript), Some(read)) = (&session.transcript, session.offsets.get("main")) else {
        return false;
    };
    fs::metadata(transcript).is_ok_and(|metadata| metadata.len() > read.offset)
}

/// Feeds `sessions` what `deltas` recorded of them, as their transcripts'
/// readers would have read it. Readers that read each line on its own
/// aren't shown them, which matters for a hook reading a long session.
/// Whether the session's agent needs the lines recorded so far to read the
/// next ones, or to know what it recorded.
fn reads_recorded(session: &Session) -> bool {
    session.agent.capture() == Capture::Snapshot || session.agent.reader().remembers()
}

pub(crate) fn read_recorded(sessions: &mut [Session], deltas: &[silt_log::Delta]) {
    let by_reference: HashMap<SessionRef, usize> =
        sessions.iter().enumerate().map(|(index, session)| (session.reference(), index)).collect();
    // Most agents' readers keep nothing between lines, and a long session's
    // lines would each be looked at for nothing.
    let wanted: Vec<bool> = sessions.iter().map(reads_recorded).collect();
    for delta in deltas {
        match &delta.op {
            Op::Conv(ConvOp::Native { session, file, line }) => {
                let Some(index) = by_reference.get(session).copied().filter(|index| wanted[*index]) else {
                    continue;
                };
                let found = &mut sessions[index];
                let agent = found.agent;
                let reader = found.readers.entry(file_key(file)).or_insert_with(|| agent.reader());
                if reader.remembers() {
                    reader.push(line);
                }
                if agent.capture() == Capture::Snapshot
                    && let Some(key) = agent.record_key(line)
                {
                    found.recorded.insert(key);
                }
            }
            Op::Conv(ConvOp::SessionStarted { session, cwd, .. }) => {
                if let Some(index) = by_reference.get(session) {
                    sessions[*index].agent_cwd = PathBuf::from(cwd);
                }
            }
            _ => {}
        }
    }
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
    Author::Agent { agent: session.agent.clone(), session: session.id.clone(), tool_use_id: None }
}

/// The branch a thread started from `base` follows: `base` itself, or the
/// one an `origin/` branch tracks; none for a commit or a tag.
fn branch_named(repo: &Repo, base: &str) -> Result<Option<String>, CoreError> {
    let branches = repo.branches()?;
    Ok(branches.iter().any(|branch| branch == base).then(|| base.strip_prefix("origin/").unwrap_or(base).to_owned()))
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
        // A `SessionStart` hook runs again when a session resumes or clears.
        if let Some(session) = session_id.and_then(|id| self.session(id))
            && already_ran
        {
            return Ok(SessionStarted {
                thread: session.record.thread.to_string(),
                session_id: Some(session.record.session_id.clone()),
                mount: None,
                checkout: self.checkout(session.record.checkout_id)?.record.path.clone(),
                agent_cwd: session.agent_cwd.clone(),
            });
        }
        let created = self.create_thread(cwd, isolated, already_ran, None).await?;
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
            None => self.await_session(
                agent,
                created.thread,
                created.family,
                created.checkout_id,
                &created.agent_cwd,
                &created.checkout,
            ),
        }
        if let Some(commit) = created.head {
            self.record_bundle(created.family, created.thread, &created.repo, commit, &[]).await?;
        }
        Ok(SessionStarted {
            thread: created.thread.to_string(),
            session_id: session_id.map(str::to_owned),
            mount: isolated.then(|| self.mount(&created.repo)),
            checkout: created.checkout,
            agent_cwd: created.agent_cwd,
        })
    }

    /// Records that the session runs in the cloud, once per thread.
    pub(crate) async fn mark_cloud(
        &mut self,
        started: SessionStarted,
        agent: &str,
        id: CloudSessionId,
    ) -> Result<SessionStarted, CoreError> {
        let (Some(session_id), Ok(thread)) = (&started.session_id, started.thread.parse::<ThreadId>()) else {
            return Ok(started);
        };
        let Some(family) = self.threads.get(&thread).map(|record| record.family) else {
            return Ok(started);
        };
        if self
            .families
            .get(&family)
            .and_then(|family| family.thread(thread))
            .is_some_and(|state| state.cloud_session().is_some())
        {
            return Ok(started);
        }
        let session = SessionRef { agent: resolve_agent(agent)?.id().to_owned(), id: session_id.clone() };
        self.append_conv(family, thread, agent_author(&session), ConvOp::CloudSession { session, id }).await?;
        Ok(started)
    }

    /// A new thread for the clone at `cwd`: its family imported from the
    /// clone's HEAD and a checkout, managed when `isolated`, being watched.
    pub(crate) async fn create_thread(
        &mut self,
        cwd: &Path,
        isolated: bool,
        already_ran: bool,
        base: Option<&str>,
    ) -> Result<CreatedThread, CoreError> {
        let cwd = CanonicalCwd::new(cwd)?;
        let repo = Repo::discover(&cwd)?;
        // A chat's folder is no clone of a project (ADR 0059).
        if !repo.is_chat() {
            self.remember_clone(&cwd).await;
        }
        let relative = cwd.strip_prefix(repo.root()).unwrap_or(Path::new("")).to_owned();
        let (head, branch) = match base {
            Some(base) => {
                if !isolated {
                    return Err(CoreError::Invalid(format!(
                        "work in your own clone starts from what's checked out there; give the thread its own checkout to start from {base}"
                    )));
                }
                let commit = repo.commit_of(base)?.ok_or_else(|| CoreError::Invalid(format!("{base} names no commit here")))?;
                (Some(commit), branch_named(&repo, base)?)
            }
            None => (repo.head_commit()?, repo.head_branch()?),
        };
        if isolated && head.is_none() {
            return Err(CoreError::Invalid("an isolated checkout needs at least one commit".into()));
        }
        let files: Vec<ImportFile> = match head {
            Some(commit) => repo
                .files_at(commit)?
                .into_iter()
                .filter(|file| file.mode != FileMode::Symlink)
                .map(|file| ImportFile { path: file.path, oid: file.oid, executable: file.mode == FileMode::Executable })
                .collect(),
            None => Vec::new(),
        };
        let thread = ThreadId(uuid::Uuid::now_v7().as_u128());
        let family_id = FamilyId(uuid::Uuid::now_v7().as_u128());
        let project = repo.project()?;
        if let Some(home) = self.new_home_in(&repo, &project)? {
            self.set_home(family_id, home).await?;
        }
        let workspace = self.new_family_workspace(&repo, &project);
        self.pin_workspace(family_id, &workspace).await?;
        let record = ThreadRecord {
            id: thread,
            family: family_id,
            repo_root: repo.root().to_owned(),
            repo: project,
            created_ms: now_ms(),
            owner: self.own_user_in(family_id),
        };
        self.store.create_thread(&record).await?;
        self.register_thread(&record);
        self.threads.insert(thread, record);

        let mut family = Family::new();
        let deltas =
            import(&mut family, self.thread_actor(thread), thread, head.map(|commit| (commit, branch)), &files, now_ms())?;
        self.store.append_deltas(family_id, &deltas).await?;
        self.families.insert(family_id, family);
        self.announce_family(family_id, &deltas);

        let checkout_path = match head {
            Some(commit) if isolated => repo.create_managed_checkout(&thread.to_string(), commit, None)?.path,
            _ => repo.root().to_owned(),
        };
        let checkout_actor = ActorId(uuid::Uuid::new_v4().as_u128());
        // From the store, as a process other than the daemon doesn't hold
        // every checkout.
        let existing = self.store.checkouts().await?.into_iter().find(|checkout| checkout.path == checkout_path);
        let checkout = match existing {
            Some(previous) => {
                self.checkouts.remove(&previous.id);
                self.store.rebind_checkout(previous.id, thread, checkout_actor).await?;
                silt_store::CheckoutRecord { thread, actor: checkout_actor, ..previous }
            }
            None => self.store.add_checkout(thread, &checkout_path, checkout_actor, isolated, now_ms()).await?,
        };
        let checkout_id = checkout.id;
        self.start_checkout(checkout, BTreeMap::new())?;
        self.importing = !already_ran;
        let scanned = self.rescan(checkout_id).await;
        self.importing = false;
        scanned?;

        // `join("")` would add a trailing slash, which changes Claude's slug.
        let agent_cwd = if relative.as_os_str().is_empty() { checkout_path.clone() } else { checkout_path.join(&relative) };
        Ok(CreatedThread { thread, family: family_id, checkout_id, checkout: checkout_path, agent_cwd, repo, head })
    }

    /// Waits for the agent to write a session it named itself.
    pub(crate) fn await_session(
        &mut self,
        agent: &'static dyn Agent,
        thread: ThreadId,
        family: FamilyId,
        checkout_id: CheckoutId,
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
            started: Instant::now(),
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
                    offset: if new.restored.is_empty() { 0 } else { fs::metadata(&path).map_or(0, |metadata| metadata.len()) },
                    path,
                };
                self.store.set_session_offset(&record.agent, &new.id, &offset).await?;
                offsets.insert(key, offset);
            }
        }
        let recorded = new.restored.iter().filter_map(|line| new.agent.record_key(line)).collect();
        self.sessions.retain(|existing| existing.record.session_id != new.id);
        let reference = SessionRef { agent: record.agent.clone(), id: new.id.clone() };
        self.sessions.push(Session {
            record,
            family: new.family,
            agent: new.agent,
            agent_cwd: new.agent_cwd.clone(),
            transcript: new.transcript.clone(),
            offsets,
            readers,
            in_turn: false,
            closed_tool: None,
            turn_closed: None,
            live: true,
            attached: HashMap::new(),
            recorded,
            snapshot_running: false,
            last_snapshot: None,
        });
        self.append_session_started(new.family, new.thread, reference, &new.agent_cwd, &new.checkout_root).await?;
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
        let dir = self.session(&session.id).and_then(|found| found.agent.session_dir(found.transcript.as_ref()?));
        let op = ConvOp::SessionStarted {
            session: session.clone(),
            cwd: cwd.to_string_lossy().into_owned(),
            checkout_root: checkout_root.to_string_lossy().into_owned(),
        };
        self.append_conv(family, thread, author.clone(), op).await?;
        if let Some(dir) = dir {
            let op = ConvOp::SessionDir { session, dir: dir.to_string_lossy().into_owned() };
            self.append_conv(family, thread, author, op).await?;
        }
        Ok(())
    }

    /// Loads the sessions recorded before a restart, with readers that have
    /// seen what was already recorded.
    pub(crate) async fn load_sessions(&mut self) -> Result<(), CoreError> {
        let records = self.store.sessions().await?;
        self.take_sessions(records).await
    }

    /// How far each of the session's files was read, as the store has it.
    pub(crate) async fn stored_offsets(&self, record: &SessionRecord) -> Result<HashMap<String, SessionFileOffset>, CoreError> {
        Ok(self
            .store
            .session_offsets(&record.agent, &record.session_id)
            .await?
            .into_iter()
            .map(|offset| (offset.file.clone(), offset))
            .collect())
    }

    /// Takes on the sessions of `records`, reading what each recorded so far
    /// from its family's log.
    pub(crate) async fn take_sessions(&mut self, records: Vec<SessionRecord>) -> Result<(), CoreError> {
        let mut loaded: Vec<Session> = Vec::new();
        for record in records {
            let Some(family) = self.threads.get(&record.thread).map(|thread| thread.family) else {
                continue;
            };
            let Some(agent) = silt_agents::agent(&record.agent) else {
                continue;
            };
            let offsets = self.stored_offsets(&record).await?;
            loaded.push(Session {
                transcript: offsets.get("main").map(|offset| offset.path.clone()),
                record,
                family,
                agent,
                agent_cwd: PathBuf::new(),
                offsets,
                readers: HashMap::new(),
                in_turn: false,
                closed_tool: None,
                turn_closed: None,
                live: false,
                attached: HashMap::new(),
                recorded: HashSet::new(),
                snapshot_running: false,
                last_snapshot: None,
            });
        }
        let families: HashSet<FamilyId> = loaded.iter().map(|session| session.family).collect();
        for family in families {
            // A projection knows where each session started, so its log is
            // read only for readers that need what came before.
            let projected = self.families.get(&family);
            let reads = loaded.iter().any(|session| session.family == family && reads_recorded(session));
            if reads || projected.is_none() {
                read_recorded(&mut loaded, self.local_log(family));
            }
            if let Some(projected) = projected {
                for session in loaded.iter_mut().filter(|session| session.family == family) {
                    if let Some(cwd) = projected.session_cwd(&session.reference()) {
                        session.agent_cwd = PathBuf::from(cwd);
                    }
                }
            }
        }
        self.sessions.extend(loaded);
        Ok(())
    }

    /// Finds sessions waiting to be discovered and reads sessions kept in a
    /// store, both off the event loop.
    pub(crate) fn poll_sessions(&mut self) {
        self.pending.retain(|pending| {
            let alive = pending.searching || pending.started.elapsed() < PENDING_TTL;
            if !alive {
                warn!(thread = %pending.thread, agent = pending.agent.id(), "the agent never wrote its session; no longer looking for it");
            }
            alive
        });
        let known: Vec<String> = self.sessions.iter().map(|session| session.record.session_id.clone()).collect();
        for pending in &mut self.pending {
            if pending.searching || pending.last_search.is_some_and(|last| last.elapsed() < DISCOVER_INTERVAL) {
                continue;
            }
            pending.searching = true;
            pending.last_search = Some(Instant::now());
            let (agent, cwd, since, known, token) =
                (pending.agent, pending.agent_cwd.clone(), pending.since_ms, known.clone(), pending.token);
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
            let found = Found { id: session.record.session_id.clone(), transcript: transcript.clone() };
            let agent = session.agent;
            if agent.capture() != Capture::Snapshot {
                continue;
            }
            session.snapshot_running = true;
            let events = self.events.clone();
            let cwd = session.agent_cwd.clone();
            tokio::task::spawn_blocking(move || {
                let result = agent.snapshot(&found, &cwd).map_err(|error| error.to_string());
                let _ = events.send(Event::Snapshotted { session_id: found.id, result });
            });
        }
    }

    /// Looks for the sessions still unknown on `checkout` right away, so
    /// their edits are credited to their tool calls. Agents found through
    /// their own CLI are left to the background search.
    async fn discover_now(&mut self, checkout: CheckoutId) {
        let known: Vec<String> = self.sessions.iter().map(|session| session.record.session_id.clone()).collect();
        let tokens: Vec<u64> = self
            .pending
            .iter()
            .filter(|pending| {
                pending.checkout_id == checkout && !pending.searching && pending.agent.capture() != Capture::Snapshot
            })
            .map(|pending| pending.token)
            .collect();
        for token in tokens {
            let pending = self.pending.iter().find(|pending| pending.token == token).expect("listed above");
            let (agent, cwd, since, known) = (pending.agent, pending.agent_cwd.clone(), pending.since_ms, known.clone());
            // Discovery reads the agent's session files; off the event loop.
            let found = tokio::task::spawn_blocking(move || agent.discover(&cwd, since, &known)).await.unwrap_or_default();
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
        self.adopted.remove(&thread);
        self.find_waiting(thread, Waiting::GiveUp).await?;
        let ids = self.live_sessions(thread, |_| true);
        // Edits still waiting for the transcript are taken in now, while
        // the marks just read still own them.
        let checkouts: HashSet<CheckoutId> = self
            .sessions
            .iter()
            .filter(|session| session.record.thread == thread)
            .map(|session| session.record.checkout_id)
            .collect();
        for id in &ids {
            self.read_session(id, u64::MAX).await?;
        }
        // Once for all the thread's sessions, with their last marks read.
        for checkout in &checkouts {
            if self.checkouts.contains_key(checkout) {
                self.rescan(*checkout).await?;
            }
        }
        for id in &ids {
            self.close_open_tool(id).await?;
            if let Some(session) = self.session_mut(id) {
                session.live = false;
                session.in_turn = false;
            }
        }
        Ok("ended")
    }

    /// Takes over recording the thread's running agent from its wrapper,
    /// which recorded it on its own until now: its sessions are live here,
    /// in the turn the log says, and what changed meanwhile is taken in.
    pub(crate) async fn adopt(&mut self, thread: ThreadId) -> Result<&'static str, CoreError> {
        self.adopted.insert(thread);
        tracing::info!(%thread, "took over recording the agent from its wrapper");
        self.take_over_sessions(thread).await?;
        Ok("adopted")
    }

    /// Records the thread's running agent from here on: its sessions are
    /// live, in the turn the log says, and what changed meanwhile is taken
    /// in.
    pub(crate) async fn take_over_sessions(&mut self, thread: ThreadId) -> Result<(), CoreError> {
        let ids: Vec<String> = self
            .sessions
            .iter()
            .filter(|session| session.record.thread == thread)
            .map(|session| session.record.session_id.clone())
            .collect();
        let mut checkouts = HashSet::new();
        for id in &ids {
            let session = self.session(id).expect("listed above");
            let in_turn = self.families().get(&session.family).is_some_and(|family| family.in_turn(&session.reference()));
            let session = self.session_mut(id).expect("listed above");
            session.live = true;
            session.in_turn = in_turn;
            checkouts.insert(session.record.checkout_id);
            self.tail_session(id).await?;
        }
        for checkout in checkouts {
            if self.checkouts.contains_key(&checkout) {
                self.rescan(checkout).await?;
            }
        }
        Ok(())
    }

    /// The capture step for the thread's agent run by a process that isn't
    /// the daemon, which has no watcher to say when files change (ADR
    /// 0018): find its session if still unknown, and for an agent without
    /// hooks read what it wrote and take in the checkout's changes.
    pub(crate) async fn record_step(&mut self, thread: ThreadId) -> Result<(), CoreError> {
        self.find_waiting(thread, Waiting::Keep).await?;
        let ids = self.live_sessions(thread, |session| match session.agent.capture() {
            Capture::Hooks => false,
            Capture::Transcript => true,
            Capture::Snapshot => session.last_snapshot.is_none_or(|last| last.elapsed() >= SNAPSHOT_INTERVAL),
        });
        let mut checkouts = HashSet::new();
        for id in &ids {
            self.read_session(id, TAIL_CHUNK).await?;
            checkouts.extend(self.session(id).map(|session| session.record.checkout_id));
        }
        for checkout in checkouts {
            if self.checkouts.contains_key(&checkout) {
                self.rescan(checkout).await?;
            }
        }
        Ok(())
    }

    /// Whether the thread's agent needs `record_step`: its session is still
    /// unknown, or it has no hooks to capture with.
    pub(crate) fn steps(&self, thread: ThreadId) -> bool {
        self.pending.iter().any(|pending| pending.thread == thread)
            || !self.live_sessions(thread, |session| session.agent.capture() != Capture::Hooks).is_empty()
    }

    /// Whether an agent works in the thread on this machine.
    pub(crate) fn agent_live(&self, thread: ThreadId) -> bool {
        self.running(thread).is_ok() || !self.live_sessions(thread, |_| true).is_empty()
    }

    /// The live sessions of `thread` that `keep` keeps.
    fn live_sessions(&self, thread: ThreadId, keep: impl Fn(&Session) -> bool) -> Vec<String> {
        self.sessions
            .iter()
            .filter(|session| session.record.thread == thread && session.live && keep(session))
            .map(|session| session.record.session_id.clone())
            .collect()
    }

    /// Looks for the sessions the thread's agents haven't been found to
    /// write yet, and records those found.
    async fn find_waiting(&mut self, thread: ThreadId, unfound: Waiting) -> Result<(), CoreError> {
        let known: Vec<String> = self.sessions.iter().map(|session| session.record.session_id.clone()).collect();
        let waiting: Vec<PendingSession> = {
            let (waiting, rest) = std::mem::take(&mut self.pending).into_iter().partition(|pending| pending.thread == thread);
            self.pending = rest;
            waiting
        };
        for pending in waiting {
            let (agent, cwd, since) = (pending.agent, pending.agent_cwd.clone(), pending.since_ms);
            let known = known.clone();
            let found = tokio::task::spawn_blocking(move || agent.discover(&cwd, since, &known)).await?;
            match (found, unfound) {
                (Some(found), _) => self.register_found(pending, found).await?,
                (None, Waiting::Keep) => self.pending.push(pending),
                (None, Waiting::GiveUp) => warn!("the agent's session was never written"),
            }
        }
        Ok(())
    }

    /// Records what the session wrote since it was last read: up to about
    /// `limit` bytes of each transcript, or the whole of a store.
    async fn read_session(&mut self, session_id: &str, limit: u64) -> Result<(), CoreError> {
        let snapshot = self.session(session_id).and_then(|session| {
            let found = Found { id: session_id.to_owned(), transcript: session.transcript.clone()? };
            let agent = session.agent;
            (agent.capture() == Capture::Snapshot).then(|| (agent, found, session.agent_cwd.clone()))
        });
        let Some((agent, found, cwd)) = snapshot else {
            return self.tail_session_up_to(session_id, limit).await.map(|_| ());
        };
        let lines = tokio::task::spawn_blocking(move || agent.snapshot(&found, &cwd)).await?;
        if let Some(session) = self.session_mut(session_id) {
            session.last_snapshot = Some(Instant::now());
        }
        match lines {
            Ok(lines) => self.record_snapshot(session_id, lines).await,
            Err(error) => {
                warn!(%error, "could not read the session");
                Ok(())
            }
        }
    }

    /// Records new lines of a session's transcripts, a chunk at a time so a
    /// long backlog doesn't hold up the daemon; the rest is drained in
    /// bigger chunks, each its own event, so requests are answered between
    /// them.
    pub(crate) async fn tail_session(&mut self, session_id: &str) -> Result<(), CoreError> {
        if self.tail_session_up_to(session_id, TAIL_CHUNK).await? {
            self.drain_later(session_id);
        }
        Ok(())
    }

    fn drain_later(&mut self, session_id: &str) {
        if self.draining.insert(session_id.to_owned()) {
            let _ = self.events.send(Event::Drain { session_id: session_id.to_owned() });
        }
    }

    /// Records the next chunk of a session's backlog.
    pub(crate) async fn drain(&mut self, session_id: &str) {
        let more = self.tail_session_up_to(session_id, BACKLOG_CHUNK).await;
        if let Ok(true) = more {
            let _ = self.events.send(Event::Drain { session_id: session_id.to_owned() });
            return;
        }
        self.draining.remove(session_id);
        if let Err(error) = more {
            warn!(%error, session = session_id, "could not read the session transcript");
        }
        if let Some(family) = self.session(session_id).map(|session| session.family) {
            self.snapshot_if_due(family).await;
        }
    }

    /// Whether any of the session's transcripts has more to read.
    async fn tail_session_up_to(&mut self, session_id: &str, limit: u64) -> Result<bool, CoreError> {
        let Some(session) = self.session(session_id) else {
            return Ok(false);
        };
        let agent = session.agent;
        let transcript = match &session.transcript {
            Some(transcript) => transcript.clone(),
            None => {
                let Some(found) = agent.transcript(&session.agent_cwd, session_id) else {
                    return Ok(false);
                };
                let offset = SessionFileOffset { file: "main".to_owned(), path: found.clone(), offset: 0 };
                self.store.set_session_offset(agent.id(), session_id, &offset).await?;
                let session = self.session_mut(session_id).expect("found above");
                session.offsets.insert("main".to_owned(), offset);
                session.transcript = Some(found.clone());
                let (family, thread, reference) = (session.family, session.record.thread, session.reference());
                if let Some(dir) = agent.session_dir(&found) {
                    let op = ConvOp::SessionDir { session: reference.clone(), dir: dir.to_string_lossy().into_owned() };
                    self.append_conv(family, thread, agent_author(&reference), op).await?;
                }
                found
            }
        };
        if agent.capture() == Capture::Snapshot {
            return Ok(false);
        }
        let mut files = vec![("main".to_owned(), transcript.clone())];
        files.extend(agent.side_transcripts(&transcript).into_iter().map(|(name, path)| (format!("subagent:{name}"), path)));
        let mut more = false;
        for (key, path) in files {
            more |= self.tail_file(session_id, &key, &path, limit).await?;
        }
        self.attach_side_files(session_id, &transcript).await?;
        Ok(more)
    }

    /// Records the transcript's new complete lines, at most about `limit`
    /// bytes of them, in one write with how far the file was read, so a
    /// failure part way never records a line twice. Whether the file has
    /// more to read.
    async fn tail_file(&mut self, session_id: &str, key: &str, path: &Path, limit: u64) -> Result<bool, CoreError> {
        let session = self.session(session_id).expect("caller checked");
        let mut offset = session.offsets.get(key).map_or(0, |offset| offset.offset);
        let Ok(mut file) = fs::File::open(path) else {
            return Ok(false);
        };
        let length = file.metadata()?.len();
        if length < offset {
            // The agent rewound its transcript; what it dropped stays recorded.
            offset = length;
        }
        if length <= offset {
            return Ok(false);
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
            return Ok(false);
        }
        let read_to = |end: usize| SessionFileOffset { file: key.to_owned(), path: path.to_owned(), offset: offset + end as u64 };
        let mut batch = Batch::default();
        for (line, end) in lines {
            let staged = match self.stage_line(session_id, key, line) {
                Ok(staged) => staged,
                Err(error) => {
                    let read = read_to(batch.read);
                    self.store_batch(session_id, key, &mut batch, read).await?;
                    return Err(error);
                }
            };
            batch.push(staged.family, staged.delta, staged.rows, end);
            if !staged.marks.is_empty() {
                self.store_batch(session_id, key, &mut batch, read_to(end)).await?;
                self.apply_marks(session_id, staged.marks).await?;
            }
        }
        // Lines that recorded nothing still count as read.
        let read = read_to(consumed);
        if !batch.deltas.is_empty() {
            self.store_batch(session_id, key, &mut batch, read).await?;
        } else if self.session(session_id).and_then(|session| session.offsets.get(key)) != Some(&read) {
            let agent = self.session(session_id).expect("caller checked").record.agent.clone();
            self.store.set_session_offset(&agent, session_id, &read).await?;
            if let Some(session) = self.session_mut(session_id) {
                session.offsets.insert(key.to_owned(), read);
            }
        }
        Ok(offset + (consumed as u64) < length)
    }

    /// Stores one transcript record, indexes its text, and applies the turn
    /// marks it carries for agents without Silt's hooks. `read` is how far
    /// its file has been read with this line; it is saved with the record,
    /// so nothing that fails afterwards records the line again.
    pub(crate) async fn record_line(
        &mut self,
        session_id: &str,
        key: &str,
        line: String,
        read: Option<SessionFileOffset>,
    ) -> Result<(), CoreError> {
        let staged = self.stage_line(session_id, key, line)?;
        self.store_lines(session_id, key, staged.family, &[staged.delta], &staged.rows, read).await?;
        self.apply_marks(session_id, staged.marks).await
    }

    /// Applies a transcript record to its family in memory, with what it
    /// indexes and the turn marks it carries for agents without Silt's
    /// hooks.
    pub(crate) fn stage_line(&mut self, session_id: &str, key: &str, line: String) -> Result<StagedLine, CoreError> {
        // An agent still running may send a line after its thread was
        // purged here.
        let session = self
            .session_mut(session_id)
            .ok_or_else(|| CoreError::Invalid(format!("session {session_id} is no longer recorded here")))?;
        let agent = session.agent;
        let parsed = session.readers.entry(key.to_owned()).or_insert_with(|| agent.reader()).push(&line);
        let (family, thread, reference) = (session.family, session.record.thread, session.reference());
        let op = ConvOp::Native { session: reference.clone(), file: source_of(key), line };
        let texts: Vec<(String, String)> = parsed
            .entries
            .iter()
            .filter_map(|entry| {
                let text = entry.searchable_text().filter(|text| !text.trim().is_empty())?;
                Some((entry.kind().to_owned(), text))
            })
            .collect();
        let (delta, rows) = self.stage_conv(family, thread, agent_author(&reference), op, texts)?;
        let marks = if agent.capture() != Capture::Hooks && key == "main" { parsed.marks } else { Vec::new() };
        Ok(StagedLine { family, delta, rows, marks })
    }

    async fn store_batch(
        &mut self,
        session_id: &str,
        key: &str,
        batch: &mut Batch,
        read: SessionFileOffset,
    ) -> Result<(), CoreError> {
        let Batch { family: Some(family), deltas, rows, .. } = std::mem::take(batch) else {
            return Ok(());
        };
        self.store_lines(session_id, key, family, &deltas, &rows, Some(read)).await
    }

    /// Stores records staged from one session file, with how far the file
    /// has been read, all or none of them.
    async fn store_lines(
        &mut self,
        session_id: &str,
        key: &str,
        family: FamilyId,
        deltas: &[Delta],
        rows: &[SearchRow],
        read: Option<SessionFileOffset>,
    ) -> Result<(), CoreError> {
        let agent = self.session(session_id).map(|session| session.record.agent.clone()).unwrap_or_default();
        let progress = read.as_ref().map(|offset| SessionRead { agent: &agent, session_id, offset });
        self.persist_with(family, deltas, rows, progress).await?;
        if let (Some(read), Some(session)) = (read, self.session_mut(session_id)) {
            session.offsets.insert(key.to_owned(), read);
        }
        Ok(())
    }

    async fn apply_marks(&mut self, session_id: &str, marks: Vec<Mark>) -> Result<(), CoreError> {
        for mark in marks {
            self.apply_mark(session_id, mark).await?;
        }
        Ok(())
    }

    pub(crate) async fn apply_mark(&mut self, session_id: &str, mark: Mark) -> Result<(), CoreError> {
        let open = self.open_tool(session_id).map(str::to_owned);
        let session = self.session_mut(session_id).expect("caller checked");
        let mark = match mark {
            Mark::PromptSubmitted => {
                session.in_turn = true;
                TurnMark::PromptSubmitted
            }
            Mark::ToolStarted { tool_use_id, tool } => {
                // A transcript can read as stopped before the reply's tool
                // calls are written, as Gemini's does; a tool means the turn
                // goes on.
                session.in_turn = true;
                TurnMark::ToolStarted { tool_use_id, tool }
            }
            Mark::ToolFinished { tool_use_id } => {
                session.closed_tool = Some((tool_use_id.clone(), Instant::now()));
                TurnMark::ToolFinished { tool_use_id }
            }
            Mark::Stopped => {
                session.in_turn = false;
                if let Some(tool) = open {
                    session.closed_tool = Some((tool, Instant::now()));
                }
                session.turn_closed = Some(Instant::now());
                TurnMark::Stopped
            }
        };
        // As a hook does when a tool or turn ends (ADR 0018): what changed
        // was made by it, however late a busy machine's watcher would tell.
        let checkout = session.record.checkout_id;
        if matches!(mark, TurnMark::ToolFinished { .. } | TurnMark::Stopped) && self.checkouts.contains_key(&checkout) {
            self.rescan(checkout).await?;
        }
        self.record_turn(session_id, mark).await
    }

    /// The tool the session's marks say is open, as any process recording
    /// it reads them from the log.
    pub(crate) fn open_tool(&self, session_id: &str) -> Option<&str> {
        let session = self.session(session_id)?;
        self.families().get(&session.family)?.open_tool(&session.reference())
    }

    /// Ends the turn of a session that stopped with a tool still open, so
    /// the tool doesn't go on taking the credit for later edits.
    pub(crate) async fn close_open_tool(&mut self, session_id: &str) -> Result<(), CoreError> {
        if self.open_tool(session_id).is_some() {
            self.record_turn(session_id, TurnMark::Stopped).await?;
        }
        Ok(())
    }

    /// Records the session's side files as attachments when they are new or
    /// changed, so a restored session is complete.
    pub(crate) async fn attach_side_files(&mut self, session_id: &str, transcript: &Path) -> Result<(), CoreError> {
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
                Op::Conv(ConvOp::Attachment { session, name: attached, blob }) if *session == reference && *attached == name => {
                    Some(*blob)
                }
                _ => None,
            });
            if known != Some(blob) {
                let op = ConvOp::Attachment { session: reference.clone(), name: name.clone(), blob };
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
    pub(crate) async fn tail_marked_sessions(&mut self, checkout: CheckoutId) {
        self.discover_now(checkout).await;
        let ids: Vec<String> = self
            .sessions
            .iter()
            .filter(|session| session.record.checkout_id == checkout && session.live && session.agent.capture() != Capture::Hooks)
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
    pub(crate) fn marks_from_transcripts(&self, checkout: CheckoutId) -> bool {
        self.pending.iter().any(|pending| pending.checkout_id == checkout)
            || self.sessions.iter().any(|session| {
                session.record.checkout_id == checkout && session.live && session.agent.capture() != Capture::Hooks
            })
    }

    /// Takes the changes in again once the transcripts have caught up.
    pub(crate) fn settle_changes(&mut self, checkout: CheckoutId, paths: Vec<PathBuf>, noticed: Instant) {
        let events = self.events.clone();
        tokio::spawn(async move {
            tokio::time::sleep(TRANSCRIPT_LAG).await;
            let _ = events.send(Event::Changed { checkout, paths, noticed, settled: true });
        });
    }

    pub(crate) fn author_for(&self, checkout: CheckoutId) -> Author {
        let sessions: Vec<attribution::SessionState<'_>> = self
            .sessions
            .iter()
            .filter(|session| session.record.checkout_id == checkout)
            .map(|session| attribution::SessionState {
                agent: &session.record.agent,
                session: &session.record.session_id,
                capture: session.agent.capture(),
                live: session.live,
                in_turn: session.in_turn,
                open_tool: self.open_tool(&session.record.session_id),
                closed_tool: session.closed_tool.as_ref().map(|(tool, at)| (tool.as_str(), *at)),
                turn_closed: session.turn_closed,
            })
            .collect();
        // An agent that hasn't written its session yet, but is running.
        let pending = self.pending.iter().find(|pending| pending.checkout_id == checkout).map(|pending| pending.agent.id());
        let noticed = self.change_noticed.unwrap_or_else(Instant::now);
        attribution::author(self.importing, &sessions, pending, noticed)
    }

    pub(crate) async fn hook(&mut self, event: &str, input: serde_json::Value) -> Response {
        match serde_json::from_value::<silt_agents::claude::HookInput>(input) {
            Ok(input) => to_response(self.capture(event, &input).await),
            Err(error) => Response::error(error),
        }
    }

    /// What the agent hears with a prompt: Silt's notices and teammates'
    /// live drafts.
    pub(crate) async fn prompt_context(&mut self, thread: ThreadId) -> Option<String> {
        let parts: Vec<String> =
            [self.take_notices(thread).await, self.teammates_drafts(thread).await].into_iter().flatten().collect();
        (!parts.is_empty()).then(|| parts.join("\n\n"))
    }

    /// Records a turn mark, unless the log has it already: a mark seen by
    /// two processes, or read again, is recorded once.
    pub(crate) async fn record_turn(&mut self, session_id: &str, mark: TurnMark) -> Result<(), CoreError> {
        let Some(session) = self.session(session_id) else {
            return Ok(());
        };
        let reference = session.reference();
        let (family, thread) = (session.family, session.record.thread);
        if self.families().get(&family).is_some_and(|loaded| loaded.has_turn_mark(&reference, &mark)) {
            return Ok(());
        }
        self.note_git(session_id, &mark).await?;
        let author = agent_author(&reference);
        self.append_conv(family, thread, author, ConvOp::Turn { session: reference, mark }).await?;
        Ok(())
    }

    /// Keeps the git state a tool call starts at, and records when it ends
    /// whether git rewrote the worktree meanwhile, so the edits credited to
    /// it read as git's (ADR 0066). In the shared step, as a hook's process
    /// may see the start and another the end (ADR 0018).
    async fn note_git(&mut self, session_id: &str, mark: &TurnMark) -> Result<(), CoreError> {
        let Some(session) = self.session(session_id) else {
            return Ok(());
        };
        let (reference, family, thread, checkout) =
            (session.reference(), session.family, session.record.thread, session.record.checkout_id);
        let Some(repo) = self.checkouts.get(&checkout).map(|checkout| &checkout.repo) else {
            return Ok(());
        };
        let ended = match mark {
            TurnMark::ToolStarted { tool_use_id, .. } => {
                let state = repo.git_state().to_string();
                self.store.put_tool_git_state(&reference.agent, &reference.id, tool_use_id, &state).await?;
                return Ok(());
            }
            TurnMark::ToolFinished { tool_use_id } => Some(tool_use_id.clone()),
            TurnMark::Stopped => self.open_tool(session_id).map(str::to_owned),
            TurnMark::PromptSubmitted => return Ok(()),
        };
        let started = match &ended {
            Some(tool) => self.store.take_tool_git_state(&reference.agent, &reference.id, tool).await?,
            None => None,
        };
        if matches!(mark, TurnMark::Stopped) {
            self.store.forget_tool_git_states(&reference.agent, &reference.id).await?;
        }
        let (Some(tool_use_id), Some(started)) = (ended, started) else {
            return Ok(());
        };
        let Ok(started) = started.parse::<silt_git::GitState>() else {
            return Ok(());
        };
        if !self.checkouts[&checkout].repo.rewritten_since(&started) {
            return Ok(());
        }
        let head = self.families()[&family].thread_head(thread);
        self.append_op(family, thread, head, Op::Git(GitOp::Rewrote { session: reference, tool_use_id })).await?;
        Ok(())
    }

    pub(crate) fn session(&self, session_id: &str) -> Option<&Session> {
        self.sessions.iter().find(|session| session.record.session_id == session_id)
    }

    pub(crate) fn session_mut(&mut self, session_id: &str) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|session| session.record.session_id == session_id)
    }
}
