use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use silt_agents::claude;
use silt_crdt::InodeKind;
use silt_git::Repo;
use silt_log::{Author, ConvOp, Family, GitOp, Op, SessionFile, SessionRef};
use silt_store::{SessionFileOffset, SessionRecord, ThreadRecord};
use silt_types::{ActorId, FamilyId, GitOid, ThreadId};
use tracing::warn;

use crate::core::{Core, CoreError, RepoBases, Session, now_ms};
use crate::disk;
use crate::protocol::Resumed;

impl Core {
    /// Continues a thread on this machine (ADR 0009): takes the lease,
    /// materializes the thread's files in a checkout and restores its agent
    /// session there.
    pub(crate) async fn resume(&mut self, selector: &str, cwd: &Path, here: bool) -> Result<Resumed, CoreError> {
        let thread = self.find_thread(selector)?;
        let record = self.threads[&thread].clone();
        let cwd = cwd.canonicalize()?;
        let repo = Repo::discover(&cwd)?;
        let identity = repo
            .remote_url("origin")?
            .map(|url| silt_git::normalize_remote(&url))
            .unwrap_or_default();
        if identity != record.repo {
            return Err(CoreError::Invalid(format!(
                "this clone's origin is {identity:?} but the thread belongs to {:?}",
                record.repo
            )));
        }

        if here && !repo.git(&["status", "--porcelain"], None)?.is_empty() {
            return Err(CoreError::Invalid("--here needs a clean working tree".into()));
        }

        self.catch_up(record.family).await?;
        let base = self.genesis_commit(record.family)?;
        if let Some(base) = base {
            self.ensure_commit(&repo, record.family, thread, base).await?;
        }
        self.project_family(record.family, &repo)?;
        let updated = ThreadRecord {
            repo_root: repo.root().to_owned(),
            ..record.clone()
        };
        self.store.create_thread(&updated).await?;
        self.threads.insert(thread, updated);
        self.take_lease(thread).await?;

        let checkout_path = if here {
            if let Some(base) = base {
                repo.git(&["checkout", "-q", "--detach", &base.to_string()], None)?;
            }
            repo.root().to_owned()
        } else {
            let base = base.ok_or_else(|| CoreError::Invalid("the thread has no base commit".into()))?;
            let path = repo.managed_checkout_path(&thread.to_string());
            if path.exists() {
                path
            } else {
                repo.create_managed_checkout(&thread.to_string(), base, None)?.path
            }
        };
        let checkout_id = self.attach_checkout(thread, &checkout_path, !here).await?;
        self.materialize(checkout_id).await?;

        let (session_id, agent_cwd) = match self.restore_session(thread, record.family, &checkout_path).await? {
            Some((session_id, agent_cwd)) => (Some(session_id), agent_cwd),
            None => (None, checkout_path.clone()),
        };
        Ok(Resumed {
            thread: thread.to_string(),
            checkout: checkout_path,
            agent_cwd,
            session_id,
        })
    }

    /// Pulls everything the server has for the family, so a resume never
    /// starts from a partial thread.
    async fn catch_up(&mut self, family: FamilyId) -> Result<(), CoreError> {
        let Some(remote) = self.remote.clone() else {
            return Ok(());
        };
        let mut after = 0;
        let mut last = None;
        loop {
            let (_, outcome, error) = remote.pull(family, after, last, std::time::Duration::ZERO).await;
            let deltas = match outcome {
                silt_sync::Outcome::Pulled(deltas) => deltas,
                _ => {
                    return Err(CoreError::Invalid(format!(
                        "could not fetch the thread from the server: {}",
                        error.map_or_else(|| "no answer".to_owned(), |error| error.to_string())
                    )));
                }
            };
            let Some((seq, delta)) = deltas.last() else {
                return Ok(());
            };
            (after, last) = (*seq, Some(delta.span.first()));
            self.apply_pulled(family, deltas.into_iter().map(|(_, delta)| delta).collect()).await?;
        }
    }

    fn genesis_commit(&self, family: FamilyId) -> Result<Option<GitOid>, CoreError> {
        Ok(self.local_log(family).iter().find_map(|delta| match &delta.op {
            Op::Git(GitOp::Genesis { commit, .. }) => Some(*commit),
            _ => None,
        }))
    }

    /// Makes sure `commit` is in `repo`: from origin, else from the family's
    /// bundles.
    async fn ensure_commit(&self, repo: &Repo, family: FamilyId, thread: ThreadId, commit: GitOid) -> Result<(), CoreError> {
        if repo.has_commit(commit) {
            return Ok(());
        }
        repo.fetch_origin();
        if repo.has_commit(commit) {
            return Ok(());
        }
        let bundles: Vec<_> = self
            .local_log(family)
            .iter()
            .filter_map(|delta| match &delta.op {
                Op::Git(GitOp::Bundle { commits, blob }) => Some((commits.clone(), *blob)),
                _ => None,
            })
            .collect();
        for (commits, blob) in bundles.into_iter().rev() {
            let bytes = self.blob(blob).await?;
            if let Err(error) = repo.fetch_bundle(&bytes, &thread.to_string()) {
                warn!(%error, "could not fetch a bundle");
            }
            if commits.contains(&commit) && repo.has_commit(commit) {
                return Ok(());
            }
        }
        if repo.has_commit(commit) {
            Ok(())
        } else {
            Err(CoreError::Invalid(format!("commit {commit} is neither on origin nor in the thread's bundles")))
        }
    }

    /// Turns a family known only from the server into a projection, reading
    /// git blobs from `repo`.
    fn project_family(&mut self, family_id: FamilyId, repo: &Repo) -> Result<(), CoreError> {
        let Some(deltas) = self.raw.remove(&family_id) else {
            return Ok(());
        };
        let mut family = Family::new();
        for delta in deltas {
            family.apply(delta, &mut RepoBases(repo))?;
        }
        self.families.insert(family_id, family);
        Ok(())
    }

    async fn take_lease(&mut self, thread: ThreadId) -> Result<(), CoreError> {
        let Some(remote) = self.remote.clone() else {
            return Ok(());
        };
        let lease = remote
            .take_lease(thread)
            .await
            .map_err(|error| CoreError::Invalid(format!("could not take the thread's lease: {error}")))?;
        let family_id = self.threads[&thread].family;
        let family = self.families.get_mut(&family_id).expect("projected above");
        let delta = family.delta(
            self.actor,
            thread,
            Author::User,
            now_ms(),
            family.thread_head(thread),
            Op::Lease {
                device: lease.device,
                epoch: lease.epoch,
            },
        );
        family.apply(delta.clone(), &mut HashMap::new())?;
        self.store.append_deltas(family_id, &[delta]).await?;
        Ok(())
    }

    async fn attach_checkout(&mut self, thread: ThreadId, path: &Path, managed: bool) -> Result<i64, CoreError> {
        let actor = ActorId(uuid::Uuid::new_v4().as_u128());
        let existing = self
            .checkouts
            .values()
            .find(|checkout| checkout.record.path == path)
            .map(|checkout| checkout.record.clone());
        let record = match existing {
            Some(previous) => {
                self.checkouts.remove(&previous.id);
                self.store.rebind_checkout(previous.id, thread, actor).await?;
                silt_store::CheckoutRecord {
                    thread,
                    actor,
                    ..previous
                }
            }
            None => self.store.add_checkout(thread, path, actor, managed, now_ms()).await?,
        };
        let id = record.id;
        self.start_checkout(record, BTreeMap::new())?;
        Ok(id)
    }

    /// Writes the family's files into a fresh checkout and removes files the
    /// thread deleted, so the following rescan finds nothing to record.
    async fn materialize(&mut self, checkout_id: i64) -> Result<(), CoreError> {
        let checkout = &self.checkouts[&checkout_id];
        let family_id = checkout.family;
        let family = &self.families[&family_id];
        let files: Vec<_> = family
            .tree()
            .inodes()
            .into_iter()
            .filter(|inode| family.tree().kind(*inode) != Some(InodeKind::Dir))
            .collect();
        let actions = checkout
            .bridge
            .reconcile(family, &mut RepoBases(&checkout.repo), files);
        let paths = family.tree().paths();
        let root = checkout.record.path.clone();
        let stale: Vec<String> = checkout
            .repo
            .worktree_files()?
            .into_iter()
            .filter(|path| !paths.contains_key(path) && !disk::is_internal(path))
            .collect();
        for path in stale {
            disk::remove(&root, &path)?;
        }
        let again = self.perform_with(checkout_id, actions, true).await?;
        for path in again {
            self.ingest_file(checkout_id, &path).await?;
        }
        Ok(())
    }

    /// Restores the thread's latest agent session in the checkout and starts
    /// tailing it after the restored lines.
    async fn restore_session(
        &mut self,
        thread: ThreadId,
        family_id: FamilyId,
        checkout: &Path,
    ) -> Result<Option<(String, PathBuf)>, CoreError> {
        let family = &self.families[&family_id];
        let Some(session) = family.conversation(thread).rev().find_map(|delta| match &delta.op {
            Op::Conv(ConvOp::Native { session, .. }) => Some(session.clone()),
            _ => None,
        }) else {
            return Ok(None);
        };
        let mut old_cwd = String::new();
        let mut relative = PathBuf::new();
        let mut main = Vec::new();
        let mut subagents: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for delta in family.conversation(thread) {
            match &delta.op {
                Op::Conv(ConvOp::SessionStarted {
                    session: started,
                    cwd,
                    checkout_root,
                }) if *started == session => {
                    old_cwd.clone_from(cwd);
                    relative = Path::new(cwd).strip_prefix(checkout_root).map(Path::to_path_buf).unwrap_or_default();
                }
                Op::Conv(ConvOp::Native {
                    session: from,
                    file,
                    line,
                }) if *from == session => match file {
                    SessionFile::Main => main.push(line.clone()),
                    SessionFile::Subagent(name) => subagents.entry(name.clone()).or_default().push(line.clone()),
                },
                _ => {}
            }
        }
        let agent_cwd = if relative.as_os_str().is_empty() {
            checkout.to_owned()
        } else {
            checkout.join(relative)
        };
        let config = claude::config_dir().ok_or_else(|| CoreError::Invalid("HOME is not set".into()))?;
        let subagents: Vec<(String, Vec<String>)> = subagents.into_iter().collect();
        let paths = claude::restore(&config, &agent_cwd, &session.id, &old_cwd, &main, &subagents)?;

        let checkout_id = self
            .checkouts
            .values()
            .find(|candidate| candidate.record.path == checkout)
            .map(|candidate| candidate.record.id)
            .expect("attached above");
        let record = SessionRecord {
            agent: session.agent.clone(),
            session_id: session.id.clone(),
            thread,
            checkout_id,
        };
        self.store.add_session(&record, now_ms()).await?;
        let mut offsets = HashMap::new();
        let mut files = vec![("main".to_owned(), paths.transcript.clone())];
        files.extend(
            subagents
                .iter()
                .map(|(name, _)| (format!("subagent:{name}"), paths.subagents.join(format!("{name}.jsonl")))),
        );
        for (key, path) in files {
            let offset = SessionFileOffset {
                file: key.clone(),
                offset: std::fs::metadata(&path).map_or(0, |metadata| metadata.len()),
                path,
            };
            self.store.set_session_offset(&session.agent, &session.id, &offset).await?;
            offsets.insert(key, offset);
        }
        self.sessions.retain(|existing| existing.record.session_id != session.id);
        self.sessions.push(Session {
            record,
            family: family_id,
            transcript: paths.transcript,
            offsets,
            open_tool: None,
        });
        let reference = SessionRef {
            agent: session.agent.clone(),
            id: session.id.clone(),
        };
        self.append_session_started(family_id, thread, reference, &agent_cwd, checkout).await?;
        Ok(Some((session.id, agent_cwd)))
    }
}
