//! What the expanded review asks of a thread's diff (ADR 0067): the turn
//! that last wrote each of its lines, and which files the person marked
//! seen, with what changed since.

use std::str::FromStr;

use silt_crdt::VersionVector;
use silt_proto::SeenMark;
use silt_proto::view::{FileSeen, FileTurns, LineSpan, MarkSeen, Seen, ThreadSeenView, TurnRun};
use silt_types::{DeltaId, Frontier, ThreadId};

use crate::core::{Core, CoreError, now_ms};
use crate::views::{Only, TurnRange, diff_range, prompt_marks};

/// A file's lines, each with the id of its newest character.
type Lines = [(String, Option<DeltaId>)];

impl Core {
    /// For each file of the diff, the turns that last wrote its new lines.
    pub(crate) fn diff_turns(
        &self,
        thread: ThreadId,
        turns: TurnRange,
        base: Option<&str>,
        only: Only<'_>,
    ) -> Result<Vec<FileTurns>, CoreError> {
        let changes = self.diff_changes(thread, turns, base, only, false)?;
        let (_, family) = self.diff_family(thread)?;
        let graph = family.graph();
        let to = diff_range(family, thread, turns, base)?.to().clone();
        let marks: Vec<VersionVector> = prompt_marks(family, thread).iter().map(|mark| graph.version_vector(mark)).collect();
        Ok(changes
            .iter()
            .filter_map(|change| change.new_path.as_deref())
            .filter_map(|path| {
                let lines = family.lines_at(path, &to)?;
                Some(FileTurns {
                    path: path.to_owned(),
                    runs: runs_of(lines.iter().map(|(_, id)| id.map(|id| turn_of(&marks, id)))),
                })
            })
            .collect())
    }
}

impl Core {
    /// For each file of the diff the person marked seen, whether it's still
    /// as they saw it, and else which of its lines are new since.
    pub(crate) fn thread_seen(
        &self,
        thread: ThreadId,
        turns: TurnRange,
        base: Option<&str>,
    ) -> Result<ThreadSeenView, CoreError> {
        let changes = self.diff_changes(thread, turns, base, Only::default(), false)?;
        let (_, family) = self.diff_family(thread)?;
        let graph = family.graph();
        let to = diff_range(family, thread, turns, base)?.to().clone();
        let files = changes
            .iter()
            .filter_map(|change| change.new_path.as_deref().or(change.old_path.as_deref()))
            .filter_map(|path| {
                let mark = self.personal.thread_mark(thread, path).filter(|mark| mark.seen)?;
                let lines = family.lines_at(path, &to);
                if fingerprint(lines.as_deref()) == mark.hash {
                    return Some(FileSeen { path: path.to_owned(), seen: Seen::Seen, new_lines: Vec::new() });
                }
                // A version from another device not here yet counts every line as new.
                let seen = mark.version.as_deref().and_then(|version| Frontier::from_str(version).ok());
                let new_lines = match (&lines, seen) {
                    (Some(lines), Some(seen)) => {
                        let seen = graph.version_vector(&seen);
                        new_since(lines, |id| seen.contains(id))
                    }
                    _ => Vec::new(),
                };
                Some(FileSeen { path: path.to_owned(), seen: Seen::Changed, new_lines })
            })
            .collect();
        Ok(ThreadSeenView { head: family.thread_head(thread).to_string(), files })
    }

    /// Marks a file of the thread seen, or not, as it was at `at`, the
    /// version the review read: a change since then shows at once.
    pub(crate) async fn mark_thread_seen(&mut self, selector: &str, mark: MarkSeen) -> Result<&'static str, CoreError> {
        let thread = self.find_thread(selector)?;
        let (_, family) = self.diff_family(thread)?;
        let graph = family.graph();
        let head = family.thread_head(thread);
        // The browser names the version; only one of this thread's is taken.
        let at = Frontier::from_str(&mark.at)
            .ok()
            .filter(|at| !at.is_empty() && at.ids().iter().all(|id| graph.is_ancestor(*id, &head)))
            .ok_or_else(|| CoreError::Invalid(format!("{} isn't a version of this thread", mark.at)))?;
        let hash = fingerprint(family.lines_at(&mark.path, &graph.version_vector(&at)).as_deref());
        let seen = SeenMark { path: mark.path, hash, seen: mark.seen, at_ms: now_ms(), version: Some(at.to_string()) };
        self.put_thread_mark(thread, seen).await;
        Ok("marked")
    }
}

/// What a file's text was, to tell whether it changed since: BLAKE3 of its
/// lines, or of nothing where it's gone or isn't text.
fn fingerprint(lines: Option<&Lines>) -> String {
    let mut hasher = blake3::Hasher::new();
    for (text, _) in lines.unwrap_or_default() {
        hasher.update(text.as_bytes());
        hasher.update(b"\n");
    }
    hasher.finalize().to_hex().to_string()
}

/// The lines written since a version: those whose newest character it
/// doesn't hold, as runs; lines from git are never new.
fn new_since(lines: &Lines, seen: impl Fn(DeltaId) -> bool) -> Vec<LineSpan> {
    let mut spans: Vec<LineSpan> = Vec::new();
    for (index, (_, id)) in lines.iter().enumerate() {
        if !id.is_some_and(|id| !seen(id)) {
            continue;
        }
        let line = index + 1;
        match spans.last_mut() {
            Some(span) if span.end + 1 == line => span.end = line,
            _ => spans.push(LineSpan { start: line, end: line }),
        }
    }
    spans
}

/// The turn a character was written in: how many of the thread's prompts
/// came before it, the prompts' versions in order; 0 before the first.
fn turn_of(marks: &[VersionVector], id: DeltaId) -> usize {
    marks.partition_point(|mark| !mark.contains(id))
}

/// Each line's turn, from the first line, as runs of lines from one turn;
/// a line from no turn ends a run.
fn runs_of(turns: impl Iterator<Item = Option<usize>>) -> Vec<TurnRun> {
    let mut runs: Vec<TurnRun> = Vec::new();
    let mut previous = None;
    for (index, turn) in turns.enumerate() {
        let line = u32::try_from(index + 1).unwrap_or(u32::MAX);
        let turn = turn.filter(|turn| *turn > 0).map(|turn| u32::try_from(turn).unwrap_or(u32::MAX));
        match (turn, runs.last_mut()) {
            (Some(turn), Some(run)) if previous == Some(turn) && run.turn == turn => run.end = line,
            (Some(turn), _) => runs.push(TurnRun { start: line, end: line, turn }),
            (None, _) => {}
        }
        previous = turn;
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_written_since_a_version_are_new_and_lines_from_git_never_are() {
        let id = |lamport| DeltaId::new(silt_types::ActorId(1), silt_types::Lamport(lamport));
        let seen = |at: DeltaId| at.lamport.0 <= 5;
        let lines = [
            ("fn main() {}".to_owned(), None),
            ("fn farewell() {}".to_owned(), Some(id(3))),
            ("fn later() {}".to_owned(), Some(id(8))),
            ("fn last() {}".to_owned(), Some(id(9))),
        ];
        assert_eq!(new_since(&lines, seen), [LineSpan { start: 3, end: 4 }]);
        assert_ne!(fingerprint(Some(&lines)), fingerprint(Some(&lines[..3])));
        assert_eq!(fingerprint(None), fingerprint(Some(&[])));
    }

    #[test]
    fn lines_from_one_turn_run_together_and_others_break_them() {
        let turns = [Some(1), Some(1), Some(2), None, Some(2), Some(0), Some(2)];
        let runs = runs_of(turns.into_iter());
        assert_eq!(
            runs,
            [
                TurnRun { start: 1, end: 2, turn: 1 },
                TurnRun { start: 3, end: 3, turn: 2 },
                TurnRun { start: 5, end: 5, turn: 2 },
                TurnRun { start: 7, end: 7, turn: 2 },
            ]
        );
    }
}
