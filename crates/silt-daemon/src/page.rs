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
        let docs = match self.families.get(&family_id) {
            Some(family) => family.page_documents(thread, artifact),
            // A family known only from the server is folded from its log.
            None => {
                let log = self.local_log(family_id);
                silt_log::page_documents_in(log, &graph_of(log), thread, artifact)
            }
        };
        if let Some(read) = silt_proto::page::read(&docs, &request, &rules, &me, level) {
            return Ok(read);
        }
        let writes = request.writes().map_err(CoreError::Invalid)?;
        let ops = silt_proto::page::page_ops(artifact, &docs, writes, &rules, &me, level).map_err(CoreError::Invalid)?;
        let written = ops.len();
        for op in ops {
            self.append_page(family_id, thread, op).await?;
        }
        Ok(serde_json::json!({ "written": written }))
    }

    /// Writes `op` on `thread`: through the family's projection where this
    /// machine has one, else onto the log it keeps from the server, as page
    /// data needs no file's text.
    async fn append_page(&mut self, family_id: FamilyId, thread: ThreadId, op: PageOp) -> Result<(), CoreError> {
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
    fn page_level(&self, thread: ThreadId) -> Level {
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
fn graph_of(log: &[Delta]) -> CausalGraph {
    let mut graph = CausalGraph::new();
    for delta in log {
        // A delta whose parents the log lacks has no place in a lineage.
        let _ = graph.add(delta.span, &delta.parents);
    }
    graph
}
