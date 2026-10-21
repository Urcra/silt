use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use silt_agents::{Entry, Transcripts};
use silt_crdt::{InodeId, InodeKind, VersionVector};
use silt_log::{Author, BaseText, ConvOp, Delta, Family, FileView, GitOp, HistoryItem, HistorySource, Op, TurnMark};
use silt_store::ThreadRecord;
use silt_types::{DeltaId, FamilyId, Frontier, GitOid, ThreadId};
use similar::TextDiff;

use crate::core::{Core, CoreError, RepoBases, touched_inode};
use silt_proto::view::FileAt;

use crate::homes::Home;

use crate::protocol::{FileEdit, SearchFilters, SearchResult, ShowLine, ThreadSummary};
use crate::transcripts::native_entries;

impl Core {
    /// Every thread here but those in the trash.
    pub(crate) fn thread_summaries(&self) -> Vec<ThreadSummary> {
        self.summaries(|thread| !self.deleted.contains(&thread))
    }

    pub(crate) fn summaries(&self, include: impl Fn(ThreadId) -> bool) -> Vec<ThreadSummary> {
        let records: Vec<&ThreadRecord> = self.thread_records().values().filter(|record| include(record.id)).collect();
        let families: BTreeSet<FamilyId> = records.iter().map(|record| record.family).collect();
        let mut by_thread: HashMap<ThreadId, Vec<&Delta>> = HashMap::new();
        for family in families {
            for delta in self.local_log(family) {
                by_thread.entry(delta.thread).or_default().push(delta);
            }
        }
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
                let deltas = by_thread.get(&record.id).map_or(&[][..], Vec::as_slice);
                let conversation: Vec<&Delta> = deltas.iter().copied().filter(|delta| matches!(delta.op, Op::Conv(_))).collect();
                let lease = deltas.iter().rev().find_map(|delta| match delta.op {
                    Op::Lease { device, .. } => Some(device),
                    _ => None,
                });
                let checked_out = checked_out.contains(&record.id);
                let (first_prompt, messages) = first_prompt_and_messages(&conversation);
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
                    title: named_title(&conversation).or(first_prompt).or_else(|| {
                        let family = self.families.get(&record.family)?;
                        let inherited: Vec<&Delta> = family.history(record.id).into_iter().map(|item| item.delta).collect();
                        title(&inherited)
                    }),
                    created_ms: record.created_ms,
                    agent: conversation.iter().rev().find_map(|delta| match &delta.op {
                        Op::Conv(ConvOp::Native { session, .. }) => Some(
                            silt_agents::agent(&session.agent)
                                .map_or_else(|| session.agent.clone(), |agent| agent.command().to_owned()),
                        ),
                        _ => None,
                    }),
                    messages,
                    edits: deltas.iter().filter(|delta| matches!(delta.op, Op::Text { .. })).count(),
                    location: location.into(),
                    forked_from: deltas.iter().find_map(|delta| match delta.op {
                        Op::Fork { from } => Some(from.to_string()),
                        _ => None,
                    }),
                    owner: record.owner.clone(),
                    collaborators: self.collaborators.get(&record.id).cloned().unwrap_or_default(),
                    writable: self.may_write(record.id),
                    agent_state: self.shown_state(record.id),
                    last_ms: deltas.iter().map(|delta| delta.time_ms).max().unwrap_or(record.created_ms),
                    merged: merged.contains(record.id.to_string().as_str()),
                    review: reviewing.contains(record.id.to_string().as_str()),
                    mine: self.personal.mine(record.id),
                    visibility: self.visibility.get(&record.id).cloned(),
                    server: self.remote_of(record.family).map(|remote| remote.config.url.clone()),
                    local_only: self.homes.get(&record.family) == Some(&crate::homes::Home::Local),
                }
            })
            .collect();
        summaries.sort_by_key(|summary| summary.created_ms);
        summaries
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
                            Entry::ToolCall { id, name, input } => {
                                lines.push(ShowLine::ToolCall { id, name, input: input.to_string() })
                            }
                            Entry::ToolResult { tool_use_id, text, .. } => {
                                lines.push(ShowLine::Entry { kind: "tool_result".into(), text });
                                if let Some(files) = edits.get(&Some(tool_use_id.clone())) {
                                    lines.push(ShowLine::Edits { tool_use_id: Some(tool_use_id), files: files.clone() });
                                }
                            }
                            Entry::Compaction { summary } => lines.push(ShowLine::Compaction { summary }),
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
        if let Some(files) = edits.get(&None) {
            lines.push(ShowLine::Edits { tool_use_id: None, files: files.clone() });
        }
        lines
    }

    pub(crate) fn diff(&self, thread: ThreadId, turn: Option<usize>, base: Option<&str>) -> Result<String, CoreError> {
        let family_id = self.thread_records()[&thread].family;
        let family = self
            .families()
            .get(&family_id)
            .ok_or_else(|| CoreError::Invalid("the thread isn't on this machine yet; use `silt resume`".into()))?;
        let graph = family.graph();
        let head = family.thread_head(thread);
        if turn.is_none()
            && let Some(rev) = base.and_then(git_base)
        {
            return self.diff_against_git(family_id, &graph.version_vector(&head), rev);
        }
        let (from, to) = match (turn, base) {
            (Some(turn), _) => {
                let marks = prompt_marks(family, thread);
                let start = turn
                    .checked_sub(1)
                    .and_then(|index| marks.get(index))
                    .ok_or_else(|| CoreError::Invalid(format!("thread has {} turns", marks.len())))?;
                let end = marks.get(turn).cloned().unwrap_or_else(|| head.clone());
                (graph.version_vector(start), graph.version_vector(&end))
            }
            (None, base) => (diff_base(family, thread, base)?, graph.version_vector(&head)),
        };
        Ok(render_diff(&self.changes(family_id, &from, &to)))
    }

    /// How the files at `head` differ from a commit's tree in this machine's
    /// clone, such as its main branch as it is now.
    fn diff_against_git(&self, family_id: FamilyId, head: &VersionVector, rev: GitBase) -> Result<String, CoreError> {
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
        let paths: BTreeSet<&String> = in_git.keys().chain(ours.keys()).collect();
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
        Ok(render_diff(&changes))
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
        let text = match family.file_in_thread_at(thread, &query.path, &at).ok_or_else(missing)? {
            FileView::Text(text) => Some(text),
            FileView::GitText(oid) => Some(
                self.family_repos(family_id)
                    .iter()
                    .find_map(|repo| RepoBases(repo).text(oid))
                    .ok_or_else(|| CoreError::Invalid(format!("this machine lacks the git blob {oid}")))?,
            ),
            FileView::Blob(_) => None,
        };
        Ok(FileAt { path: query.path.clone(), at: at.to_string(), text })
    }

    /// How the family's files differ between two versions.
    pub(crate) fn changes(&self, family_id: FamilyId, before: &VersionVector, after: &VersionVector) -> Vec<FileChange> {
        let Some(family) = self.families().get(&family_id) else {
            return Vec::new();
        };
        let repo = self.family_repo(family_id).ok();
        let mut bases = repo.as_ref().map(RepoBases);
        let files_at = |vv: &VersionVector| -> BTreeMap<InodeId, String> {
            let tree = family.tree().at(|id| vv.contains(id));
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
        let mut changes = Vec::new();
        for inode in inodes {
            let (old_path, new_path) = (before_files.get(&inode).cloned(), after_files.get(&inode).cloned());
            let old = old_path.as_ref().and_then(|_| family.file_at(inode, before));
            let new = new_path.as_ref().and_then(|_| family.file_at(inode, after));
            if old == new && old_path == new_path {
                continue;
            }
            changes.push(FileChange {
                inode: Some(inode),
                binary: matches!(old, Some(FileView::Blob(_))) || matches!(new, Some(FileView::Blob(_))),
                old_text: text_of(&old),
                new_text: text_of(&new),
                old_path,
                new_path,
            });
        }
        changes.sort_by(|a, b| a.path().cmp(b.path()));
        changes
    }

    /// Searches every server signed in to, and this machine for the threads
    /// it keeps to itself (ADR 0032). Without a server answering, it
    /// searches this machine alone.
    pub(crate) async fn search(&self, query: &str, limit: u32, filters: &SearchFilters) -> Result<Vec<SearchResult>, CoreError> {
        let remotes = self.asked_remotes().unwrap_or_default();
        if remotes.is_empty() && !filters.is_empty() {
            return Err(CoreError::Invalid("search filters need a server; use `silt auth login`".into()));
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
            })
            .collect();
        Ok((hits, deltas))
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
                (hit.turn, hit.before_compaction) = timeline.place(*delta, at);
            }
        }
        hits
    }
}

/// The title the agent gave the conversation, or else its first prompt.
fn title(conversation: &[&Delta]) -> Option<String> {
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
        // Parsing every line of a long conversation to find none is the
        // listing's main cost.
        Op::Conv(ConvOp::Native { line, .. }) if line.contains("ai-title") => {
            let value: serde_json::Value = serde_json::from_str(line).ok()?;
            (value.get("type")?.as_str()? == "ai-title").then(|| value.get("aiTitle")?.as_str().map(str::to_owned)).flatten()
        }
        Op::Conv(ConvOp::Title(title)) => Some(title.clone()),
        _ => None,
    })
}

fn clip_title(text: &str) -> String {
    const TITLE_CHARS: usize = 60;
    if text.chars().count() <= TITLE_CHARS { text.to_owned() } else { text.chars().take(TITLE_CHARS).collect::<String>() + "…" }
}

/// Files each tool call changed (`None`: changes made outside tool calls).
fn edits_by_tool(family: &Family, thread: ThreadId) -> BTreeMap<Option<String>, Vec<FileEdit>> {
    let mut counts: BTreeMap<Option<String>, BTreeMap<InodeId, (usize, usize)>> = BTreeMap::new();
    for delta in family.log().iter().filter(|delta| delta.thread == thread) {
        let tool = match &delta.author {
            Author::Agent { tool_use_id, .. } => tool_use_id.clone(),
            Author::User => None,
            Author::Import => continue,
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
enum GitBase<'a> {
    Main,
    Rev(&'a str),
}

fn git_base(base: &str) -> Option<GitBase<'_>> {
    match base {
        "main" => Some(GitBase::Main),
        other => other.strip_prefix("branch:").map(GitBase::Rev),
    }
}

/// The version a diff of `thread` starts from (ADR 0011's diff bases).
fn diff_base(family: &Family, thread: ThreadId, base: Option<&str>) -> Result<VersionVector, CoreError> {
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
fn prompt_marks(family: &Family, thread: ThreadId) -> Vec<Frontier> {
    family
        .conversation(thread)
        .filter(|delta| matches!(&delta.op, Op::Conv(ConvOp::Turn { mark: TurnMark::PromptSubmitted, .. })))
        .map(|delta| Frontier::single(delta.span.first()))
        .collect()
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
