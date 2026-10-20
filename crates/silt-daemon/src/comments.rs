//! Comments on code lines and conversation entries (ADR 0011). Code
//! comments hold CRDT anchors, so they follow their lines through edits.

use std::path::Path;

use silt_crdt::{Anchor, Bias};
use silt_git::Repo;
use silt_log::{CommentOp, CommentTarget, Family, Op};
use silt_store::ThreadRecord;
use silt_types::{FamilyId, ThreadId};

use crate::core::{Core, CoreError, RepoBases};
use crate::forks::short_mr;
use crate::protocol::{CommentLocation, CommentOn, CommentView};

impl Core {
    /// Projects a family known only from the server, reading its git blobs
    /// from the clone at `cwd` or any other local clone of its repository.
    pub(crate) async fn ensure_projected(&mut self, family_id: FamilyId, cwd: &Path) -> Result<(), CoreError> {
        if self.families.contains_key(&family_id) {
            return Ok(());
        }
        let records: Vec<ThreadRecord> = self
            .threads
            .values()
            .filter(|record| record.family == family_id)
            .cloned()
            .collect();
        let first = records
            .first()
            .ok_or_else(|| CoreError::Invalid(format!("family {family_id} has no threads here")))?;
        let here = Repo::discover(cwd).ok().filter(|repo| {
            repo.remote_url("origin")
                .ok()
                .flatten()
                .is_some_and(|url| silt_git::normalize_remote(&url) == first.repo)
        });
        let clone = here
            .or_else(|| {
                self.threads
                    .values()
                    .filter(|record| record.repo == first.repo && !record.repo_root.as_os_str().is_empty())
                    .find_map(|record| Repo::discover(&record.repo_root).ok())
            })
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
            let updated = ThreadRecord {
                repo_root: clone.root().to_owned(),
                ..record
            };
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
        let target = match on {
            CommentOn::Entry { index } => {
                let entries = self.history_entries(thread);
                let (delta, _) = entries
                    .get(index)
                    .ok_or_else(|| CoreError::Invalid(format!("the thread has {} history entries", entries.len())))?;
                CommentTarget::Message { delta: *delta }
            }
            CommentOn::Lines { path, start, end } => self.code_target(family_id, thread, &path, start, end)?,
        };
        let by = match self.own_user() {
            user if user.is_empty() => "you".to_owned(),
            user => user,
        };
        let head = self.families[&family_id].thread_head(thread);
        let delta = self
            .append_op(family_id, thread, head, Op::Comment(CommentOp::Added { target, text, by }))
            .await?;
        self.comments(selector)?
            .into_iter()
            .find(|comment| comment.id == short_mr(delta.span.first()))
            .ok_or_else(|| CoreError::Invalid("the comment was not recorded".into()))
    }

    /// Anchors for lines `start..=end` (1-based) of `path` as the thread
    /// sees it now.
    fn code_target(&mut self, family_id: FamilyId, thread: ThreadId, path: &str, start: usize, end: usize) -> Result<CommentTarget, CoreError> {
        let repo = self.family_repo(family_id)?;
        let family = self.families.get_mut(&family_id).expect("projected above");
        let inode = family
            .thread_view(thread)
            .paths()
            .get(path.trim_start_matches("./"))
            .copied()
            .ok_or_else(|| CoreError::Invalid(format!("{path} is not a file of this thread")))?;
        family.text_doc_or_base(inode, &mut RepoBases(&repo))?;
        let head = family.thread_head(thread);
        let doc = family.text_doc(inode).expect("made a text document above");
        let text = doc.text_at(family.graph(), &head);
        let (from, to) = line_span(&text, start, end)
            .ok_or_else(|| CoreError::Invalid(format!("{path} has {} lines", text.lines().count())))?;
        let anchor = |pos, bias| -> Result<Anchor, CoreError> {
            doc.anchor_at(family.graph(), &head, pos, bias)
                .map_err(|error| CoreError::Invalid(error.to_string()))
        };
        Ok(CommentTarget::Code {
            inode,
            start: anchor(from, Bias::Right)?,
            end: anchor(to, Bias::Left)?,
        })
    }

    pub(crate) async fn resolve_comment(&mut self, selector: &str, comment: &str, cwd: &Path) -> Result<CommentView, CoreError> {
        let thread = self.find_thread(selector)?;
        let family_id = self.threads[&thread].family;
        self.ensure_projected(family_id, cwd).await?;
        let family = &self.families[&family_id];
        let id = family
            .comments(thread)
            .into_iter()
            .find(|found| short_mr(found.id) == comment || found.id.to_string() == comment)
            .map(|found| found.id)
            .ok_or_else(|| CoreError::Invalid(format!("no comment {comment} on this thread")))?;
        let head = family.thread_head(thread);
        self.append_op(family_id, thread, head, Op::Comment(CommentOp::Resolved { comment: id }))
            .await?;
        self.comments(selector)?
            .into_iter()
            .find(|found| found.id == short_mr(id))
            .ok_or_else(|| CoreError::Invalid("the comment disappeared".into()))
    }

    pub(crate) async fn list_comments(&mut self, selector: &str, cwd: &Path) -> Result<Vec<CommentView>, CoreError> {
        let thread = self.find_thread(selector)?;
        // Without a clone there's nothing to place code comments in yet.
        let _ = self.ensure_projected(self.threads[&thread].family, cwd).await;
        self.comments(selector)
    }

    /// The thread's comments, placed in the thread as it is now.
    pub(crate) fn comments(&mut self, selector: &str) -> Result<Vec<CommentView>, CoreError> {
        let thread = self.find_thread(selector)?;
        let family_id = self.threads[&thread].family;
        if !self.families.contains_key(&family_id) {
            return Ok(Vec::new());
        }
        self.materialize_commented(family_id, thread);
        let family = &self.families[&family_id];
        let entries = self.history_entries(thread);
        let view = family.thread_view(thread);
        let paths = view.paths();
        Ok(family
            .comments(thread)
            .into_iter()
            .map(|comment| {
                let location = match comment.target {
                    CommentTarget::Message { delta } => entries
                        .iter()
                        .find(|(id, _)| *id == delta)
                        .map_or(CommentLocation::Unknown, |(_, entry)| CommentLocation::Entry {
                            index: entry.index,
                            excerpt: entry.text.chars().take(120).collect(),
                        }),
                    CommentTarget::Code { inode, start, end } => {
                        let path = paths.iter().find(|(_, found)| **found == inode).map(|(path, _)| path.clone());
                        match (path, place(family, thread, inode, &start, &end)) {
                            (Some(path), Some((start_line, end_line, outdated))) => CommentLocation::Lines {
                                path,
                                start: start_line,
                                end: end_line,
                                outdated,
                            },
                            _ => CommentLocation::Unknown,
                        }
                    }
                };
                CommentView {
                    id: short_mr(comment.id),
                    by: comment.by,
                    time_ms: comment.time_ms,
                    text: comment.text,
                    resolved: comment.resolved,
                    location,
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

/// Where anchored lines are at the thread's head: first and last line
/// (1-based) and whether the commented text is gone.
fn place(family: &Family, thread: ThreadId, inode: silt_crdt::InodeId, start: &Anchor, end: &Anchor) -> Option<(usize, usize, bool)> {
    let doc = family.text_doc(inode)?;
    let head = family.thread_head(thread);
    let text = doc.text_at(family.graph(), &head);
    let start = doc.resolve_at(family.graph(), &head, start).ok()?;
    let end = doc.resolve_at(family.graph(), &head, end).ok()?;
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
