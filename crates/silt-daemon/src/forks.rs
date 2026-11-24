//! Pulls, merge requests and visibility (ADR 0010). Forking itself shares
//! its checkout and session restore with resume, in `resume.rs`.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use silt_agents::{Entry, Transcripts};
use silt_crdt::VersionVector;
use silt_log::{Author, ConvOp, Delta, Family, HistorySource, MergeRequest, MrOp, MrStatus, Op, ThreadState};
use silt_proto::Visibility;
use silt_types::{DeltaId, FamilyId, Frontier, ThreadId};
use similar::{DiffTag, TextDiff};
use tracing::warn;

use crate::core::{CanonicalCwd, Core, CoreError, now_ms, touched_inode};
use crate::protocol::{MrDetails, MrFile, MrGuide, MrSummary, Pulled};
use crate::transcripts::native_entries;
use crate::views::FileChange;

/// Longest agent message quoted in a generated merge request summary.
const QUOTE_CHARS: usize = 600;

impl Core {
    /// The thread named by `selector`, or else the one checked out at `cwd`.
    pub(crate) fn thread_at(&self, selector: Option<&str>, cwd: &Path) -> Result<ThreadId, CoreError> {
        if let Some(selector) = selector {
            return self.find_thread(selector);
        }
        match self.threads_checked_out_at(&CanonicalCwd::new(cwd)?).as_slice() {
            [] => Err(CoreError::Invalid("no thread is checked out here; name one".into())),
            [thread] => Ok(*thread),
            several => {
                let named: Vec<String> = several.iter().map(|thread| thread.short().to_string()).collect();
                Err(CoreError::Invalid(format!("agents of threads {} work here; name one", named.join(", "))))
            }
        }
    }

    /// The thread of the innermost checkout holding `cwd`. In a clone
    /// several agents work in, the newest of their threads (ADR 0089).
    pub(crate) fn thread_checked_out_at(&self, cwd: &CanonicalCwd) -> Option<ThreadId> {
        self.threads_checked_out_at(cwd).last().copied()
    }

    /// The threads of the innermost checkouts holding `cwd`, oldest first:
    /// those whose agents work there, or else the one there.
    pub(crate) fn threads_checked_out_at(&self, cwd: &CanonicalCwd) -> Vec<ThreadId> {
        let Some(innermost) = self
            .checkouts
            .values()
            .filter(|checkout| cwd.starts_with(&checkout.record.path))
            .max_by_key(|checkout| checkout.record.path.as_os_str().len())
        else {
            return Vec::new();
        };
        let rows: Vec<&crate::core::Checkout> =
            self.checkouts.values().filter(|checkout| checkout.record.path == innermost.record.path).collect();
        let at_work: Vec<ThreadId> = rows
            .iter()
            .filter(|checkout| self.agent_at_work_anywhere(checkout.record.thread))
            .map(|checkout| checkout.record.thread)
            .collect();
        match at_work.as_slice() {
            [] => rows.last().map(|checkout| checkout.record.thread).into_iter().collect(),
            _ => at_work,
        }
    }

    fn loaded_family(&self, thread: ThreadId) -> Result<(FamilyId, &Family), CoreError> {
        let family_id = self.threads[&thread].family;
        let family = self.families.get(&family_id).ok_or_else(|| {
            CoreError::Invalid(format!("thread {} isn't on this machine yet; resume or fork it first", thread.short()))
        })?;
        Ok((family_id, family))
    }

    /// Fails unless the thread is this user's and this device holds its
    /// lease, or nobody does.
    pub(crate) fn ensure_writer(&self, thread: ThreadId) -> Result<(), CoreError> {
        let owner = &self.threads[&thread].owner;
        let user = self.own_user(thread);
        if !owner.is_empty() && !user.is_empty() && *owner != user {
            return Err(CoreError::Invalid(format!(
                "thread {} belongs to {owner}; fork it to continue the work",
                thread.short()
            )));
        }
        let (_, family) = self.loaded_family(thread)?;
        let lease = family.thread(thread).and_then(ThreadState::lease);
        let device = self.device_for(thread);
        match (lease, device) {
            (Some((holder, _)), Some(device)) if holder != device => Err(CoreError::Invalid(format!(
                "thread {} is being worked on from another device; run this there, or fork the thread",
                thread.short()
            ))),
            _ => Ok(()),
        }
    }

    /// Appends a delta by this daemon on `thread` with the given parents.
    pub(crate) async fn append_op(
        &mut self,
        family_id: FamilyId,
        thread: ThreadId,
        parents: Frontier,
        op: Op,
    ) -> Result<Delta, CoreError> {
        let actor = self.thread_actor(thread);
        let family = self.families.get_mut(&family_id).expect("caller loaded the family");
        let delta = family.delta(actor, thread, Author::User, crate::core::now_ms(), parents, op);
        family.apply(delta.clone(), &mut HashMap::new())?;
        self.persist(family_id, std::slice::from_ref(&delta)).await?;
        Ok(delta)
    }

    /// Writes what changed between two versions of the family into the
    /// checkouts whose threads now see it.
    async fn bring_checkouts_to(
        &mut self,
        family_id: FamilyId,
        before: &VersionVector,
        after: &VersionVector,
    ) -> Result<(), CoreError> {
        let changed: Vec<Delta> = self.families[&family_id]
            .log()
            .iter()
            .filter(|delta| after.contains(delta.span.first()) && !before.contains(delta.span.first()))
            .filter(|delta| touched_inode(delta).is_some())
            .cloned()
            .collect();
        self.update_checkouts(family_id, &changed).await
    }

    /// Merges the source's newer code and conversation into a fork.
    pub(crate) async fn pull(&mut self, selector: Option<&str>, cwd: &Path) -> Result<Pulled, CoreError> {
        let thread = self.thread_at(selector, cwd)?;
        self.ensure_writer(thread)?;
        let (family_id, family) = self.loaded_family(thread)?;
        let source = family
            .thread(thread)
            .and_then(ThreadState::forked_from)
            .ok_or_else(|| CoreError::Invalid(format!("thread {} is not a fork; there is nothing to pull", thread.short())))?;
        let head = family.thread_head(thread);
        let source_head = family.thread_head(source);
        let mut pulled = Pulled { thread: thread.to_string(), from: source.to_string(), messages: 0, files: Vec::new() };
        if source_head.ids().iter().all(|id| family.graph().is_ancestor(*id, &head)) {
            return Ok(pulled);
        }
        let before = family.graph().version_vector(&head);
        let known: HashSet<DeltaId> = family.history(thread).iter().map(|item| item.delta.span.first()).collect();
        let parents: Frontier = head.ids().iter().chain(source_head.ids()).copied().collect();
        self.append_op(family_id, thread, parents, Op::Merge { from: source, mr: None }).await?;

        let family = &self.families[&family_id];
        let after = family.graph().version_vector(&family.thread_head(thread));
        let mut transcripts = Transcripts::default();
        pulled.messages = family
            .history(thread)
            .iter()
            .filter(|item| item.source == HistorySource::Upstream && !known.contains(&item.delta.span.first()))
            .flat_map(|item| native_entries(&mut transcripts, item.delta))
            .filter(|entry| matches!(entry, Entry::User { .. } | Entry::Assistant { .. }))
            .count();
        pulled.files = self.changes(family_id, &before, &after).iter().map(|change| change.path().to_owned()).collect();
        self.bring_checkouts_to(family_id, &before, &after).await?;
        self.notify(
            thread,
            format!(
                "Silt pulled newer work from thread {source} (this thread is a fork of it): {} new messages{}. \
                 Run `silt show {source}` to read its conversation.",
                pulled.messages,
                files_note(&pulled.files)
            ),
        )
        .await;
        Ok(pulled)
    }

    /// Offers a fork's work back to its source.
    pub(crate) async fn mr_create(
        &mut self,
        selector: Option<&str>,
        cwd: &Path,
        title: Option<String>,
        summary: Option<String>,
    ) -> Result<MrSummary, CoreError> {
        let thread = self.thread_at(selector, cwd)?;
        self.ensure_writer(thread)?;
        let (family_id, family) = self.loaded_family(thread)?;
        let target = family.thread(thread).and_then(ThreadState::forked_from).ok_or_else(|| {
            CoreError::Invalid(format!("thread {} is not a fork; merge requests go from a fork to its source", thread.short()))
        })?;
        let title = title
            .or_else(|| family.thread(thread).and_then(ThreadState::title).map(str::to_owned))
            .unwrap_or_else(|| format!("Changes from fork {}", thread.short()));
        let head = family.thread_head(thread);
        let summary = match summary {
            Some(summary) => summary,
            None => {
                let graph = family.graph();
                let base = graph.version_vector(&head).intersection(&graph.version_vector(&family.thread_head(target)));
                let files: Vec<String> = self
                    .changes(family_id, &base, &graph.version_vector(&head))
                    .iter()
                    .map(|change| change.path().to_owned())
                    .collect();
                generated_summary(family, thread, &files)
            }
        };
        let delta = self.append_op(family_id, thread, head, Op::Mr(MrOp::Opened { target, title, summary })).await?;
        let request = self.families[&family_id]
            .merge_requests()
            .into_iter()
            .find(|request| request.id == delta.span.first())
            .expect("just opened");
        Ok(mr_summary(&self.families[&family_id], &request))
    }

    /// Merge requests involving `selector`'s thread, or all of them.
    pub(crate) fn mr_list(&self, selector: Option<&str>) -> Result<Vec<MrSummary>, CoreError> {
        let thread = selector.map(|selector| self.find_thread(selector)).transpose()?;
        let families: Vec<FamilyId> = self.families.keys().copied().collect();
        let mut requests: Vec<(MergeRequest, MrSummary)> = families
            .into_iter()
            .flat_map(|family| self.requests_in(family))
            .filter(|(request, _)| thread.is_none_or(|thread| request.fork == thread || request.target == thread))
            .collect();
        requests.sort_by_key(|(request, _)| request.opened_ms);
        Ok(requests.into_iter().map(|(_, summary)| summary).collect())
    }

    /// The family's merge requests with their summaries: from the family if
    /// held, else from its digest.
    pub(crate) fn requests_in(&self, family_id: FamilyId) -> Vec<(MergeRequest, MrSummary)> {
        if let Some(digest) = self.digested(family_id) {
            return digest.requests.clone();
        }
        let Some(family) = self.viewed(family_id) else {
            return Vec::new();
        };
        family
            .merge_requests()
            .into_iter()
            .map(|request| {
                let summary = mr_summary(family, &request);
                (request, summary)
            })
            .collect()
    }

    pub(crate) fn find_mr(&self, selector: &str) -> Result<(FamilyId, MergeRequest), CoreError> {
        let families: Vec<FamilyId> = self.families.keys().copied().collect();
        let matches: Vec<(FamilyId, MergeRequest)> = families
            .into_iter()
            .flat_map(|family| self.requests_in(family).into_iter().map(move |(request, _)| (family, request)))
            .filter(|(_, request)| short_mr(request.id) == selector || request.id.to_string() == selector)
            .collect();
        match <[_; 1]>::try_from(matches) {
            Ok([found]) => Ok(found),
            Err(matches) if matches.is_empty() => Err(CoreError::Invalid(format!("no merge request {selector}"))),
            Err(_) => Err(CoreError::Invalid(format!("{selector} matches several merge requests"))),
        }
    }

    pub(crate) fn mr_show(&self, selector: &str, hide: silt_log::Hide) -> Result<MrDetails, CoreError> {
        let (family_id, request) = self.find_mr(selector)?;
        let family = &self.families[&family_id];
        let graph = family.graph();
        let offered = graph.version_vector(&Frontier::single(request.id));
        // A merged request shows what it brought, against the target as it was before.
        let target =
            graph.version_vector(&target_before_merge(family, &request).unwrap_or_else(|| family.thread_head(request.target)));
        let base = offered.intersection(&target);
        let ours = self.changes(family_id, &base, &target);
        let theirs = self.changes(family_id, &base, &offered);
        let conflicts = conflicts(&ours, &theirs);
        let mr = request.id.to_string();
        // Seen marks keep to the files as they are, git's edits and all.
        let shown = if hide.any() { self.changes_hiding(family_id, &base, &offered, hide) } else { theirs.clone() };
        let shown_paths: HashSet<&str> = shown.iter().map(FileChange::path).collect();
        let files = file_hashes(&theirs)
            .into_iter()
            .filter(|(path, _)| shown_paths.contains(path.as_str()))
            .map(|(path, hash)| MrFile { seen: self.personal.has_seen(&mr, &path, &hash), path })
            .collect();
        let conversation = self.show_items(
            family,
            request.fork,
            family
                .history(request.fork)
                .into_iter()
                .filter(|item| item.source != HistorySource::Inherited && offered.contains(item.delta.span.first()))
                .collect(),
        );
        Ok(MrDetails {
            mr: mr_summary(family, &request),
            diff: crate::views::render_diff(&shown),
            conflicts,
            conversation,
            files,
            guide: request.guide.clone().map(|(text, by)| MrGuide { text, by }),
        })
    }

    /// The files the request changes, each with the hash of its text in it.
    pub(crate) fn request_files(&self, family_id: FamilyId, request: &MergeRequest) -> Vec<(String, String)> {
        let family = &self.families[&family_id];
        let graph = family.graph();
        let offered = graph.version_vector(&Frontier::single(request.id));
        let target = graph.version_vector(&family.thread_head(request.target));
        file_hashes(&self.changes(family_id, &offered.intersection(&target), &offered))
    }

    /// Merges a merge request into its target on the device that holds the
    /// target, and tells the target's agent.
    pub(crate) async fn mr_approve(&mut self, selector: &str) -> Result<MrSummary, CoreError> {
        let (family_id, request) = self.open_request(selector)?;
        let family = &self.families[&family_id];
        let head = family.thread_head(request.target);
        let before = family.graph().version_vector(&head);
        let parents: Frontier = head.ids().iter().copied().chain([request.id]).collect();
        let merge = Op::Merge { from: request.fork, mr: Some(request.id) };
        self.append_op(family_id, request.target, parents, merge).await?;
        let head = self.families[&family_id].thread_head(request.target);
        self.append_op(family_id, request.target, head, Op::Mr(MrOp::Approved { mr: request.id })).await?;
        let merged =
            ConvOp::Merged { from: request.fork, mr: request.id, title: request.title.clone(), summary: request.summary.clone() };
        self.append_conv(family_id, request.target, Author::User, merged).await?;

        let family = &self.families[&family_id];
        let after = family.graph().version_vector(&family.thread_head(request.target));
        let files: Vec<String> = self.changes(family_id, &before, &after).iter().map(|change| change.path().to_owned()).collect();
        self.bring_checkouts_to(family_id, &before, &after).await?;
        self.notify(
            request.target,
            format!(
                "Silt merged the merge request \"{}\" from fork {} into this thread{}. Its summary:\n{}\n\
                 The fork's full conversation stays readable with `silt show {}`.",
                request.title,
                request.fork,
                files_note(&files),
                request.summary,
                request.fork
            ),
        )
        .await;
        Ok(self.summary_of(family_id, request.id))
    }

    pub(crate) async fn mr_decline(&mut self, selector: &str, reason: Option<String>) -> Result<MrSummary, CoreError> {
        let (family_id, request) = self.open_request(selector)?;
        let head = self.families[&family_id].thread_head(request.target);
        let declined = MrOp::Declined { mr: request.id, reason: reason.unwrap_or_default() };
        self.append_op(family_id, request.target, head, Op::Mr(declined)).await?;
        Ok(self.summary_of(family_id, request.id))
    }

    /// Keeps the request open and asks its fork for changes; its agent, if
    /// running here, hears the reason with its next prompt (ADR 0025).
    pub(crate) async fn mr_request_changes(&mut self, selector: &str, reason: String) -> Result<MrSummary, CoreError> {
        if reason.trim().is_empty() {
            return Err(CoreError::Invalid("say what should change".into()));
        }
        let (family_id, request) = self.open_request(selector)?;
        let head = self.families[&family_id].thread_head(request.target);
        let asked = MrOp::ChangesRequested { mr: request.id, reason: reason.clone() };
        self.append_op(family_id, request.target, head, Op::Mr(asked)).await?;
        self.notify(request.fork, changes_requested_notice(&request.title, &reason)).await;
        Ok(self.summary_of(family_id, request.id))
    }

    /// Records `guide` as the request's change guide, on its fork.
    pub(crate) async fn keep_guide(
        &mut self,
        family_id: FamilyId,
        request: &MergeRequest,
        guide: String,
        by: String,
    ) -> Result<(), CoreError> {
        let head = self.families[&family_id].thread_head(request.fork);
        self.append_op(family_id, request.fork, head, Op::Mr(MrOp::Guided { mr: request.id, guide, by })).await?;
        self.announce(crate::protocol::BusEvent::Mrs);
        Ok(())
    }

    fn summary_of(&self, family_id: FamilyId, mr: silt_types::DeltaId) -> MrSummary {
        let family = &self.families[&family_id];
        let request = family.merge_requests().into_iter().find(|found| found.id == mr).expect("still there");
        mr_summary(family, &request)
    }

    /// An open merge request whose target this device may write to.
    fn open_request(&self, selector: &str) -> Result<(FamilyId, MergeRequest), CoreError> {
        let (family_id, request) = self.find_mr(selector)?;
        if request.status != MrStatus::Open {
            return Err(CoreError::Invalid(format!(
                "merge request {} is already {}",
                short_mr(request.id),
                status_text(&request.status)
            )));
        }
        self.ensure_writer(request.target)?;
        Ok((family_id, request))
    }

    pub(crate) async fn set_visibility(
        &mut self,
        cwd: &Path,
        selector: Option<&str>,
        visibility: Visibility,
    ) -> Result<String, CoreError> {
        let refused = |source| CoreError::Server { doing: "change the visibility", source };
        match selector {
            Some(selector) => {
                let thread = self.find_thread(selector)?;
                let remote = self.server_for(thread)?;
                remote.set_visibility(None, Some(thread), visibility.clone()).await.map_err(refused)?;
                // Shown at once, before the next listing says so.
                self.visibility.insert(thread, visibility.clone());
                self.announce(crate::protocol::BusEvent::Threads);
                Ok(format!("thread {} is now {visibility}; forks without a setting of their own follow it", thread.short(),))
            }
            None => {
                let repo = silt_git::Repo::discover(&CanonicalCwd::new(cwd)?)?;
                let identity = repo
                    .remote_url("origin")?
                    .map(|url| silt_git::normalize_remote(&url))
                    .ok_or_else(|| CoreError::Invalid("this clone has no origin remote".into()))?;
                let remote = self.server_for_project(&identity)?;
                remote.set_visibility(Some(identity.clone()), None, visibility.clone()).await.map_err(refused)?;
                Ok(format!("project {identity} is now {visibility}"))
            }
        }
    }

    /// Keeps a message for the agent working in `thread`, handed over on its
    /// next prompt. One that can't be kept is only logged: what it tells of
    /// has happened either way.
    pub(crate) async fn notify(&self, thread: ThreadId, message: String) {
        if let Err(error) = self.store.add_notice(thread, &message, now_ms()).await {
            warn!(%error, %thread, "could not keep a message for the agent");
        }
    }

    /// Takes the messages waiting for `thread`'s agent.
    pub(crate) async fn take_notices(&self, thread: ThreadId) -> Option<String> {
        match self.store.take_notices(thread).await {
            Ok(notices) => (!notices.is_empty()).then(|| notices.join("\n\n")),
            Err(error) => {
                warn!(%error, %thread, "could not read the messages for the agent");
                None
            }
        }
    }
}

/// The target's head just before it merged `request`, for a merged one: its
/// merge's parents are that head and the request's point.
fn target_before_merge(family: &Family, request: &MergeRequest) -> Option<Frontier> {
    let merge = family
        .log()
        .iter()
        .find(|delta| delta.thread == request.target && matches!(delta.op, Op::Merge { mr: Some(mr), .. } if mr == request.id))?;
    let before: Frontier = merge.parents.ids().iter().copied().filter(|parent| *parent != request.id).collect();
    (!before.is_empty()).then_some(before)
}

/// The files and the fork's last agent message, for a merge request whose
/// author gave no summary.
fn generated_summary(family: &Family, thread: ThreadId, files: &[String]) -> String {
    // As code, so a path such as `calc/__init__.py` isn't read as Markdown's bold.
    let named: Vec<String> = files.iter().map(|file| format!("`{file}`")).collect();
    let mut summary = match &named[..] {
        [] => "No file changes.".to_owned(),
        [file] => format!("Changes {file}."),
        files => format!("Changes {} files: {}.", files.len(), files.join(", ")),
    };
    let mut transcripts = Transcripts::default();
    let own: Vec<Entry> = family
        .history(thread)
        .iter()
        .filter(|item| item.source == HistorySource::Own)
        .flat_map(|item| native_entries(&mut transcripts, item.delta))
        .collect();
    // The fork's prompts stay out: the summary goes into the target's history with the merge, and the
    // fork's conversation stays the fork's.
    let last_message = own.iter().rev().find_map(|entry| match entry {
        Entry::Assistant { text } if !text.trim().is_empty() => Some(text.trim()),
        _ => None,
    });
    // Set apart as a quote, so the agent's words don't read as the request's own.
    if let Some(message) = last_message {
        summary.push_str("\n\nWhere the fork's agent left it:\n\n");
        for line in quoted(message, QUOTE_CHARS).lines() {
            summary.push_str(&format!("> {line}\n"));
        }
    }
    summary.trim_end().to_owned()
}

/// `text`, or as much of it as fits in `limit` characters, cut at a line,
/// else at a word, and saying so: a cut in a word, or in a Markdown list,
/// reads as a mistake.
fn quoted(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let head: String = text.chars().take(limit).collect();
    let cut = head.rfind('\n').filter(|at| *at > limit / 2).or_else(|| head.rfind(char::is_whitespace)).unwrap_or(head.len());
    format!("{} …", head[..cut].trim_end())
}

fn files_note(files: &[String]) -> String {
    match files {
        [] => String::new(),
        files => format!("; files changed: {}", files.join(", ")),
    }
}

/// Where both sides changed the same lines of a file since the merge base.
fn conflicts(ours: &[FileChange], theirs: &[FileChange]) -> Vec<String> {
    let mut conflicts = Vec::new();
    for change in theirs {
        let Some(other) = ours.iter().find(|other| other.inode.is_some() && other.inode == change.inode) else {
            continue;
        };
        for (start, end) in overlapping_lines(&change.old_text, &other.new_text, &change.new_text) {
            conflicts.push(format!(
                "{}: both changed lines {}-{} of the merge base",
                change.path(),
                start + 1,
                end.max(start + 1)
            ));
        }
    }
    conflicts
}

/// Line ranges of `base` that both `ours` and `theirs` changed, as
/// half-open ranges.
fn overlapping_lines(base: &str, ours: &str, theirs: &str) -> Vec<(usize, usize)> {
    let changed = |other: &str| -> Vec<(usize, usize)> {
        TextDiff::from_lines(base, other)
            .ops()
            .iter()
            .filter(|op| op.tag() != DiffTag::Equal)
            .map(|op| (op.old_range().start, op.old_range().end))
            .collect()
    };
    let (ours, theirs) = (changed(ours), changed(theirs));
    let mut overlaps: Vec<(usize, usize)> = Vec::new();
    for (start, end) in &theirs {
        for (other_start, other_end) in &ours {
            // An insertion is an empty range; it overlaps what touches it.
            if start <= other_end && other_start <= end {
                overlaps.push(((*start).min(*other_start), (*end).max(*other_end)));
            }
        }
    }
    overlaps.sort_unstable();
    overlaps.dedup();
    overlaps
}

pub(crate) fn short_mr(id: DeltaId) -> String {
    let actor = id.actor.to_string();
    format!("{}.{}", &actor[actor.len() - 6..], id.lamport.0)
}

fn status_text(status: &MrStatus) -> String {
    match status {
        MrStatus::Open => "open".into(),
        MrStatus::Approved => "approved".into(),
        MrStatus::Declined { reason } if reason.is_empty() => "declined".into(),
        MrStatus::Declined { reason } => format!("declined: {reason}"),
    }
}

/// Each changed file's path and the BLAKE3 of its text in the change, or
/// of nothing for one the change deletes, so a seen mark goes stale when
/// the file differs (ADR 0024).
fn file_hashes(changes: &[FileChange]) -> Vec<(String, String)> {
    changes
        .iter()
        .map(|change| {
            let text = if change.new_path.is_some() { change.new_text.as_bytes() } else { b"" };
            (change.path().to_owned(), blake3::hash(text).to_hex().to_string())
        })
        .collect()
}

/// What the fork's agent is told when changes are asked of its request.
pub(crate) fn changes_requested_notice(title: &str, reason: &str) -> String {
    format!(
        "The reviewer asked for changes to the merge request \"{title}\" from this thread before taking it: {reason}\n\
         Make them here; the request stays open and shows that the fork moved on."
    )
}

fn mr_summary(family: &Family, request: &MergeRequest) -> MrSummary {
    let open = request.status == MrStatus::Open;
    let asked = request.changes_requested.as_ref().filter(|_| open);
    let fork_moved_ms = family.log().iter().filter(|delta| delta.thread == request.fork).map(|delta| delta.time_ms).max();
    MrSummary {
        id: short_mr(request.id),
        fork: request.fork.to_string(),
        target: request.target.to_string(),
        title: request.title.clone(),
        summary: request.summary.clone(),
        status: status_text(&request.status),
        opened_ms: request.opened_ms,
        decided_ms: request.decided_ms,
        changes_requested: asked.map(|asked| asked.reason.clone()),
        updated_since: asked.is_some_and(|asked| fork_moved_ms.is_some_and(|moved| moved > asked.at_ms)),
    }
}

#[cfg(test)]
mod tests {
    use silt_crdt::InodeId;

    use super::*;

    #[test]
    fn overlapping_edits_are_hinted_and_separate_ones_are_not() {
        let base = "a\nb\nc\nd\ne\n";
        assert_eq!(overlapping_lines(base, "a\nB\nc\nd\ne\n", "a\nb\nc\nd\nE\n"), vec![]);
        assert_eq!(overlapping_lines(base, "a\nB\nc\nd\ne\n", "a\nb2\nc\nd\ne\n"), vec![(1, 2)]);
        assert_eq!(overlapping_lines(base, "a\nb\nc\nd\ne\n", "x\n"), vec![]);
    }

    #[test]
    fn conflicts_name_the_file_and_lines_both_sides_changed() {
        let change = |actor: u128, path: &str, new_text: &str| FileChange {
            inode: Some(InodeId(DeltaId::new(silt_types::ActorId(actor), silt_types::Lamport(0)))),
            old_path: Some(path.to_owned()),
            new_path: Some(path.to_owned()),
            old_text: "a\nb\nc\n".to_owned(),
            new_text: new_text.to_owned(),
            binary: false,
        };
        let ours = [change(1, "x.rs", "a\nB\nc\n"), change(2, "y.rs", "a\nB\nc\n")];
        let theirs = [change(1, "x.rs", "a\nb2\nc\n"), change(3, "z.rs", "a\nb2\nc\n")];
        assert_eq!(conflicts(&ours, &theirs), ["x.rs: both changed lines 2-2 of the merge base"]);
    }

    #[test]
    fn a_file_is_hashed_as_the_request_leaves_it() {
        let change = |new_path: Option<&str>, new_text: &str| FileChange {
            inode: None,
            old_path: Some("x.rs".to_owned()),
            new_path: new_path.map(str::to_owned),
            old_text: "old\n".to_owned(),
            new_text: new_text.to_owned(),
            binary: false,
        };
        let edited = file_hashes(&[change(Some("x.rs"), "new\n")]);
        let edited_again = file_hashes(&[change(Some("x.rs"), "newer\n")]);
        let deleted = file_hashes(&[change(None, "")]);
        assert_eq!(edited[0].0, "x.rs");
        assert_ne!(edited[0].1, edited_again[0].1, "a changed file is unseen again");
        assert_eq!(deleted[0], ("x.rs".to_owned(), blake3::hash(b"").to_hex().to_string()));
    }

    #[test]
    fn a_long_quote_is_cut_between_words_and_says_so() {
        assert_eq!(quoted("short enough", 40), "short enough");
        assert_eq!(quoted("I also added divide to the import line", 30), "I also added divide to the …");
        assert_eq!(quoted("first line\nsecond line\nthird line", 25), "first line\nsecond line …");
    }
}
