//! `silt blame` (ADR 0042): each line's thread, turn and prompt, from the
//! log when a thread here has the file, else from git blame and the notes.

use std::collections::HashMap;
use std::path::Path;

use silt_log::{Note, NoteAuthor, Origins};
use silt_types::{GitOid, ThreadId};

use crate::core::{Core, CoreError};
use crate::protocol::{BlameLine, BlameSource};

impl Core {
    pub(crate) fn blame(&self, cwd: &Path, path: &str) -> Result<Vec<BlameLine>, CoreError> {
        let (_, relative, here) = self.repo_path(cwd, path)?;
        let file = cwd.join(path);
        let Some(thread) = here else {
            return Ok(blame_from_notes(&silt_git::Repo::discover(cwd)?, &relative, &read_lines(&file)?));
        };
        let disk = read_lines(&file)?;
        let repo = silt_git::Repo::discover(cwd)?;
        let mut lines = blame_from_notes(&repo, &relative, &disk);
        let Some(family) = self.threads.get(&thread).and_then(|record| self.families.get(&record.family)) else {
            return Ok(lines);
        };
        let at = family.graph().version_vector(&family.thread_head(thread));
        let Some(ours) = family.lines_at(&relative, &at) else {
            return Ok(lines);
        };
        let origins = Origins::new(family.log());
        let texts: Vec<&str> = ours.iter().map(|(text, _)| text.as_str()).collect();
        let theirs: Vec<&str> = disk.iter().map(String::as_str).collect();
        let mut prompts: HashMap<ThreadId, Vec<String>> = HashMap::new();
        for op in similar::capture_diff_slices(similar::Algorithm::Myers, &texts, &theirs) {
            let similar::DiffOp::Equal { old_index, new_index, len } = op else {
                continue;
            };
            for offset in 0..len {
                let Some(origin) = ours[old_index + offset].1.and_then(|id| origins.of(id)) else {
                    continue;
                };
                let Some(author) = NoteAuthor::of(&origin.author) else {
                    continue;
                };
                let prompts = prompts.entry(origin.thread).or_insert_with(|| {
                    if self.threads.contains_key(&origin.thread) {
                        self.turns(origin.thread).into_iter().map(|turn| turn.prompt).collect()
                    } else {
                        Vec::new()
                    }
                });
                let line = &mut lines[new_index + offset];
                line.source = BlameSource::Log;
                line.thread = Some(origin.thread.to_string());
                line.turn = origin.turn;
                line.prompt = origin.turn.and_then(|turn| prompts.get(turn - 1)).map(|prompt| first_line(prompt));
                line.author = Some(author_name(&author));
                line.time_ms = Some(origin.time_ms);
            }
        }
        Ok(lines)
    }
}

/// Each line of `disk` with what git blame and the notes say of it, for
/// when no thread here has the file, or Silt isn't reachable at all.
pub fn blame_from_notes(repo: &silt_git::Repo, path: &str, disk: &[String]) -> Vec<BlameLine> {
    let mut lines: Vec<BlameLine> = disk
        .iter()
        .enumerate()
        .map(|(index, text)| BlameLine {
            line: u32::try_from(index + 1).unwrap_or(u32::MAX),
            text: text.clone(),
            source: BlameSource::Git,
            commit: None,
            thread: None,
            turn: None,
            prompt: None,
            author: None,
            time_ms: None,
        })
        .collect();
    let Ok(blamed) = repo.blame(path) else {
        return lines;
    };
    let mut notes: HashMap<GitOid, Option<Note>> = HashMap::new();
    for (line, blamed) in lines.iter_mut().zip(blamed) {
        line.commit = Some(blamed.commit.to_string());
        let note = notes
            .entry(blamed.commit)
            .or_insert_with(|| repo.note(blamed.commit).ok().flatten().and_then(|text| Note::parse(&text).ok()));
        let Some(note) = note else {
            continue;
        };
        let Some(run) = note.run_at(&blamed.path, blamed.line) else {
            continue;
        };
        line.source = BlameSource::Notes;
        line.thread = Some(run.thread.unwrap_or(note.thread).to_string());
        line.turn = run.turn;
        line.prompt = run.turn.filter(|_| run.thread.is_none()).and_then(|turn| note.prompt(turn)).map(str::to_owned);
        line.author = Some(author_name(&run.author));
    }
    lines
}

fn read_lines(file: &Path) -> Result<Vec<String>, CoreError> {
    let text = std::fs::read(file)?;
    Ok(String::from_utf8_lossy(&text).lines().map(str::to_owned).collect())
}

fn author_name(author: &NoteAuthor) -> String {
    match author {
        NoteAuthor::Person => "person".into(),
        NoteAuthor::Agent(agent) => agent.clone(),
    }
}

fn first_line(text: &str) -> String {
    text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default().to_owned()
}
