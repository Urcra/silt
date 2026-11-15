//! Comments on code lines and conversation entries (ADR 0011). Code
//! comments hold CRDT anchors, so they follow their lines through edits;
//! a pending review comment takes its anchors when written (ADR 0066).

use std::path::Path;

use silt_crdt::{Anchor, Bias, InodeId, TextDoc, VersionVector};
use silt_git::Repo;
use silt_log::{CommentOp, CommentTarget, Family, GitMade, Op};
use silt_proto::view::{PendingAnchor, PlaceAnchors};
use silt_store::ThreadRecord;
use silt_types::{FamilyId, ThreadId};

use crate::core::{Core, CoreError, RepoBases};
use crate::forks::short_mr;
use crate::protocol::{CommentLocation, CommentOn, CommentView, LineSpan, ReplyView};
use crate::views::{diff_base, git_base};

impl Core {
    /// Projects a family known only from the server, reading its git blobs
    /// from the clone at `cwd` or any other local clone of its repository.
    pub(crate) async fn ensure_projected(&mut self, family_id: FamilyId, cwd: &Path) -> Result<(), CoreError> {
        if self.families.contains_key(&family_id) {
            return Ok(());
        }
        let records: Vec<ThreadRecord> = self.threads.values().filter(|record| record.family == family_id).cloned().collect();
        let first = records.first().ok_or_else(|| CoreError::Invalid(format!("family {family_id} has no threads here")))?;
        let here = Repo::discover(cwd).ok().filter(|repo| repo.project().is_ok_and(|project| project == first.repo));
        let clone = here
            .or_else(|| {
                self.threads
                    .values()
                    .filter(|record| record.repo == first.repo && !record.repo_root.as_os_str().is_empty())
                    .find_map(|record| Repo::discover(&record.repo_root).ok())
            })
            .or_else(|| self.clones.get(&first.repo).and_then(|root| Repo::discover(root).ok()))
            .ok_or_else(|| {
                CoreError::Invalid(format!(
                    "no clone of {} is known here; run `silt fork` or `silt resume` in one first",
                    first.repo
                ))
            })?;
        if let Some(base) = self.genesis_commit(family_id)? {
            self.ensure_commit(&clone, family_id, first.id, base).await?;
        }
        self.project_family(family_id, &clone)?;
        for record in records {
            let updated = ThreadRecord { repo_root: clone.root().to_owned(), ..record };
            self.store.create_thread(&updated).await?;
            self.threads.insert(updated.id, updated);
        }
        Ok(())
    }

    pub(crate) async fn add_comment(
        &mut self,
        selector: &str,
        on: CommentOn,
        text: String,
        cwd: &Path,
    ) -> Result<CommentView, CoreError> {
        let thread = self.find_thread(selector)?;
        let family_id = self.threads[&thread].family;
        self.ensure_projected(family_id, cwd).await?;
        let hide_git = matches!(on, CommentOn::Lines { hide_git: true, .. });
        let target = match on {
            CommentOn::Entry { index } => {
                let entries = self.history_entries(thread);
                let (delta, _) = entries
                    .get(index)
                    .ok_or_else(|| CoreError::Invalid(format!("the thread has {} history entries", entries.len())))?;
                CommentTarget::Message { delta: *delta }
            }
            CommentOn::Lines { path, start, end, base, hide_git } => {
                self.code_target(family_id, thread, &path, (start, end), base.as_deref(), hide_git)?
            }
            CommentOn::Anchored { anchor } => {
                let (inode, start, end) = self.held_anchor(family_id, thread, &anchor)?;
                CommentTarget::Code { inode, start, end }
            }
        };
        let by = match self.own_user(thread) {
            user if user.is_empty() => "you".to_owned(),
            user => user,
        };
        let head = self.families[&family_id].thread_head(thread);
        let delta = self.append_op(family_id, thread, head, Op::Comment(CommentOp::Added { target, text, by })).await?;
        self.comments(selector, None, hide_git)?
            .into_iter()
            .find(|comment| comment.id == short_mr(delta.span.first()))
            .ok_or_else(|| CoreError::Invalid("the comment was not recorded".into()))
    }

    /// Anchors for lines `start..=end` (1-based) of `path` as the thread
    /// sees it now, or as it was at the diff base `base`; with `hide_git`,
    /// numbered without git's rewrites (ADR 0069).
    fn code_target(
        &mut self,
        family_id: FamilyId,
        thread: ThreadId,
        path: &str,
        (start, end): (usize, usize),
        base: Option<&str>,
        hide_git: bool,
    ) -> Result<CommentTarget, CoreError> {
        let repo = self.family_repo(family_id)?;
        let family = self.families.get_mut(&family_id).expect("projected above");
        let path = path.trim_start_matches("./");
        let git_made = hide_git.then(|| GitMade::of(family.log())).filter(|made| !made.is_empty());
        let (version, inode) = match base {
            None if git_made.is_none() => {
                let version = family.graph().version_vector(&family.thread_head(thread));
                (version, family.thread_view(thread).paths().get(path).copied())
            }
            None => {
                let version = family.graph().version_vector(&family.thread_head(thread));
                let made = git_made.as_ref().expect("checked above");
                let inode = family.tree().at(|id| version.contains(id) && !made.contains(id)).paths().get(path).copied();
                (version, inode)
            }
            Some(base) if git_base(base).is_some() => {
                return Err(CoreError::Invalid(format!(
                    "lines of {base} can't take comments, as it isn't a version of the thread; comment on the thread's own lines"
                )));
            }
            Some(base) => {
                let version = diff_base(family, thread, Some(base))?;
                let shown = |id| version.contains(id) && !git_made.as_ref().is_some_and(|made| made.contains(id));
                let inode = family.tree().at(shown).paths().get(path).copied();
                (version, inode)
            }
        };
        let inode = inode.ok_or_else(|| CoreError::Invalid(format!("{path} is not a file of this thread")))?;
        family.text_doc_or_base(inode, &mut RepoBases(&repo))?;
        let doc = family.text_doc(inode).expect("made a text document above");
        let shown = |id| version.contains(id) && !git_made.as_ref().is_some_and(|made| made.contains(id));
        let text = if git_made.is_some() { doc.text_where(shown) } else { doc.text_at_vv(&version) };
        let (from, to) = line_span(&text, start, end)
            .ok_or_else(|| CoreError::Invalid(format!("{path} has {} lines", text.lines().count())))?;
        let anchor = |pos, bias| -> Result<Anchor, CoreError> {
            Ok(if git_made.is_some() { doc.anchor_where(shown, pos, bias)? } else { doc.anchor_at_vv(&version, pos, bias)? })
        };
        Ok(CommentTarget::Code { inode, start: anchor(from, Bias::Right)?, end: anchor(to, Bias::Left)? })
    }

    /// Anchors lines for a pending review comment, as a comment on them
    /// would be, without recording anything (ADR 0066).
    pub(crate) async fn anchor_lines(&mut self, selector: &str, on: CommentOn, cwd: &Path) -> Result<PendingAnchor, CoreError> {
        let thread = self.find_thread(selector)?;
        let family_id = self.threads[&thread].family;
        self.ensure_projected(family_id, cwd).await?;
        let CommentOn::Lines { path, start, end, base, hide_git } = on else {
            return Err(CoreError::Invalid("only lines of code take an anchor".into()));
        };
        let target = self.code_target(family_id, thread, &path, (start, end), base.as_deref(), hide_git)?;
        let anchor = serde_json::to_string(&target).map_err(|error| CoreError::Invalid(error.to_string()))?;
        Ok(PendingAnchor { anchor })
    }

    /// Where pending review comments' anchors are in the thread now, and at
    /// the diff base asked for, as [`Self::comments`] places comments; an
    /// anchor that isn't this thread's is `Unknown`.
    pub(crate) async fn place_anchors(
        &mut self,
        selector: &str,
        place: PlaceAnchors,
        cwd: &Path,
    ) -> Result<Vec<CommentLocation>, CoreError> {
        let thread = self.find_thread(selector)?;
        let family_id = self.threads[&thread].family;
        let _ = self.ensure_projected(family_id, cwd).await;
        if !self.families.contains_key(&family_id) || place.anchors.len() > MAX_PLACED {
            return Ok(vec![CommentLocation::Unknown; place.anchors.len().min(MAX_PLACED)]);
        }
        let held: Vec<Option<(InodeId, Anchor, Anchor)>> =
            place.anchors.iter().map(|anchor| self.held_anchor(family_id, thread, anchor).ok()).collect();
        let family = &self.families[&family_id];
        let places = Places::new(family, thread, place.base.as_deref(), place.hide_git);
        Ok(held
            .into_iter()
            .map(|held| held.map_or(CommentLocation::Unknown, |(inode, start, end)| places.locate(inode, &start, &end)))
            .collect())
    }

    /// The anchors an untrusted anchor from a browser names, once they're
    /// known to be lines of one of this family's text files.
    fn held_anchor(
        &mut self,
        family_id: FamilyId,
        thread: ThreadId,
        anchor: &str,
    ) -> Result<(InodeId, Anchor, Anchor), CoreError> {
        let invalid = || CoreError::Invalid("that isn't an anchor of this thread's lines".into());
        if anchor.len() > MAX_ANCHOR {
            return Err(invalid());
        }
        let Ok(CommentTarget::Code { inode, start, end }) = serde_json::from_str::<CommentTarget>(anchor) else {
            return Err(invalid());
        };
        let repo = self.family_repo(family_id)?;
        let family = self.families.get_mut(&family_id).ok_or_else(invalid)?;
        let head = family.graph().version_vector(&family.thread_head(thread));
        let doc = family.text_doc_or_base(inode, &mut RepoBases(&repo)).map_err(|_| invalid())?;
        if doc.resolve_at_vv(&head, &start).is_err() || doc.resolve_at_vv(&head, &end).is_err() {
            return Err(invalid());
        }
        Ok((inode, start, end))
    }

    /// Records that `comments` went to the thread's agent (ADR 0055).
    pub(crate) async fn comments_sent(
        &mut self,
        selector: &str,
        comments: &[String],
        cwd: &Path,
    ) -> Result<Vec<CommentView>, CoreError> {
        let thread = self.find_thread(selector)?;
        let family_id = self.threads[&thread].family;
        self.ensure_projected(family_id, cwd).await?;
        let ids = comments.iter().map(|comment| self.comment_id(family_id, thread, comment)).collect::<Result<Vec<_>, _>>()?;
        if ids.is_empty() {
            return Err(CoreError::Invalid("name the comments that were sent".into()));
        }
        let head = self.families[&family_id].thread_head(thread);
        self.append_op(family_id, thread, head, Op::Comment(CommentOp::Sent { comments: ids })).await?;
        self.comments(selector, None, false)
    }

    pub(crate) async fn resolve_comment(&mut self, selector: &str, comment: &str, cwd: &Path) -> Result<CommentView, CoreError> {
        let thread = self.find_thread(selector)?;
        let family_id = self.threads[&thread].family;
        self.ensure_projected(family_id, cwd).await?;
        let id = self.comment_id(family_id, thread, comment)?;
        let head = self.families[&family_id].thread_head(thread);
        self.append_op(family_id, thread, head, Op::Comment(CommentOp::Resolved { comment: id })).await?;
        self.comment_view(selector, id)
    }

    /// Answers `comment`, or the comment a reply of that id answers.
    pub(crate) async fn reply(
        &mut self,
        selector: &str,
        comment: &str,
        text: String,
        cwd: &Path,
    ) -> Result<CommentView, CoreError> {
        if text.trim().is_empty() {
            return Err(CoreError::Invalid("there is nothing to reply".into()));
        }
        let thread = self.find_thread(selector)?;
        let family_id = self.threads[&thread].family;
        self.ensure_projected(family_id, cwd).await?;
        let parent = self.comment_id(family_id, thread, comment)?;
        let by = match self.own_user(thread) {
            user if user.is_empty() => "you".to_owned(),
            user => user,
        };
        let head = self.families[&family_id].thread_head(thread);
        self.append_op(family_id, thread, head, Op::Comment(CommentOp::Replied { parent, text, by })).await?;
        self.comment_view(selector, parent)
    }

    /// The comment `name` (short or whole) names on the thread; a reply's
    /// name means its comment.
    fn comment_id(&self, family_id: FamilyId, thread: ThreadId, name: &str) -> Result<silt_types::DeltaId, CoreError> {
        let named = |id: silt_types::DeltaId| short_mr(id) == name || id.to_string() == name;
        self.families[&family_id]
            .comments(thread)
            .into_iter()
            .find(|found| named(found.id) || found.replies.iter().any(|reply| named(reply.id)))
            .map(|found| found.id)
            .ok_or_else(|| CoreError::Invalid(format!("no comment {name} on this thread")))
    }

    fn comment_view(&mut self, selector: &str, id: silt_types::DeltaId) -> Result<CommentView, CoreError> {
        self.comments(selector, None, false)?
            .into_iter()
            .find(|found| found.id == short_mr(id))
            .ok_or_else(|| CoreError::Invalid("the comment disappeared".into()))
    }

    /// The thread's comments, with where code comments are at the diff base
    /// `base` too.
    pub(crate) async fn list_comments(
        &mut self,
        selector: &str,
        base: Option<&str>,
        hide_git: bool,
        cwd: &Path,
    ) -> Result<Vec<CommentView>, CoreError> {
        let thread = self.find_thread(selector)?;
        // Without a clone there's nothing to place code comments in yet.
        let _ = self.ensure_projected(self.threads[&thread].family, cwd).await;
        self.comments(selector, base, hide_git)
    }

    /// The thread's comments, placed in the thread as it is now, and at the
    /// diff base `base` where it names a version of the thread; with
    /// `hide_git`, as a diff without git's rewrites numbers the lines.
    pub(crate) fn comments(&mut self, selector: &str, base: Option<&str>, hide_git: bool) -> Result<Vec<CommentView>, CoreError> {
        let thread = self.find_thread(selector)?;
        let family_id = self.threads[&thread].family;
        if !self.families.contains_key(&family_id) {
            return Ok(Vec::new());
        }
        self.materialize_commented(family_id, thread);
        let family = &self.families[&family_id];
        let entries = self.history_entries(thread);
        let places = Places::new(family, thread, base, hide_git);
        Ok(family
            .comments(thread)
            .into_iter()
            .map(|comment| {
                let location = match comment.target {
                    CommentTarget::Message { delta } => {
                        entries.iter().find(|(id, _)| *id == delta).map_or(CommentLocation::Unknown, |(_, entry)| {
                            CommentLocation::Entry { index: entry.index, excerpt: entry.text.chars().take(120).collect() }
                        })
                    }
                    CommentTarget::Code { inode, start, end } => places.locate(inode, &start, &end),
                };
                CommentView {
                    id: short_mr(comment.id),
                    by: comment.by,
                    time_ms: comment.time_ms,
                    text: comment.text,
                    resolved: comment.resolved,
                    sent: comment.sent,
                    location,
                    replies: comment
                        .replies
                        .into_iter()
                        .map(|reply| ReplyView { id: short_mr(reply.id), by: reply.by, time_ms: reply.time_ms, text: reply.text })
                        .collect(),
                }
            })
            .collect())
    }
}

impl Core {
    /// Makes text documents of git-lazy files that comments point into, so
    /// their anchors resolve.
    fn materialize_commented(&mut self, family_id: FamilyId, thread: ThreadId) {
        let Ok(repo) = self.family_repo(family_id) else {
            return;
        };
        let family = self.families.get_mut(&family_id).expect("caller checked");
        for comment in family.comments(thread) {
            if let CommentTarget::Code { inode, .. } = comment.target {
                let _ = family.text_doc_or_base(inode, &mut RepoBases(&repo));
            }
        }
    }
}

/// The longest anchor a browser may hand back, and how many it may ask to
/// place at once: a pending review's are a few hundred bytes each.
const MAX_ANCHOR: usize = 4096;
const MAX_PLACED: usize = 500;

/// What placing a thread's anchored lines needs: its files' paths now and
/// at a diff base, both versions, and git's edits when a diff leaves them
/// out (ADR 0069).
struct Places<'a> {
    family: &'a Family,
    git_made: Option<GitMade>,
    paths: std::sync::Arc<std::collections::BTreeMap<String, InodeId>>,
    head: VersionVector,
    before: Option<VersionVector>,
    base_paths: Option<std::collections::BTreeMap<String, InodeId>>,
}

impl<'a> Places<'a> {
    fn new(family: &'a Family, thread: ThreadId, base: Option<&str>, hide_git: bool) -> Self {
        let paths = family.thread_view(thread).paths();
        let head = family.graph().version_vector(&family.thread_head(thread));
        let before = base.filter(|base| git_base(base).is_none()).and_then(|base| diff_base(family, thread, Some(base)).ok());
        let base_paths = before.as_ref().map(|version| family.tree().at(|id| version.contains(id)).paths());
        let git_made = hide_git.then(|| GitMade::of(family.log())).filter(|made| !made.is_empty());
        Self { family, git_made, paths, head, before, base_paths }
    }

    /// Where lines anchored in `inode` are now, outdated once their text is
    /// gone, and where they were at the base when they're there.
    fn locate(&self, inode: InodeId, start: &Anchor, end: &Anchor) -> CommentLocation {
        let named = |paths: &std::collections::BTreeMap<String, InodeId>| {
            paths.iter().find(|(_, found)| **found == inode).map(|(path, _)| path.clone())
        };
        let path = named(&self.paths).or_else(|| self.base_paths.as_ref().and_then(named));
        let doc = self.family.text_doc(inode);
        let now = doc.and_then(|doc| place(doc, &self.head, self.git_made.as_ref(), start, end));
        let then = doc
            .zip(self.before.as_ref())
            .and_then(|(doc, version)| place(doc, version, self.git_made.as_ref(), start, end))
            .filter(|(.., outdated)| !outdated)
            .map(|(start, end, _)| LineSpan { start, end });
        match (path, now) {
            (Some(path), Some((start_line, end_line, outdated))) => {
                CommentLocation::Lines { path, start: start_line, end: end_line, outdated, before: then }
            }
            _ => CommentLocation::Unknown,
        }
    }
}

/// Where anchored lines are at `version`, without the edits in
/// `git_made`: first and last line (1-based) and whether the commented
/// text is gone.
fn place(
    doc: &TextDoc,
    version: &VersionVector,
    git_made: Option<&GitMade>,
    start: &Anchor,
    end: &Anchor,
) -> Option<(usize, usize, bool)> {
    let (text, start, end) = match git_made {
        None => (doc.text_at_vv(version), doc.resolve_at_vv(version, start).ok()?, doc.resolve_at_vv(version, end).ok()?),
        Some(made) => {
            let shown = |id| version.contains(id) && !made.contains(id);
            (doc.text_where(shown), doc.resolve_where(shown, start).ok()?, doc.resolve_where(shown, end).ok()?)
        }
    };
    let line_of = |pos: usize| text.chars().take(pos).filter(|ch| *ch == '\n').count() + 1;
    let first = line_of(start.pos);
    let last = line_of(end.pos.max(start.pos)).max(first);
    Some((first, last, start.outdated && end.outdated))
}

/// Character offsets of the start of line `start` and the end of line
/// `end` (1-based, inclusive, newline excluded).
fn line_span(text: &str, start: usize, end: usize) -> Option<(usize, usize)> {
    if start == 0 || end < start {
        return None;
    }
    let mut offset = 0;
    let mut span_start = None;
    for (number, line) in text.split_inclusive('\n').enumerate().map(|(index, line)| (index + 1, line)) {
        let content = line.trim_end_matches('\n').chars().count();
        if number == start {
            span_start = Some(offset);
        }
        if number == end {
            return Some((span_start?, offset + content));
        }
        offset += line.chars().count();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_spans_cover_whole_lines() {
        let text = "fn a() {}\nfn b() {}\nfn c() {}\n";
        assert_eq!(line_span(text, 2, 2), Some((10, 19)));
        assert_eq!(line_span(text, 1, 3), Some((0, 29)));
        assert_eq!(line_span(text, 4, 4), None);
        assert_eq!(line_span(text, 0, 1), None);
    }
}
