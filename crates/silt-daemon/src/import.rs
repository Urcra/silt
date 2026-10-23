//! Importing the sessions agents already have on disk, each into a thread
//! of its own kept on this machine until sent (ADR 0043).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use silt_agents::{Agent, Capture, Found, OnDisk, complete_lines_ending};
use silt_checkout::{ImportFile, import};
use silt_git::{FileMode, Repo};
use silt_log::{Delta, Family};
use silt_store::{CheckoutId, ImportRecord, SearchRow, SessionRecord, ThreadRecord};
use silt_types::{FamilyId, ThreadId};

use crate::core::{Core, CoreError, now_ms};
use crate::homes::Home;
use crate::protocol::{ImportCandidate, ImportedThread, Sent};
use crate::sessions::{Session, resolve_agent};

/// Records staged before they are stored in one write.
const BATCH: usize = 4096;

impl From<(&str, OnDisk)> for ImportCandidate {
    fn from((agent, found): (&str, OnDisk)) -> Self {
        Self { agent: agent.to_owned(), id: found.id, transcript: found.transcript, cwd: found.cwd, started_ms: found.started_ms }
    }
}

impl Core {
    /// The sessions on disk not yet recorded or imported, of `agent` or of
    /// every agent, that ran under one of `paths` or anywhere without any.
    pub(crate) async fn import_candidates(
        &self,
        paths: &[PathBuf],
        agent: Option<&str>,
    ) -> Result<Vec<ImportCandidate>, CoreError> {
        let agents: Vec<&'static dyn Agent> = match agent {
            Some(name) => vec![resolve_agent(name)?],
            None => silt_agents::AGENTS.to_vec(),
        };
        let paths: Vec<PathBuf> = paths.iter().map(|path| path.canonicalize().unwrap_or_else(|_| path.clone())).collect();
        let known = self.known_sessions().await?;
        let mut found: Vec<ImportCandidate> = Vec::new();
        // Agents listed per directory are asked last, in the directories the
        // others' sessions ran in too.
        let (by_directory, whole): (Vec<_>, Vec<_>) = agents.into_iter().partition(|agent| agent.capture() == Capture::Snapshot);
        for agent in whole {
            found.extend(agent.on_disk(&[]).into_iter().map(|session| ImportCandidate::from((agent.id(), session))));
        }
        let mut dirs: BTreeSet<PathBuf> = self.clones.values().cloned().collect();
        dirs.extend(paths.iter().cloned());
        dirs.extend(found.iter().filter_map(|candidate| candidate.cwd.clone()));
        let dirs: Vec<PathBuf> = dirs.into_iter().collect();
        for agent in by_directory {
            let dirs = dirs.clone();
            let listed = tokio::task::spawn_blocking(move || agent.on_disk(&dirs)).await?;
            found.extend(listed.into_iter().map(|session| ImportCandidate::from((agent.id(), session))));
        }
        found.retain(|candidate| {
            !known.contains(&(candidate.agent.clone(), candidate.id.clone()))
                && (paths.is_empty()
                    || candidate.cwd.as_deref().is_some_and(|cwd| paths.iter().any(|path| cwd.starts_with(path))))
        });
        found.sort_by(|a, b| a.started_ms.cmp(&b.started_ms).then_with(|| a.id.cmp(&b.id)));
        Ok(found)
    }

    async fn known_sessions(&self) -> Result<HashSet<(String, String)>, CoreError> {
        let recorded = self.store.sessions().await?.into_iter().map(|record| (record.agent, record.session_id));
        let imported = self.store.imports().await?.into_iter().map(|record| (record.agent, record.session_id));
        Ok(recorded.chain(imported).collect())
    }

    /// Imports one session into a new thread, or `None` when it was
    /// recorded or imported before.
    pub(crate) async fn import_session(&mut self, candidate: ImportCandidate) -> Result<Option<ImportedThread>, CoreError> {
        let agent = resolve_agent(&candidate.agent)?;
        if !silt_agents::is_safe_id(&candidate.id) {
            return Err(CoreError::Invalid(format!("{:?} isn't a session id", candidate.id)));
        }
        if self.known_sessions().await?.contains(&(agent.id().to_owned(), candidate.id.clone())) {
            return Ok(None);
        }
        let started = candidate
            .started_ms
            .or_else(|| fs::metadata(&candidate.transcript).ok()?.modified().ok().map(system_ms))
            .unwrap_or_else(now_ms);
        let cwd = candidate.cwd.clone().unwrap_or_default();
        let repo = candidate.cwd.as_deref().filter(|cwd| cwd.is_dir()).and_then(|cwd| Repo::discover(cwd).ok());
        let project = match &repo {
            Some(repo) => repo.remote_url("origin")?.map(|url| silt_git::normalize_remote(&url)).unwrap_or_default(),
            None => String::new(),
        };
        let base = match &repo {
            Some(repo) => repo.commit_at(started)?,
            None => None,
        };

        let thread = ThreadId(uuid::Uuid::now_v7().as_u128());
        let family_id = FamilyId(uuid::Uuid::now_v7().as_u128());
        self.set_home(family_id, Home::Local).await?;
        let record = ThreadRecord {
            id: thread,
            family: family_id,
            repo_root: repo.as_ref().map(|repo| repo.root().to_owned()).unwrap_or_default(),
            repo: project.clone(),
            created_ms: started,
            owner: String::new(),
        };
        self.store.create_thread(&record).await?;
        self.register_thread(&record);
        self.threads.insert(thread, record);
        let mut family = Family::new();
        let deltas = import(&mut family, self.thread_actor(thread), thread, base.map(|commit| (commit, None)), &[], started)?;
        self.store.append_deltas(family_id, &deltas).await?;
        self.families.insert(family_id, family);
        self.announce_family(family_id, &deltas);
        self.store
            .add_import(
                &ImportRecord { agent: agent.id().to_owned(), session_id: candidate.id.clone(), thread, family: family_id },
                now_ms(),
            )
            .await?;

        let checkout_root = repo.as_ref().map_or_else(|| cwd.clone(), |repo| repo.root().to_owned());
        self.sessions.push(imported_session(agent, &candidate, thread, family_id, &cwd));
        self.recorded_at = Some(started);
        let read = self.read_imported(agent, &candidate, &checkout_root, started).await;
        self.recorded_at = None;
        self.sessions.retain(|session| session.record.session_id != candidate.id || session.record.thread != thread);
        let lines = read?;
        self.announce(crate::protocol::BusEvent::Threads);
        Ok(Some(ImportedThread {
            thread: thread.to_string(),
            agent: agent.id().to_owned(),
            session_id: candidate.id,
            project,
            cwd: candidate.cwd,
            started_ms: started,
            lines,
        }))
    }

    /// Records everything the session's files hold, with the times its
    /// records carry. How many records it read.
    async fn read_imported(
        &mut self,
        agent: &'static dyn Agent,
        candidate: &ImportCandidate,
        checkout_root: &Path,
        started: i64,
    ) -> Result<usize, CoreError> {
        let thread = self.session(&candidate.id).map(|session| session.record.thread).expect("pushed by the caller");
        let family = self.threads[&thread].family;
        let reference = self.session(&candidate.id).expect("pushed by the caller").reference();
        self.append_session_started(family, thread, reference, candidate.cwd.as_deref().unwrap_or(Path::new("")), checkout_root)
            .await?;
        if agent.capture() == Capture::Snapshot {
            let found = Found { id: candidate.id.clone(), transcript: candidate.transcript.clone() };
            let lines = tokio::task::spawn_blocking(move || agent.snapshot(&found))
                .await?
                .map_err(|source| CoreError::Agent { doing: "export the session", source })?;
            return self.import_lines(&candidate.id, "main", lines, started).await;
        }
        let mut files = vec![("main".to_owned(), candidate.transcript.clone())];
        files.extend(
            agent.side_transcripts(&candidate.transcript).into_iter().map(|(name, path)| (format!("subagent:{name}"), path)),
        );
        let mut read = 0;
        for (key, path) in files {
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };
            let (lines, _) = complete_lines_ending(&bytes);
            let lines = lines.into_iter().map(|(line, _)| line).collect();
            read += self.import_lines(&candidate.id, &key, lines, started).await?;
        }
        if let Some(dir) = agent.session_dir(&candidate.transcript) {
            let reference = self.session(&candidate.id).expect("pushed by the caller").reference();
            let op = silt_log::ConvOp::SessionDir { session: reference.clone(), dir: dir.to_string_lossy().into_owned() };
            self.append_conv(family, thread, crate::sessions::agent_author(&reference), op).await?;
        }
        self.attach_side_files(&candidate.id, &candidate.transcript).await?;
        self.close_open_tool(&candidate.id).await?;
        Ok(read)
    }

    /// Stages and stores one file's records in batches, a batch ending
    /// before each turn mark so marks follow the records they belong to.
    async fn import_lines(&mut self, session_id: &str, key: &str, lines: Vec<String>, started: i64) -> Result<usize, CoreError> {
        let count = lines.len();
        let mut time = started;
        let mut deltas: Vec<Delta> = Vec::new();
        let mut rows: Vec<SearchRow> = Vec::new();
        let mut family = None;
        for line in lines {
            time = silt_agents::record_time(&line).unwrap_or(time);
            self.recorded_at = Some(time);
            let staged = self.stage_line(session_id, key, line)?;
            family = Some(staged.family);
            deltas.push(staged.delta);
            rows.extend(staged.rows);
            if !staged.marks.is_empty() || deltas.len() >= BATCH {
                self.persist_with(staged.family, &std::mem::take(&mut deltas), &std::mem::take(&mut rows), None).await?;
                for mark in staged.marks {
                    self.apply_mark(session_id, mark).await?;
                }
            }
        }
        if let Some(family) = family.filter(|_| !deltas.is_empty()) {
            self.persist_with(family, &deltas, &rows, None).await?;
        }
        Ok(count)
    }

    /// Gives a thread with a genesis that never had a file the genesis
    /// commit's tree, as a new thread starts with, before it is
    /// checked out: imported threads leave it until needed (ADR 0043).
    pub(crate) async fn import_tree_if_missing(&mut self, thread: ThreadId, repo: &Repo) -> Result<(), CoreError> {
        let family_id = self.threads[&thread].family;
        // Trashed files stay in the tree, so a thread that deleted every
        // file isn't mistaken for one that never had any.
        if !self.families[&family_id].thread_view(thread).tree().inodes().is_empty() {
            return Ok(());
        }
        let Some(commit) = self.genesis_commit(family_id)? else {
            return Ok(());
        };
        let files: Vec<ImportFile> = repo
            .files_at(commit)?
            .into_iter()
            .filter(|file| file.mode != FileMode::Symlink)
            .map(|file| ImportFile { path: file.path, oid: file.oid, executable: file.mode == FileMode::Executable })
            .collect();
        if files.is_empty() {
            return Ok(());
        }
        let actor = self.thread_actor(thread);
        let family = self.families.get_mut(&family_id).expect("projected above");
        // `import` applies them; storing them is what's left.
        let created = import(family, actor, thread, None, &files, now_ms())?;
        self.persist(family_id, &created).await
    }

    /// Sends imported threads kept on this machine to a server signed in
    /// to (ADR 0043): `threads`, those of `project`, or with neither all.
    pub(crate) async fn send(
        &mut self,
        threads: &[String],
        project: Option<&str>,
        server: Option<&str>,
    ) -> Result<Sent, CoreError> {
        let remote = match server {
            Some(url) => {
                let url = url.trim_end_matches('/');
                self.remotes.iter().find(|remote| remote.config.url == url).cloned().ok_or_else(|| {
                    CoreError::Invalid(format!("not signed in to {url}; run `silt auth login --server {url}` first"))
                })?
            }
            None => self
                .default_remote()
                .cloned()
                .ok_or_else(|| CoreError::Invalid("not connected to a server; use `silt auth login`".into()))?,
        };
        let held: HashMap<ThreadId, FamilyId> = self
            .store
            .imports()
            .await?
            .into_iter()
            .filter(|record| self.threads.contains_key(&record.thread) && self.homes.get(&record.family) == Some(&Home::Local))
            .map(|record| (record.thread, record.family))
            .collect();
        let chosen: Vec<ThreadId> = if threads.is_empty() {
            held.keys().copied().filter(|thread| project.is_none_or(|project| self.threads[thread].repo == project)).collect()
        } else {
            let mut chosen = Vec::new();
            for selector in threads {
                let thread = self.find_thread(selector)?;
                if !held.contains_key(&thread) {
                    return Err(CoreError::Invalid(format!(
                        "thread {} isn't an imported thread kept on this machine",
                        thread.short()
                    )));
                }
                chosen.push(thread);
            }
            chosen
        };
        // The secrets file as it is now: the guide says to add old secrets
        // to it before sending.
        let added = crate::core::redactor(&self.data_dir).secrets().to_vec();
        self.redactor = self.redactor.clone().with(added.iter().cloned());
        self.learn_secrets(added);
        let mut secrets = BTreeSet::new();
        for thread in &chosen {
            let family = held[thread];
            for delta in self.local_log(family) {
                secrets.extend(self.redactor.redact_delta(delta).found);
            }
            self.set_home(family, Home::Server(remote.config.url.clone())).await?;
            let record = ThreadRecord { owner: remote.config.user.clone(), ..self.threads[thread].clone() };
            self.store.create_thread(&record).await?;
            self.threads.insert(*thread, record);
        }
        self.drive_sync();
        self.announce(crate::protocol::BusEvent::Threads);
        Ok(Sent { threads: chosen.len(), server: remote.config.url.clone(), redacted: secrets.len() })
    }

    /// Imported families kept on this machine until sent, which a project's
    /// new home doesn't move.
    pub(crate) async fn held_imports(&self) -> Result<HashSet<FamilyId>, CoreError> {
        Ok(self
            .store
            .imports()
            .await?
            .into_iter()
            .map(|record| record.family)
            .filter(|family| self.homes.get(family) == Some(&Home::Local))
            .collect())
    }
}

/// The session being imported, while its records are read: never live, in
/// no checkout.
fn imported_session(
    agent: &'static dyn Agent,
    candidate: &ImportCandidate,
    thread: ThreadId,
    family: FamilyId,
    cwd: &Path,
) -> Session {
    Session {
        record: SessionRecord {
            agent: agent.id().to_owned(),
            session_id: candidate.id.clone(),
            thread,
            checkout_id: CheckoutId(-1),
        },
        family,
        agent,
        agent_cwd: cwd.to_owned(),
        transcript: Some(candidate.transcript.clone()),
        offsets: HashMap::new(),
        readers: HashMap::new(),
        in_turn: false,
        closed_tool: None,
        turn_closed: None,
        live: false,
        attached: HashMap::new(),
        recorded: HashSet::new(),
        snapshot_running: false,
        last_snapshot: None::<Instant>,
    }
}

fn system_ms(time: std::time::SystemTime) -> i64 {
    time.duration_since(std::time::UNIX_EPOCH).map_or(0, |elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
}
