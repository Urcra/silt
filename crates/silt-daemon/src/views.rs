use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use silt_agents::{Entry, Transcripts};
use silt_crdt::{InodeId, InodeKind, VersionVector};
use silt_log::{
    Author, BaseText, ConvOp, Delta, Family, FileView, GitOp, Hidden, Hide, HistoryItem, HistorySource, Op, TurnMark,
};
use silt_store::ThreadRecord;
use silt_types::{DeltaId, FamilyId, Frontier, GitOid, ThreadId};
use similar::TextDiff;

use crate::core::{Core, CoreError, RepoBases, touched_inode};
use silt_proto::view::{DiffStat, FileAt, SharedEdits};

use crate::homes::Home;

use crate::protocol::{FileEdit, FilePage, SearchFilters, SearchResult, ShowLine, ThreadSummary};
use crate::transcripts::native_entries;

/// A family's log as folded for the views across families: each thread's
/// summary so far and the merge requests' deltas. A thread's summary read
/// every line of its conversation again each time its log grew; now only
/// the lines added are read (ADR 0117).
#[derive(Default)]
pub(crate) struct FamilyFold {
    version: usize,
    pub(crate) threads: HashMap<ThreadId, ThreadFold>,
    pub(crate) requests: Vec<Delta>,
}

impl FamilyFold {
    fn push(&mut self, delta: &Delta) {
        if let Op::Mr(_) = delta.op {
            self.requests.push(delta.clone());
        }
        self.threads.entry(delta.thread).or_default().push(delta);
    }
}

/// What a thread's summary reads from its own deltas, folded in order.
#[derive(Default, Clone)]
pub(crate) struct ThreadFold {
    transcripts: std::sync::Arc<std::sync::Mutex<Transcripts>>,
    named: Option<String>,
    first_prompt: Option<String>,
    messages: usize,
    /// The agent of the last line its sessions recorded.
    agent: Option<String>,
    /// The name an ACP session's latest `session` record gives its agent.
    acp_name: Option<String>,
    edits: usize,
    shared: bool,
    forked_from: Option<ThreadId>,
    lease: Option<silt_types::DeviceId>,
    last_ms: Option<i64>,
    cloud: Option<String>,
    /// When the thread last recorded anything.
    pub(crate) moved_ms: Option<i64>,
}

impl ThreadFold {
    fn push(&mut self, delta: &Delta) {
        self.moved_ms = self.moved_ms.max(Some(delta.time_ms));
        if delta.op.is_activity() {
            self.last_ms = self.last_ms.max(Some(delta.time_ms));
        }
        self.shared |= matches!(delta.author, Author::Others { .. });
        match &delta.op {
            Op::Text { .. } => self.edits += 1,
            Op::Fork { from } => {
                self.forked_from.get_or_insert(*from);
            }
            Op::Lease { device, .. } => self.lease = Some(*device),
            Op::Conv(conv) => self.push_conversation(delta, conv),
            _ => {}
        }
    }

    fn push_conversation(&mut self, delta: &Delta, conv: &ConvOp) {
        match conv {
            ConvOp::Title(title) => self.named = Some(title.clone()),
            ConvOp::CloudSession { id, .. } => {
                self.cloud.get_or_insert_with(|| id.to_string());
            }
            ConvOp::Native { session, line, .. } => {
                self.agent = Some(session.agent.clone());
                if session.agent == silt_agents::acp::AGENT
                    && let Some(name) = silt_agents::acp::name_of(line)
                {
                    self.acp_name = Some(name);
                }
                if let Some(title) = ai_title(line) {
                    self.named = Some(title);
                }
                let mut transcripts = self.transcripts.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                for entry in native_entries(&mut transcripts, delta) {
                    match entry {
                        Entry::User { text } => {
                            self.messages += 1;
                            if self.first_prompt.is_none() {
                                self.first_prompt =
                                    text.lines().find(|line| !line.trim().is_empty()).map(|line| clip_title(line.trim()));
                            }
                        }
                        Entry::Assistant { .. } => self.messages += 1,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

/// What a thread's summary reads from its log, kept until the family's log
/// grows: parsing every line of every conversation is the listing's cost.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct LoggedSummary {
    title: Option<String>,
    agent: Option<String>,
    messages: usize,
    edits: usize,
    shared: bool,
    forked_from: Option<String>,
    lease: Option<silt_types::DeviceId>,
    last_ms: Option<i64>,
    cloud: Option<String>,
}

impl Core {
    /// Every thread here but those in the trash.
    pub(crate) fn thread_summaries(&self) -> Vec<ThreadSummary> {
        self.summaries(|thread| !self.deleted.contains(&thread))
    }

    pub(crate) fn summaries(&self, include: impl Fn(ThreadId) -> bool) -> Vec<ThreadSummary> {
        let records: Vec<&ThreadRecord> = self.thread_records().values().filter(|record| include(record.id)).collect();
        let logged = self.logged_summaries(&records);
        let checked_out: HashSet<ThreadId> = self.checkouts.values().map(|checkout| checkout.record.thread).collect();
        let mrs = self.mr_list(None).unwrap_or_default();
        let merged: HashSet<&str> = mrs.iter().filter(|mr| mr.status == "approved").map(|mr| mr.fork.as_str()).collect();
        let reviewing: HashSet<&str> = mrs
            .iter()
            .filter(|mr| mr.status == "open")
            .filter(|mr| {
                self.find_thread(&mr.target)
                    .ok()
                    .and_then(|target| self.threads.get(&target))
                    .is_some_and(|target| target.owner.is_empty() || target.owner == self.own_user(target.id))
            })
            .map(|mr| mr.target.as_str())
            .collect();
        let mut summaries: Vec<ThreadSummary> = records
            .into_iter()
            .map(|record| {
                let logged = logged[&record.id].clone();
                let lease = logged.lease;
                let checked_out = checked_out.contains(&record.id);
                let diverged = self.syncs.get(&record.family).is_some_and(|sync| sync.diverged().is_some());
                let location = if diverged {
                    "diverged"
                } else if lease.is_some_and(|holder| Some(holder) != self.device_for(record.id)) {
                    "moved"
                } else if checked_out {
                    "here"
                } else {
                    "remote"
                };
                ThreadSummary {
                    id: record.id.to_string(),
                    repo: if record.repo.is_empty() { record.repo_root.display().to_string() } else { record.repo.clone() },
                    title: logged.title,
                    created_ms: record.created_ms,
                    agent: logged.agent,
                    messages: logged.messages,
                    edits: logged.edits,
                    shared: logged.shared,
                    location: location.into(),
                    forked_from: logged.forked_from,
                    owner: record.owner.clone(),
                    collaborators: self.collaborators.get(&record.id).cloned().unwrap_or_default(),
                    writable: self.may_write(record.id),
                    agent_state: self.shown_state(record.id),
                    background: self.shown_background(record.id),
                    last_ms: logged.last_ms.unwrap_or(record.created_ms),
                    merged: merged.contains(record.id.to_string().as_str()),
                    review: reviewing.contains(record.id.to_string().as_str()),
                    mine: self.personal.mine(record.id),
                    visibility: self.visibility.get(&record.id).cloned(),
                    server: self.remote_of(record.family).map(|remote| remote.config.url.clone()),
                    local_only: self.homes.get(&record.family) == Some(&crate::homes::Home::Local),
                    workspace: Some(self.family_workspace(record.family).id.to_string()),
                    cloud: logged.cloud,
                    machine: lease
                        .or_else(|| self.made_on(record.id))
                        .filter(|holder| matches!(location, "moved" | "remote") && Some(*holder) != self.device_for(record.id))
                        .and_then(|holder| self.machine_name(holder))
                        .map(str::to_owned),
                }
            })
            .collect();
        summaries.sort_by_key(|summary| summary.created_ms);
        summaries
    }

    /// What each of `records`' summaries reads from its log: from its
    /// family's digest while it says what the family holds, else from what
    /// was folded of its log, the deltas added since folded in first.
    fn logged_summaries(&self, records: &[&ThreadRecord]) -> HashMap<ThreadId, LoggedSummary> {
        let mut logged = HashMap::new();
        let mut folding: BTreeSet<FamilyId> = BTreeSet::new();
        for record in records {
            match self.digested(record.family).and_then(|digest| digest.threads.get(&record.id)) {
                Some(digested) => {
                    logged.insert(record.id, digested.summary.clone());
                }
                None => {
                    folding.insert(record.family);
                }
            }
        }
        for family in folding {
            let records: Vec<&ThreadRecord> = records.iter().copied().filter(|record| record.family == family).collect();
            logged.extend(self.summaries_in(family, &records));
        }
        logged
    }

    /// The summaries of `records`, threads of `family`, read from its log.
    pub(crate) fn summaries_in(&self, family: FamilyId, records: &[&ThreadRecord]) -> HashMap<ThreadId, LoggedSummary> {
        let folds: Vec<(ThreadId, Option<ThreadFold>)> = self
            .folded(family, |folded| records.iter().map(|record| (record.id, folded.threads.get(&record.id).cloned())).collect());
        folds
            .into_iter()
            .map(|(thread, fold)| {
                let record = &self.thread_records()[&thread];
                (thread, self.summary_folded(record, &fold.unwrap_or_default()))
            })
            .collect()
    }

    /// `then` given the family's log folded: what was folded before, and
    /// the deltas added since (ADR 0117).
    pub(crate) fn folded<T>(&self, family: FamilyId, then: impl FnOnce(&FamilyFold) -> T) -> T {
        let mut folds = self.summary_folds.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let length = self.local_log_len(family);
        let fold = folds.entry(family).or_default();
        if fold.version > length {
            *fold = FamilyFold::default();
        }
        if fold.version < length {
            for delta in crate::sync::log_since_in(&self.families, family, fold.version) {
                fold.push(delta);
            }
            fold.version = length;
        }
        then(fold)
    }

    fn summary_folded(&self, record: &ThreadRecord, fold: &ThreadFold) -> LoggedSummary {
        let inherited = || {
            let family = self.families.get(&record.family)?;
            let inherited: Vec<&Delta> = family.history(record.id).into_iter().map(|item| item.delta).collect();
            title(&inherited)
        };
        LoggedSummary {
            // A fork says so until its own first prompt names it, so it isn't taken for its source.
            title: fold.named.clone().or_else(|| fold.first_prompt.clone()).or_else(|| match fold.forked_from {
                Some(from) => Some(format!("Fork of {}", inherited().unwrap_or_else(|| from.short()))),
                None => inherited(),
            }),
            agent: fold.agent.as_ref().map(|agent| {
                if agent == silt_agents::acp::AGENT {
                    fold.acp_name.clone().unwrap_or_else(|| agent.clone())
                } else {
                    silt_agents::agent(agent).map_or_else(|| agent.clone(), |known| known.command().to_owned())
                }
            }),
            messages: fold.messages,
            edits: fold.edits,
            shared: fold.shared,
            forked_from: fold.forked_from.map(|from| from.to_string()),
            lease: fold.lease,
            last_ms: fold.last_ms,
            cloud: fold.cloud.clone(),
        }
    }

    /// The thread's conversation across compactions, with the files each tool
    /// call changed listed after its result.
    pub(crate) fn show(&self, thread: ThreadId) -> Vec<ShowLine> {
        let family_id = self.thread_records()[&thread].family;
        if let Some(family) = self.families().get(&family_id) {
            return self.show_items(family, thread, family.history(thread));
        }
        let log = self.local_log(family_id);
        let rewound = silt_log::rewound_in(log, thread);
        let conversation: Vec<HistoryItem<'_>> = log
            .iter()
            .filter(|delta| delta.thread == thread && matches!(delta.op, Op::Conv(_)))
            .map(|delta| HistoryItem { delta, source: HistorySource::Own, rewound: rewound.contains(&delta.span.first()) })
            .collect();
        self.show_lines(None, thread, conversation)
    }

    /// Lines for `items` of `thread`'s history, with headings where the
    /// history moves between inherited, upstream and own parts.
    pub(crate) fn show_items(&self, family: &Family, thread: ThreadId, items: Vec<HistoryItem<'_>>) -> Vec<ShowLine> {
        self.show_lines(Some(family), thread, items)
    }

    fn show_lines(&self, family: Option<&Family>, thread: ThreadId, items: Vec<HistoryItem<'_>>) -> Vec<ShowLine> {
        let edits = family.map(|family| edits_by_tool(family, thread)).unwrap_or_default();
        let mut lines = Vec::new();
        let mut transcripts = Transcripts::default();
        let mut segment = (HistorySource::Own, thread);
        for item in items {
            let delta = item.delta;
            if (item.source, delta.thread) != segment && (item.source != HistorySource::Own || segment.0 != HistorySource::Own) {
                let label = match item.source {
                    HistorySource::Inherited => format!("from thread {} (before this fork)", delta.thread.short()),
                    HistorySource::Upstream => format!("upstream progress in thread {}", delta.thread.short()),
                    HistorySource::Own => format!("thread {}", thread.short()),
                };
                lines.push(ShowLine::Segment { label });
            }
            segment = (item.source, delta.thread);
            match &delta.op {
                Op::Merge { from, mr: None } => {
                    lines.push(ShowLine::Event { text: format!("pulled newer work from thread {}", from.short()) })
                }
                Op::Conv(ConvOp::Merged { from, title, summary, .. }) => lines.push(ShowLine::Event {
                    text: format!(
                        "merged \"{title}\" from fork {} (read it with `silt show {}`): {summary}",
                        from.short(),
                        from.short()
                    ),
                }),
                Op::Conv(ConvOp::Turn { mark, .. }) => {
                    if let TurnMark::PromptSubmitted | TurnMark::Stopped = mark {
                        lines.push(ShowLine::Turn { mark: format!("{mark:?}") });
                    }
                }
                Op::Conv(ConvOp::Native { .. }) => {
                    for entry in native_entries(&mut transcripts, delta) {
                        match entry {
                            Entry::ToolCall { id, name, input, .. } => {
                                lines.push(ShowLine::ToolCall { id, name, input: input.to_string() })
                            }
                            Entry::ToolResult { tool_use_id, text, .. } => {
                                lines.push(ShowLine::Entry { kind: "tool_result".into(), text });
                                if let Some(files) = edits.get(&(Some(tool_use_id.clone()), None)) {
                                    lines.push(ShowLine::Edits {
                                        tool_use_id: Some(tool_use_id),
                                        files: files.clone(),
                                        shared: None,
                                    });
                                }
                            }
                            Entry::Compaction { summary } => lines.push(ShowLine::Compaction { summary }),
                            Entry::Woke { text } => lines.push(ShowLine::Entry { kind: "wake".into(), text }),
                            Entry::Other { .. } => {}
                            other => lines.push(ShowLine::Entry {
                                kind: other.kind().to_owned(),
                                text: other.searchable_text().unwrap_or_default(),
                            }),
                        }
                    }
                }
                _ => {}
            }
        }
        for ((_, shared), files) in edits.iter().filter(|((tool, _), _)| tool.is_none()) {
            lines.push(ShowLine::Edits { tool_use_id: None, files: files.clone(), shared: shared.clone() });
        }
        lines
    }

    pub(crate) fn diff(
        &self,
        thread: ThreadId,
        turns: TurnRange,
        base: Option<&str>,
        only: Only<'_>,
        hide: Hide,
    ) -> Result<String, CoreError> {
        Ok(render_diff(&self.diff_changes(thread, turns, base, only, hide)?))
    }

    /// [`Self::diff`]'s files, each with its counts and how long its part of
    /// the diff is, so a page can ask for one file at a time.
    pub(crate) fn diff_stats(
        &self,
        thread: ThreadId,
        turns: TurnRange,
        base: Option<&str>,
        hide: Hide,
    ) -> Result<Vec<DiffStat>, CoreError> {
        Ok(self.diff_changes(thread, turns, base, Only::default(), hide)?.iter().map(diff_stat).collect())
    }

    /// The family a thread's diff reads, here.
    pub(crate) fn diff_family(&self, thread: ThreadId) -> Result<(FamilyId, &Family), CoreError> {
        let family_id = self.thread_records()[&thread].family;
        let family = self
            .families()
            .get(&family_id)
            .ok_or_else(|| CoreError::Invalid("the thread isn't on this machine yet; use `silt resume`".into()))?;
        Ok((family_id, family))
    }

    /// The files a diff of `thread` changes, without what `hide` leaves out:
    /// what git itself rewrote (ADR 0069), other agents' edits (ADR 0089).
    pub(crate) fn diff_changes(
        &self,
        thread: ThreadId,
        turns: TurnRange,
        base: Option<&str>,
        only: Only<'_>,
        hide: Hide,
    ) -> Result<Vec<FileChange>, CoreError> {
        let (family_id, family) = self.diff_family(thread)?;
        Ok(match diff_range(family, thread, turns, base)? {
            DiffRange::Git { head, rev } => self.diff_against_git(family_id, &head, rev, only)?,
            DiffRange::Versions { from, to } => {
                let hidden = Hidden::of(family.log(), hide);
                self.changes_in(family_id, &from, &to, only, hidden.as_ref())
            }
        })
    }

    /// How the files at `head` differ from a commit's tree in this machine's
    /// clone, such as its main branch as it is now.
    fn diff_against_git(
        &self,
        family_id: FamilyId,
        head: &VersionVector,
        rev: GitBase,
        only: Only<'_>,
    ) -> Result<Vec<FileChange>, CoreError> {
        let family = &self.families()[&family_id];
        let repos = self.family_repos(family_id);
        let names: &[&str] = match rev {
            GitBase::Main => &["origin/HEAD", "origin/main", "origin/master", "main", "master"],
            GitBase::Rev(rev) => &[rev],
        };
        let (repo, commit) = names
            .iter()
            .find_map(|name| repos.iter().find_map(|repo| Some((repo, repo.commit_of(name).ok().flatten()?))))
            .ok_or_else(|| match rev {
                GitBase::Main => CoreError::Invalid("this machine's clone has no main branch".into()),
                GitBase::Rev(rev) => CoreError::Invalid(format!("{rev} names no commit in this machine's clone")),
            })?;
        let in_git: BTreeMap<String, GitOid> = repo
            .files_at(commit)?
            .into_iter()
            .filter(|file| file.mode != silt_git::FileMode::Symlink)
            .map(|file| (file.path, file.oid))
            .collect();
        let tree = family.tree().at(|id| head.contains(id));
        let ours: BTreeMap<String, InodeId> =
            tree.paths().into_iter().filter(|(_, inode)| tree.kind(*inode) == Some(InodeKind::File)).collect();
        let text = |oid: GitOid| repos.iter().find_map(|repo| RepoBases(repo).text(oid));
        let paths: BTreeSet<&String> = in_git.keys().chain(ours.keys()).filter(|path| only.has(path)).collect();
        let mut changes = Vec::new();
        for path in paths {
            let theirs = in_git.get(path).copied();
            let inode = ours.get(path).copied();
            let view = inode.and_then(|inode| family.file_at(inode, head));
            if let (Some(oid), Some(FileView::GitText(same))) = (theirs, &view)
                && oid == *same
            {
                continue;
            }
            let old_text = theirs.map(text);
            let new_text = match &view {
                Some(FileView::Text(written)) => Some(Some(written.clone())),
                Some(FileView::GitText(oid)) => Some(text(*oid)),
                Some(FileView::Blob(_)) => Some(None),
                None => None,
            };
            if old_text == new_text {
                continue;
            }
            let binary = matches!(old_text, Some(None)) || matches!(new_text, Some(None));
            changes.push(FileChange {
                inode,
                old_path: theirs.map(|_| path.clone()),
                new_path: view.as_ref().map(|_| path.clone()),
                old_text: old_text.flatten().unwrap_or_default(),
                new_text: new_text.flatten().unwrap_or_default(),
                binary,
            });
        }
        Ok(page_of(changes, only.files))
    }

    /// A file of the thread as a permalink shows it: at the version the link
    /// names, or at the thread's head for a new link (ADR 0031).
    pub(crate) fn file_for_link(&self, thread: ThreadId, query: &silt_proto::view::FileQuery) -> Result<FileAt, CoreError> {
        let family_id = self.threads[&thread].family;
        let family = self.families().get(&family_id).ok_or_else(|| {
            CoreError::Invalid(format!("thread {} isn't on this machine yet; resume or fork it first", thread.short()))
        })?;
        let at = match &query.at {
            Some(text) => text.parse().map_err(|_| CoreError::Invalid(format!("{text:?} is not a version")))?,
            None => family.thread_head(thread),
        };
        let missing = || CoreError::Invalid(format!("the thread had no {} at that version", query.path));
        let mut media = None;
        let text = match family.file_in_thread_at(thread, &query.path, &at).ok_or_else(missing)? {
            FileView::Text(text) => Some(text),
            FileView::GitText(oid) => Some(
                self.family_repos(family_id)
                    .iter()
                    .find_map(|repo| RepoBases(repo).text(oid))
                    .ok_or_else(|| CoreError::Invalid(format!("this machine lacks the git blob {oid}")))?,
            ),
            FileView::Blob(hash) => {
                // An image or a video plays on the page, by its blob (ADR 0120).
                media = self.file_media(hash, &query.path);
                if media.is_some() {
                    self.shows_file(thread, hash);
                }
                None
            }
        };
        Ok(FileAt { path: query.path.clone(), at: at.to_string(), text, media })
    }

    /// What a binary file is when it's an image or a video: by its first
    /// bytes when it's here, else by its name, as the server has it.
    fn file_media(&self, hash: silt_types::BlobHash, path: &str) -> Option<silt_proto::view::FileMedia> {
        let (media_type, size) = match self.store.blobs().file(hash) {
            Some(file) => {
                let (media_type, _) = silt_media::sniff(&crate::artifacts::head_of(&file).ok()?)?;
                (media_type, std::fs::metadata(&file).ok()?.len())
            }
            None => (media_type_by_name(path)?, 0),
        };
        Some(silt_proto::view::FileMedia { blob: hash.to_string(), media_type: media_type.to_owned(), size })
    }

    /// How the family's files differ between two versions.
    pub(crate) fn changes(&self, family_id: FamilyId, before: &VersionVector, after: &VersionVector) -> Vec<FileChange> {
        self.changes_in(family_id, before, after, Only::default(), None)
    }

    /// [`Self::changes`] with what `hide` names left out on both sides
    /// (ADR 0069, ADR 0089).
    pub(crate) fn changes_hiding(
        &self,
        family_id: FamilyId,
        before: &VersionVector,
        after: &VersionVector,
        hide: Hide,
    ) -> Vec<FileChange> {
        let hidden = self.families().get(&family_id).and_then(|family| Hidden::of(family.log(), hide));
        self.changes_in(family_id, before, after, Only::default(), hidden.as_ref())
    }

    /// [`Self::changes`], in path order, only those `only` names, without
    /// the edits in `hidden`.
    fn changes_in(
        &self,
        family_id: FamilyId,
        before: &VersionVector,
        after: &VersionVector,
        only: Only<'_>,
        hidden: Option<&Hidden>,
    ) -> Vec<FileChange> {
        let Some(family) = self.families().get(&family_id) else {
            return Vec::new();
        };
        let repo = self.family_repo(family_id).ok();
        let mut bases = repo.as_ref().map(RepoBases);
        let hidden = hidden.filter(|hidden| !hidden.is_empty());
        let seen = |vv: &VersionVector, id: DeltaId| vv.contains(id) && !hidden.is_some_and(|hidden| hidden.contains(id));
        let file_at = |inode: InodeId, vv: &VersionVector| match hidden {
            Some(_) => family.file_where(inode, |id| seen(vv, id)),
            None => family.file_at(inode, vv),
        };
        let files_at = |vv: &VersionVector| -> BTreeMap<InodeId, String> {
            let tree = family.tree().at(|id| seen(vv, id));
            tree.paths()
                .into_iter()
                .filter(|(_, inode)| tree.kind(*inode) == Some(InodeKind::File))
                .map(|(path, inode)| (inode, path))
                .collect()
        };
        let (before_files, after_files) = (files_at(before), files_at(after));
        let inodes: BTreeSet<InodeId> = before_files.keys().chain(after_files.keys()).copied().collect();
        let mut git_texts: HashMap<GitOid, String> = HashMap::new();
        let mut text_of = |view: &Option<FileView>| -> String {
            match view {
                Some(FileView::Text(text)) => text.clone(),
                Some(FileView::GitText(oid)) => git_texts
                    .entry(*oid)
                    .or_insert_with(|| bases.as_mut().and_then(|bases| bases.text(*oid)).unwrap_or_default())
                    .clone(),
                Some(FileView::Blob(_)) | None => String::new(),
            }
        };
        let mut changed = Vec::new();
        for inode in inodes {
            let (old_path, new_path) = (before_files.get(&inode).cloned(), after_files.get(&inode).cloned());
            let old = old_path.as_ref().and_then(|_| file_at(inode, before));
            let new = new_path.as_ref().and_then(|_| file_at(inode, after));
            if old == new && old_path == new_path {
                continue;
            }
            if !(old_path.as_deref().is_some_and(|path| only.has(path)) || new_path.as_deref().is_some_and(|path| only.has(path)))
            {
                continue;
            }
            changed.push((inode, old_path, new_path, old, new));
        }
        changed.sort_by(|(_, old_a, new_a, ..), (_, old_b, new_b, ..)| {
            new_a.as_ref().or(old_a.as_ref()).cmp(&new_b.as_ref().or(old_b.as_ref()))
        });
        // Texts are read only for the page asked for: a change touching
        // thousands of files shows them a page at a time.
        page_of(changed, only.files)
            .into_iter()
            .map(|(inode, old_path, new_path, old, new)| FileChange {
                inode: Some(inode),
                binary: matches!(old, Some(FileView::Blob(_))) || matches!(new, Some(FileView::Blob(_))),
                old_text: text_of(&old),
                new_text: text_of(&new),
                old_path,
                new_path,
            })
            .collect()
    }

    /// Searches every server signed in to, and this machine for the threads
    /// it keeps to itself (ADR 0032). Without a server answering, it
    /// searches this machine alone.
    pub(crate) async fn search(&self, query: &str, limit: u32, filters: &SearchFilters) -> Result<Vec<SearchResult>, CoreError> {
        let remotes = self.asked_remotes().unwrap_or_default();
        if remotes.is_empty() && !filters.is_empty() {
            return self.filtered_here(query, limit, filters).await;
        }
        let request = silt_proto::SearchQuery {
            q: query.to_owned(),
            repo: filters.repo.clone(),
            author: filters.author.clone(),
            status: filters.status,
            branch: filters.branch.clone(),
            file: filters.file.clone(),
            since: filters.since,
            until: filters.until,
            limit: Some(limit),
        };
        let mut found = Vec::new();
        let mut answered = false;
        for remote in &remotes {
            match remote.search(&request).await {
                Ok(reply) => {
                    answered = true;
                    found.extend(reply.hits.into_iter().map(|hit| SearchResult {
                        thread: hit.thread.to_string(),
                        kind: hit.kind,
                        snippet: hit.snippet,
                        owner: Some(hit.owner),
                        status: Some(hit.status),
                        repo: Some(hit.repo),
                        time_ms: Some(hit.time_ms),
                        turn: None,
                        before_compaction: false,
                        artifact: hit.artifact,
                    }));
                }
                Err(error) if !filters.is_empty() => {
                    return Err(CoreError::Invalid(format!("{} could not search: {error}", remote.config.url)));
                }
                Err(error) => tracing::warn!(%error, server = remote.config.url, "server search failed"),
            }
        }
        if answered {
            let mut deltas = vec![None; found.len()];
            if filters.is_empty() {
                let kept_here = |thread: ThreadId| {
                    self.threads.get(&thread).is_some_and(|record| self.homes.get(&record.family) == Some(&Home::Local))
                };
                let (here, here_deltas) = self.local_hits(query, limit, kept_here).await?;
                found.extend(here);
                deltas.extend(here_deltas);
            }
            return Ok(self.placed(found, &deltas));
        }
        let (hits, deltas) = self.local_hits(query, limit, |_| true).await?;
        Ok(self.placed(hits, &deltas))
    }

    /// This machine's hits that `filters` keep, worked out from what it has
    /// as a server works them out from what it has: no server needed
    /// (ADR 0046).
    async fn filtered_here(&self, query: &str, limit: u32, filters: &SearchFilters) -> Result<Vec<SearchResult>, CoreError> {
        let summaries: HashMap<String, ThreadSummary> =
            self.thread_summaries().into_iter().map(|summary| (summary.id.clone(), summary)).collect();
        let now = crate::core::now_ms();
        let status_of = |summary: &ThreadSummary| {
            if summary.merged {
                silt_proto::ThreadStatus::Merged
            } else if now - summary.last_ms < silt_proto::ACTIVE_MS {
                silt_proto::ThreadStatus::Active
            } else {
                silt_proto::ThreadStatus::Idle
            }
        };
        let keep = |thread: ThreadId| {
            let Some(summary) = summaries.get(&thread.to_string()) else {
                return false;
            };
            filters.repo.as_ref().is_none_or(|repo| summary.repo == *repo)
                && filters.author.as_ref().is_none_or(|author| summary.owner == *author)
                && filters.status.is_none_or(|status| status_of(summary) == status)
                && filters.branch.as_ref().is_none_or(|branch| {
                    self.commits(thread).iter().any(|commit| commit.branch.as_deref() == Some(branch.as_str()))
                })
                && filters.file.as_ref().is_none_or(|file| !self.changed_under(thread, file).0.is_empty())
        };
        let (hits, deltas) = self.local_hits(query, limit, keep).await?;
        let (hits, deltas): (Vec<_>, Vec<_>) = hits
            .into_iter()
            .zip(deltas)
            .filter(|(hit, _)| {
                let at = hit.time_ms.unwrap_or(0);
                filters.since.is_none_or(|since| at >= since) && filters.until.is_none_or(|until| at <= until)
            })
            .unzip();
        Ok(self.placed(hits, &deltas))
    }

    /// This machine's search hits in the threads `keep` keeps, with the
    /// deltas they're in.
    async fn local_hits(
        &self,
        query: &str,
        limit: u32,
        keep: impl Fn(ThreadId) -> bool,
    ) -> Result<(Vec<SearchResult>, Vec<Option<DeltaId>>), CoreError> {
        let hits: Vec<_> = self.store().search(query, limit).await?.into_iter().filter(|hit| keep(hit.thread)).collect();
        let times = self.delta_times(hits.iter().map(|hit| (hit.thread, hit.delta)));
        let deltas: Vec<Option<DeltaId>> = hits.iter().map(|hit| Some(hit.delta)).collect();
        let hits = hits
            .into_iter()
            .map(|hit| SearchResult {
                time_ms: times.get(&hit.delta).copied(),
                thread: hit.thread.to_string(),
                kind: hit.kind,
                snippet: hit.snippet,
                owner: None,
                status: None,
                repo: None,
                turn: None,
                before_compaction: false,
                artifact: None,
            })
            .collect::<Vec<_>>();
        // A doc's or a sheet's hit comes first: it's the text as it stands.
        let edited = self.edited_hits(query, limit, &keep);
        let deltas = std::iter::repeat_n(None, edited.len()).chain(deltas).collect();
        Ok((edited.into_iter().chain(hits).collect(), deltas))
    }

    /// Hits in the current text of the docs and sheets of the threads
    /// `keep` keeps, newest first; their text isn't in the index, as it's
    /// the sum of every edit rather than any one delta.
    fn edited_hits(&self, query: &str, limit: u32, keep: impl Fn(ThreadId) -> bool) -> Vec<SearchResult> {
        let mut found = Vec::new();
        for summary in self.thread_summaries() {
            let Ok(thread) = self.find_thread(&summary.id) else { continue };
            if !keep(thread) {
                continue;
            }
            let entries = self.history_entries(thread);
            for (artifact, time_ms) in silt_proto::view::latest_artifacts(entries.iter().map(|(_, entry)| entry)) {
                let Ok(id) = artifact.id.parse::<silt_types::ArtifactId>() else { continue };
                let text = match artifact.kind.as_str() {
                    "doc" => self.doc_text(thread, id),
                    "sheet" => (0..self.sheet_count(thread, id))
                        .map(|sheet| silt_proto::edited::sheet_text(&self.sheet_cells(thread, id, sheet)))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    _ => continue,
                };
                if let Some(snippet) = silt_proto::edited::text_hit(&text, query) {
                    found.push(SearchResult {
                        thread: summary.id.clone(),
                        kind: artifact.kind.clone(),
                        snippet,
                        owner: None,
                        status: None,
                        repo: None,
                        time_ms: Some(time_ms),
                        turn: None,
                        before_compaction: false,
                        artifact: Some(artifact.id.clone()),
                    });
                }
            }
        }
        found.sort_by_key(|hit| std::cmp::Reverse(hit.time_ms));
        found.truncate(limit as usize);
        found
    }

    /// When each of these deltas was made, from the families here.
    fn delta_times(&self, wanted: impl Iterator<Item = (ThreadId, DeltaId)>) -> HashMap<DeltaId, i64> {
        let mut by_family: HashMap<FamilyId, HashSet<DeltaId>> = HashMap::new();
        for (thread, delta) in wanted {
            if let Some(record) = self.threads.get(&thread) {
                by_family.entry(record.family).or_default().insert(delta);
            }
        }
        let mut times = HashMap::new();
        for (family, ids) in by_family {
            for delta in self.local_log(family) {
                if ids.contains(&delta.span.first()) {
                    times.insert(delta.span.first(), delta.time_ms);
                }
            }
        }
        times
    }

    /// Search hits with their turn and whether a compaction came after,
    /// for threads whose history is here; `deltas` are the hits' own, when
    /// known.
    fn placed(&self, mut hits: Vec<SearchResult>, deltas: &[Option<DeltaId>]) -> Vec<SearchResult> {
        let mut timelines: HashMap<String, Option<crate::history::Timeline>> = HashMap::new();
        for (hit, delta) in hits.iter_mut().zip(deltas) {
            let Some(at) = hit.time_ms else {
                continue;
            };
            let timeline = timelines
                .entry(hit.thread.clone())
                .or_insert_with(|| self.find_thread(&hit.thread).ok().and_then(|thread| self.timeline(thread)));
            if let Some(timeline) = timeline {
                (hit.turn, hit.before_compaction) = timeline.place(*delta, at, hit.kind == "user");
            }
        }
        hits
    }
}

/// The title the agent gave the conversation, or else its first prompt.
pub(crate) fn title(conversation: &[&Delta]) -> Option<String> {
    named_title(conversation).or_else(|| first_prompt_and_messages(conversation).0)
}

/// The conversation's first prompt as a title, and how many prompts and
/// replies it has, read in one pass.
fn first_prompt_and_messages(conversation: &[&Delta]) -> (Option<String>, usize) {
    let mut transcripts = Transcripts::default();
    let mut first_prompt = None;
    let mut messages = 0;
    for entry in conversation.iter().flat_map(|delta| native_entries(&mut transcripts, delta)) {
        match entry {
            Entry::User { text } => {
                messages += 1;
                if first_prompt.is_none() {
                    first_prompt = text.lines().find(|line| !line.trim().is_empty()).map(|line| clip_title(line.trim()));
                }
            }
            Entry::Assistant { .. } => messages += 1,
            _ => {}
        }
    }
    (first_prompt, messages)
}

fn named_title(conversation: &[&Delta]) -> Option<String> {
    conversation.iter().rev().find_map(|delta| match &delta.op {
        Op::Conv(ConvOp::Native { line, .. }) => ai_title(line),
        Op::Conv(ConvOp::Title(title)) => Some(title.clone()),
        _ => None,
    })
}

/// The title an agent gave its conversation on `line`, if it did there.
fn ai_title(line: &str) -> Option<String> {
    // Parsing every line of a long conversation to find none is the
    // listing's main cost.
    if !line.contains("ai-title") {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    (value.get("type")?.as_str()? == "ai-title").then(|| value.get("aiTitle")?.as_str().map(str::to_owned)).flatten()
}

fn clip_title(text: &str) -> String {
    const TITLE_CHARS: usize = 60;
    if text.chars().count() <= TITLE_CHARS { text.to_owned() } else { text.chars().take(TITLE_CHARS).collect::<String>() + "…" }
}

/// Who made a group of edits in [`edits_by_tool`].
type EditsBy = (Option<String>, Option<SharedEdits>);

/// Files each tool call changed (`None`: changes made outside tool calls),
/// and apart from them those not this thread's agent's alone (ADR 0089).
fn edits_by_tool(family: &Family, thread: ThreadId) -> BTreeMap<EditsBy, Vec<FileEdit>> {
    let shared = |others, sessions: &[silt_log::SessionRef]| {
        let agents = sessions.iter().map(|session| format!("{} {:.8}", session.agent, session.id)).collect();
        Some(SharedEdits { others, agents })
    };
    let mut counts: BTreeMap<EditsBy, BTreeMap<InodeId, (usize, usize)>> = BTreeMap::new();
    for delta in family.log().iter().filter(|delta| delta.thread == thread) {
        let tool = match &delta.author {
            Author::Agent { tool_use_id, .. } => (tool_use_id.clone(), None),
            Author::User => (None, None),
            Author::Import => continue,
            Author::Others { sessions } => (None, shared(true, sessions)),
            Author::Agents { sessions } => (None, shared(false, sessions)),
        };
        let Some(inode) = touched_inode(delta) else {
            continue;
        };
        let counter = counts.entry(tool).or_default().entry(inode).or_default();
        if let Op::Text { op, .. } = &delta.op {
            match op {
                silt_crdt::TextOp::Insert { content, .. } => counter.0 += content.chars().count(),
                silt_crdt::TextOp::Delete { len, .. } => counter.1 += len,
            }
        }
    }
    let paths: HashMap<InodeId, String> = family.tree().paths().into_iter().map(|(path, inode)| (inode, path)).collect();
    counts
        .into_iter()
        .map(|(tool, files)| {
            let files = files
                .into_iter()
                .filter(|(inode, _)| family.tree().kind(*inode) != Some(silt_crdt::InodeKind::Dir))
                .map(|(inode, (inserted, deleted))| FileEdit {
                    path: paths.get(&inode).cloned().unwrap_or_else(|| "(deleted)".into()),
                    inserted,
                    deleted,
                })
                .collect();
            (tool, files)
        })
        .collect()
}

/// One file's change between two versions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileChange {
    /// None for a file only git has.
    pub(crate) inode: Option<InodeId>,
    pub(crate) old_path: Option<String>,
    pub(crate) new_path: Option<String>,
    pub(crate) old_text: String,
    pub(crate) new_text: String,
    /// Content on either side is stored whole rather than as text; the
    /// texts are then empty.
    pub(crate) binary: bool,
}

impl FileChange {
    pub(crate) fn path(&self) -> &str {
        self.new_path.as_deref().or(self.old_path.as_deref()).unwrap_or_default()
    }
}

/// Which of a diff's files to read: a page of them, the one at a path, or
/// all of them.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Only<'a> {
    pub(crate) files: Option<FilePage>,
    pub(crate) path: Option<&'a str>,
}

impl Only<'_> {
    fn has(&self, path: &str) -> bool {
        self.path.is_none_or(|only| only == path)
    }
}

/// A file's counts and size, as its part of the diff has them.
fn diff_stat(change: &FileChange) -> DiffStat {
    let rendered = render_diff(std::slice::from_ref(change));
    let (mut added, mut removed) = (0, 0);
    // Past the headers, a line's first character says what it is.
    let mut in_hunk = false;
    for line in rendered.lines() {
        if line.starts_with("@@") {
            in_hunk = true;
        } else if in_hunk && line.starts_with('+') {
            added += 1;
        } else if in_hunk && line.starts_with('-') {
            removed += 1;
        }
    }
    DiffStat {
        path: change.path().to_owned(),
        old_path: change.new_path.as_ref().and(change.old_path.clone()).filter(|old| Some(old) != change.new_path.as_ref()),
        added,
        removed,
        binary: change.binary,
        size: rendered.len() as u64,
    }
}

/// The page of `items` that `files` asks for, or all of them.
fn page_of<T>(items: Vec<T>, files: Option<FilePage>) -> Vec<T> {
    match files {
        Some(FilePage { offset, limit }) => items.into_iter().skip(offset).take(limit).collect(),
        None => items,
    }
}

/// Changes as a git-style unified diff, with renames.
pub(crate) fn render_diff(changes: &[FileChange]) -> String {
    let mut out = String::new();
    for change in changes {
        let shown = change.path();
        let (old, new) = (change.old_path.as_deref().unwrap_or(shown), change.new_path.as_deref().unwrap_or(shown));
        out.push_str(&format!("diff --git a/{old} b/{new}\n"));
        if let (Some(old), Some(new)) = (&change.old_path, &change.new_path)
            && old != new
        {
            out.push_str(&format!("rename from {old}\nrename to {new}\n"));
        }
        let old_name = change.old_path.as_ref().map_or_else(|| "/dev/null".to_owned(), |path| format!("a/{path}"));
        let new_name = change.new_path.as_ref().map_or_else(|| "/dev/null".to_owned(), |path| format!("b/{path}"));
        if change.binary {
            out.push_str(&format!("Binary files {old_name} and {new_name} differ\n"));
        } else if change.old_text != change.new_text {
            let diff = TextDiff::from_lines(&change.old_text, &change.new_text);
            out.push_str(&diff.unified_diff().header(&old_name, &new_name).to_string());
        }
    }
    out
}

/// A diff base that is a commit in the clone rather than a version of the
/// thread: its main branch, or `branch:<rev>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitBase<'a> {
    Main,
    Rev(&'a str),
}

/// Which turns a diff covers: one, or `turn` to `to_turn`; none for the
/// diff from a base.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TurnRange {
    pub(crate) turn: Option<usize>,
    pub(crate) to_turn: Option<usize>,
}

/// The versions a diff runs between, or a branch as git has it against the
/// thread's head.
pub(crate) enum DiffRange<'a> {
    Versions { from: VersionVector, to: VersionVector },
    Git { head: VersionVector, rev: GitBase<'a> },
}

impl DiffRange<'_> {
    /// Whether it runs between versions of the thread, rather than against
    /// a branch, which is git's already.
    pub(crate) fn is_versions(&self) -> bool {
        matches!(self, Self::Versions { .. })
    }

    /// The version whose files the diff's new side shows.
    pub(crate) fn to(&self) -> &VersionVector {
        match self {
            Self::Versions { to, .. } => to,
            Self::Git { head, .. } => head,
        }
    }
}

/// The versions a diff of `thread` runs between: a range of its turns,
/// from the first's prompt to where the last ended, else from `base` to its
/// head.
pub(crate) fn diff_range<'a>(
    family: &Family,
    thread: ThreadId,
    turns: TurnRange,
    base: Option<&'a str>,
) -> Result<DiffRange<'a>, CoreError> {
    let graph = family.graph();
    let head = family.thread_head(thread);
    let Some(first) = turns.turn else {
        if turns.to_turn.is_some() {
            return Err(CoreError::Invalid("a range of turns needs its first turn".into()));
        }
        return Ok(match base.and_then(git_base) {
            Some(rev) => DiffRange::Git { head: graph.version_vector(&head), rev },
            None => DiffRange::Versions { from: diff_base(family, thread, base)?, to: graph.version_vector(&head) },
        });
    };
    let last = turns.to_turn.unwrap_or(first);
    if last < first {
        return Err(CoreError::Invalid(format!("turn {last} comes before turn {first}")));
    }
    let marks = prompt_marks(family, thread);
    let at = |turn: usize| {
        turn.checked_sub(1)
            .and_then(|index| marks.get(index))
            .ok_or_else(|| CoreError::Invalid(format!("thread has {} turns", marks.len())))
    };
    let (start, last_start) = (at(first)?, at(last)?);
    let next = marks.get(last).cloned().unwrap_or_else(|| head.clone());
    let end = turn_end(family, thread, last_start, &next).unwrap_or(next);
    Ok(DiffRange::Versions { from: graph.version_vector(start), to: graph.version_vector(&end) })
}

pub(crate) fn git_base(base: &str) -> Option<GitBase<'_>> {
    match base {
        "main" => Some(GitBase::Main),
        other => other.strip_prefix("branch:").map(GitBase::Rev),
    }
}

/// The version a diff of `thread` starts from (ADR 0011's diff bases).
pub(crate) fn diff_base(family: &Family, thread: ThreadId, base: Option<&str>) -> Result<VersionVector, CoreError> {
    let graph = family.graph();
    let head = family.thread_head(thread);
    let in_thread = graph.version_vector(&head);
    // The version at each commit; the files are imported after genesis.
    let commits = || {
        family.log().iter().filter(|delta| in_thread.contains(delta.span.first())).filter_map(|delta| match &delta.op {
            Op::Git(GitOp::Genesis { commit, .. }) => Some((imported_version(family, delta.thread), *commit)),
            Op::Git(GitOp::Committed { commit, .. }) => Some((Frontier::single(delta.span.last()), *commit)),
            _ => None,
        })
    };
    let version = match base {
        None | Some("start") => start_version(family, thread),
        Some("merge-base") => match family.thread(thread).and_then(|state| state.forked_from()) {
            Some(source) => return Ok(in_thread.intersection(&graph.version_vector(&family.thread_head(source)))),
            None => start_version(family, thread),
        },
        Some("last-commit") => commits()
            .next_back()
            .map(|(version, _)| version)
            .ok_or_else(|| CoreError::Invalid("the thread has no commits".into()))?,
        Some("last-turn") => {
            prompt_marks(family, thread).pop().ok_or_else(|| CoreError::Invalid("the thread has no turns".into()))?
        }
        Some(other) => {
            let sha = other.strip_prefix("commit:").ok_or_else(|| {
                CoreError::Invalid(format!(
                    "unknown diff base {other:?}; use merge-base, last-commit, last-turn, commit:<sha>, main or branch:<name>"
                ))
            })?;
            commits()
                .rfind(|(_, commit)| commit.to_string().starts_with(sha))
                .map(|(version, _)| version)
                .ok_or_else(|| CoreError::Invalid(format!("the thread never was at a commit {sha}")))?
        }
    };
    Ok(graph.version_vector(&version))
}

/// Where the thread's own work starts: its fork point, or the import of
/// its files.
fn start_version(family: &Family, thread: ThreadId) -> Frontier {
    family.thread(thread).and_then(|state| state.fork_point().cloned()).unwrap_or_else(|| imported_version(family, thread))
}

/// The version right after the thread's files were imported.
fn imported_version(family: &Family, thread: ThreadId) -> Frontier {
    family
        .log()
        .iter()
        .rev()
        .find(|delta| {
            delta.thread == thread
                && delta.author == Author::Import
                && matches!(
                    delta.op,
                    Op::Create { .. } | Op::SetContent { .. } | Op::Text { .. } | Op::Move { .. } | Op::Delete { .. }
                )
        })
        .map_or_else(Frontier::root, |delta| Frontier::single(delta.span.last()))
}

/// The version at the start of each prompt.
/// Where the turn from `start` ended, before `next`: at its agent's stop,
/// with the agent's edits recorded just after it, as transcripts trail them.
/// What the person did in the clone between turns isn't the turn's (ADR
/// 0047). None when the agent didn't stop before `next`.
fn turn_end(family: &Family, thread: ThreadId, start: &Frontier, next: &Frontier) -> Option<Frontier> {
    let graph = family.graph();
    let (from, to) = (graph.version_vector(start), graph.version_vector(next));
    let within = |delta: &&Delta| delta.thread == thread && !from.contains(delta.span.first()) && to.contains(delta.span.first());
    let in_turn: Vec<&Delta> = family.log().iter().filter(within).collect();
    let stop = in_turn.iter().position(|delta| matches!(&delta.op, Op::Conv(ConvOp::Turn { mark: TurnMark::Stopped, .. })))?;
    let late =
        in_turn[stop + 1..].iter().copied().filter(|delta| matches!(delta.author, Author::Agent { .. } | Author::Agents { .. }));
    Some(graph.minimize(std::iter::once(in_turn[stop]).chain(late).map(|delta| delta.span.last())))
}

pub(crate) fn prompt_marks(family: &Family, thread: ThreadId) -> Vec<Frontier> {
    family
        .conversation(thread)
        .filter(|delta| matches!(&delta.op, Op::Conv(ConvOp::Turn { mark: TurnMark::PromptSubmitted, .. })))
        .map(|delta| Frontier::single(delta.span.first()))
        .collect()
}

/// An image's or a video's media type by its file's extension.
fn media_type_by_name(path: &str) -> Option<&'static str> {
    let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_changed_binary_file_is_listed_as_such() {
        let change = FileChange {
            inode: None,
            old_path: Some("logo.png".into()),
            new_path: Some("logo.png".into()),
            old_text: String::new(),
            new_text: String::new(),
            binary: true,
        };
        assert_eq!(render_diff(&[change]), "diff --git a/logo.png b/logo.png\nBinary files a/logo.png and b/logo.png differ\n");
    }
}
