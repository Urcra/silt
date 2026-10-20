//! Pulls, merge requests and visibility (ADR 0010). Forking itself shares
//! its checkout and session restore with resume, in `resume.rs`.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use silt_agents::{Entry, claude};
use silt_crdt::VersionVector;
use silt_log::{Author, ConvOp, Delta, Family, HistorySource, MergeRequest, MrOp, MrStatus, Op};
use silt_proto::Visibility;
use silt_types::{DeltaId, FamilyId, Frontier, ThreadId};
use similar::{DiffTag, TextDiff};

use crate::core::{Core, CoreError, touched_inode};
use crate::protocol::{MrDetails, MrSummary, Pulled};

/// Longest agent message quoted in a generated merge request summary.
const QUOTE_CHARS: usize = 600;

impl Core {
    /// The thread named by `selector`, or else the one checked out at `cwd`.
    pub(crate) fn thread_at(&self, selector: Option<&str>, cwd: &Path) -> Result<ThreadId, CoreError> {
        if let Some(selector) = selector {
            return self.find_thread(selector);
        }
        let cwd = cwd.canonicalize()?;
        self.checkouts
            .values()
            .filter(|checkout| cwd.starts_with(&checkout.record.path))
            .max_by_key(|checkout| checkout.record.path.as_os_str().len())
            .map(|checkout| checkout.record.thread)
            .ok_or_else(|| CoreError::Invalid("no thread is checked out here; name one".into()))
    }

    fn loaded_family(&self, thread: ThreadId) -> Result<(FamilyId, &Family), CoreError> {
        let family_id = self.threads[&thread].family;
        let family = self.families.get(&family_id).ok_or_else(|| {
            CoreError::Invalid(format!(
                "thread {} isn't on this machine yet; resume or fork it first",
                short(thread)
            ))
        })?;
        Ok((family_id, family))
    }

    /// Fails unless the thread is this user's and this device holds its
    /// lease, or nobody does.
    fn ensure_writer(&self, thread: ThreadId) -> Result<(), CoreError> {
        let owner = &self.threads[&thread].owner;
        let user = self.own_user();
        if !owner.is_empty() && !user.is_empty() && *owner != user {
            return Err(CoreError::Invalid(format!(
                "thread {} belongs to {owner}; fork it to continue the work",
                short(thread)
            )));
        }
        let (_, family) = self.loaded_family(thread)?;
        let lease = family.thread(thread).and_then(|state| state.lease);
        let device = self.remote.as_ref().map(|remote| remote.config.device);
        match (lease, device) {
            (Some((holder, _)), Some(device)) if holder != device => Err(CoreError::Invalid(format!(
                "thread {} is being worked on from another device; run this there, or fork the thread",
                short(thread)
            ))),
            _ => Ok(()),
        }
    }

    /// Appends a delta by this daemon on `thread` with the given parents.
    async fn append_op(&mut self, family_id: FamilyId, thread: ThreadId, parents: Frontier, op: Op) -> Result<Delta, CoreError> {
        let actor = self.thread_actor(thread);
        let family = self.families.get_mut(&family_id).expect("caller loaded the family");
        let delta = family.delta(actor, thread, Author::User, crate::core::now_ms(), parents, op);
        family.apply(delta.clone(), &mut HashMap::new())?;
        self.store.append_deltas(family_id, std::slice::from_ref(&delta)).await?;
        Ok(delta)
    }

    /// Writes what changed between two versions of the family into the
    /// checkouts whose threads now see it.
    async fn bring_checkouts_to(&mut self, family_id: FamilyId, before: &VersionVector, after: &VersionVector) -> Result<(), CoreError> {
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
            .and_then(|state| state.forked_from)
            .ok_or_else(|| CoreError::Invalid(format!("thread {} is not a fork; there is nothing to pull", short(thread))))?;
        let head = family.thread_head(thread);
        let source_head = family.thread_head(source);
        let mut pulled = Pulled {
            thread: thread.to_string(),
            from: source.to_string(),
            messages: 0,
            files: Vec::new(),
        };
        if source_head.ids().iter().all(|id| family.graph().is_ancestor(*id, &head)) {
            return Ok(pulled);
        }
        let before = family.graph().version_vector(&head);
        let known: HashSet<DeltaId> = family.history(thread).iter().map(|item| item.delta.span.first()).collect();
        let parents: Frontier = head.ids().iter().chain(source_head.ids()).copied().collect();
        self.append_op(family_id, thread, parents, Op::Merge { from: source, mr: None }).await?;

        let family = &self.families[&family_id];
        let after = family.graph().version_vector(&family.thread_head(thread));
        pulled.messages = family
            .history(thread)
            .iter()
            .filter(|item| item.source == HistorySource::Upstream && !known.contains(&item.delta.span.first()))
            .flat_map(|item| entries(item.delta))
            .filter(|entry| matches!(entry, Entry::User { .. } | Entry::Assistant { .. }))
            .count();
        pulled.files = self
            .changes(family_id, &before, &after)
            .iter()
            .map(|change| change.path().to_owned())
            .collect();
        self.bring_checkouts_to(family_id, &before, &after).await?;
        self.notify(
            thread,
            format!(
                "Silt pulled newer work from thread {source} (this thread is a fork of it): {} new messages{}. \
                 Run `silt show {source}` to read its conversation.",
                pulled.messages,
                files_note(&pulled.files)
            ),
        );
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
        let target = family.thread(thread).and_then(|state| state.forked_from).ok_or_else(|| {
            CoreError::Invalid(format!(
                "thread {} is not a fork; merge requests go from a fork to its source",
                short(thread)
            ))
        })?;
        let title = title
            .or_else(|| family.thread(thread).and_then(|state| state.title.clone()))
            .unwrap_or_else(|| format!("Changes from fork {}", short(thread)));
        let head = family.thread_head(thread);
        let summary = match summary {
            Some(summary) => summary,
            None => {
                let graph = family.graph();
                let base = graph
                    .version_vector(&head)
                    .intersection(&graph.version_vector(&family.thread_head(target)));
                let files: Vec<String> = self
                    .changes(family_id, &base, &graph.version_vector(&head))
                    .iter()
                    .map(|change| change.path().to_owned())
                    .collect();
                generated_summary(family, thread, &files)
            }
        };
        let delta = self
            .append_op(family_id, thread, head, Op::Mr(MrOp::Opened { target, title, summary }))
            .await?;
        let request = self.families[&family_id]
            .merge_requests()
            .into_iter()
            .find(|request| request.id == delta.span.first())
            .expect("just opened");
        Ok(mr_summary(&request))
    }

    /// Merge requests involving `selector`'s thread, or all of them.
    pub(crate) fn mr_list(&self, selector: Option<&str>) -> Result<Vec<MrSummary>, CoreError> {
        let thread = selector.map(|selector| self.find_thread(selector)).transpose()?;
        let mut requests: Vec<MergeRequest> = self
            .families
            .values()
            .flat_map(Family::merge_requests)
            .filter(|request| thread.is_none_or(|thread| request.fork == thread || request.target == thread))
            .collect();
        requests.sort_by_key(|request| request.opened_ms);
        Ok(requests.iter().map(mr_summary).collect())
    }

    fn find_mr(&self, selector: &str) -> Result<(FamilyId, MergeRequest), CoreError> {
        let matches: Vec<(FamilyId, MergeRequest)> = self
            .families
            .iter()
            .flat_map(|(id, family)| family.merge_requests().into_iter().map(|request| (*id, request)))
            .filter(|(_, request)| short_mr(request.id) == selector || request.id.to_string() == selector)
            .collect();
        match <[_; 1]>::try_from(matches) {
            Ok([found]) => Ok(found),
            Err(matches) if matches.is_empty() => Err(CoreError::Invalid(format!("no merge request {selector}"))),
            Err(_) => Err(CoreError::Invalid(format!("{selector} matches several merge requests"))),
        }
    }

    pub(crate) fn mr_show(&self, selector: &str) -> Result<MrDetails, CoreError> {
        let (family_id, request) = self.find_mr(selector)?;
        let family = &self.families[&family_id];
        let graph = family.graph();
        let offered = graph.version_vector(&Frontier::single(request.id));
        let target = graph.version_vector(&family.thread_head(request.target));
        let base = offered.intersection(&target);
        let ours = self.changes(family_id, &base, &target);
        let theirs = self.changes(family_id, &base, &offered);
        let mut conflicts = Vec::new();
        for change in &theirs {
            let Some(other) = ours.iter().find(|other| other.inode == change.inode) else {
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
            mr: mr_summary(&request),
            diff: crate::views::render_diff(&theirs),
            conflicts,
            conversation,
        })
    }

    /// Merges a merge request into its target on the device that holds the
    /// target, and tells the target's agent.
    pub(crate) async fn mr_approve(&mut self, selector: &str) -> Result<MrSummary, CoreError> {
        let (family_id, request) = self.open_request(selector)?;
        let family = &self.families[&family_id];
        let head = family.thread_head(request.target);
        let before = family.graph().version_vector(&head);
        let parents: Frontier = head.ids().iter().copied().chain([request.id]).collect();
        let merge = Op::Merge {
            from: request.fork,
            mr: Some(request.id),
        };
        self.append_op(family_id, request.target, parents, merge).await?;
        let head = self.families[&family_id].thread_head(request.target);
        self.append_op(family_id, request.target, head, Op::Mr(MrOp::Approved { mr: request.id }))
            .await?;
        let merged = ConvOp::Merged {
            from: request.fork,
            mr: request.id,
            title: request.title.clone(),
            summary: request.summary.clone(),
        };
        self.append_conv(family_id, request.target, Author::User, merged).await?;

        let family = &self.families[&family_id];
        let after = family.graph().version_vector(&family.thread_head(request.target));
        let files: Vec<String> = self
            .changes(family_id, &before, &after)
            .iter()
            .map(|change| change.path().to_owned())
            .collect();
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
        );
        let request = self.families[&family_id]
            .merge_requests()
            .into_iter()
            .find(|found| found.id == request.id)
            .expect("still there");
        Ok(mr_summary(&request))
    }

    pub(crate) async fn mr_decline(&mut self, selector: &str, reason: Option<String>) -> Result<MrSummary, CoreError> {
        let (family_id, request) = self.open_request(selector)?;
        let head = self.families[&family_id].thread_head(request.target);
        let declined = MrOp::Declined {
            mr: request.id,
            reason: reason.unwrap_or_default(),
        };
        self.append_op(family_id, request.target, head, Op::Mr(declined)).await?;
        let request = self.families[&family_id]
            .merge_requests()
            .into_iter()
            .find(|found| found.id == request.id)
            .expect("still there");
        Ok(mr_summary(&request))
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
        let remote = self
            .remote
            .clone()
            .ok_or_else(|| CoreError::Invalid("not connected to a server; use `silt auth login`".into()))?;
        let refused = |error: crate::remote::RemoteError| CoreError::Invalid(format!("the server refused: {error}"));
        match selector {
            Some(selector) => {
                let thread = self.find_thread(selector)?;
                let family = self.threads[&thread].family;
                remote.set_visibility(None, Some(family), visibility).await.map_err(refused)?;
                Ok(format!("thread {} and its forks are now {visibility:?}", short(thread)).to_lowercase())
            }
            None => {
                let repo = silt_git::Repo::discover(&cwd.canonicalize()?)?;
                let identity = repo
                    .remote_url("origin")?
                    .map(|url| silt_git::normalize_remote(&url))
                    .ok_or_else(|| CoreError::Invalid("this clone has no origin remote".into()))?;
                remote
                    .set_visibility(Some(identity.clone()), None, visibility)
                    .await
                    .map_err(refused)?;
                Ok(format!("project {identity} is now {}", format!("{visibility:?}").to_lowercase()))
            }
        }
    }

    /// Queues a message for the agent working in `thread`, handed over on
    /// its next prompt.
    fn notify(&mut self, thread: ThreadId, message: String) {
        self.notices.entry(thread).or_default().push(message);
    }

    /// Takes the messages waiting for `thread`'s agent.
    pub(crate) fn take_notices(&mut self, thread: ThreadId) -> Option<String> {
        let notices = self.notices.remove(&thread)?;
        Some(notices.join("\n\n"))
    }
}

fn entries(delta: &Delta) -> Vec<Entry> {
    match &delta.op {
        Op::Conv(ConvOp::Native { line, .. }) => claude::normalize(line),
        _ => Vec::new(),
    }
}

/// The files and the fork's last agent message, for a merge request whose
/// author gave no summary.
fn generated_summary(family: &Family, thread: ThreadId, files: &[String]) -> String {
    let mut summary = match files {
        [] => "No file changes.".to_owned(),
        files => format!("Changes {} files: {}.", files.len(), files.join(", ")),
    };
    let last_message = family
        .history(thread)
        .iter()
        .rev()
        .filter(|item| item.source == HistorySource::Own)
        .flat_map(|item| entries(item.delta).into_iter().rev())
        .find_map(|entry| match entry {
            Entry::Assistant { text } if !text.trim().is_empty() => Some(text),
            _ => None,
        });
    if let Some(message) = last_message {
        let quoted: String = message.trim().chars().take(QUOTE_CHARS).collect();
        summary.push_str("\nThe fork's agent last said: ");
        summary.push_str(&quoted);
    }
    summary
}

fn files_note(files: &[String]) -> String {
    match files {
        [] => String::new(),
        files => format!("; files changed: {}", files.join(", ")),
    }
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

fn short(thread: ThreadId) -> String {
    let id = thread.to_string();
    id[id.len().saturating_sub(12)..].to_owned()
}

fn status_text(status: &MrStatus) -> String {
    match status {
        MrStatus::Open => "open".into(),
        MrStatus::Approved => "approved".into(),
        MrStatus::Declined { reason } if reason.is_empty() => "declined".into(),
        MrStatus::Declined { reason } => format!("declined: {reason}"),
    }
}

fn mr_summary(request: &MergeRequest) -> MrSummary {
    MrSummary {
        id: short_mr(request.id),
        fork: request.fork.to_string(),
        target: request.target.to_string(),
        title: request.title.clone(),
        summary: request.summary.clone(),
        status: status_text(&request.status),
        opened_ms: request.opened_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_edits_are_hinted_and_separate_ones_are_not() {
        let base = "a\nb\nc\nd\ne\n";
        assert_eq!(overlapping_lines(base, "a\nB\nc\nd\ne\n", "a\nb\nc\nd\nE\n"), vec![]);
        assert_eq!(overlapping_lines(base, "a\nB\nc\nd\ne\n", "a\nb2\nc\nd\ne\n"), vec![(1, 2)]);
        assert_eq!(overlapping_lines(base, "a\nb\nc\nd\ne\n", "x\n"), vec![]);
    }
}
