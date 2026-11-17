//! The git note Silt keeps on a commit in `refs/notes/silt` (ADR 0042):
//! the thread that made it, its turns, and who wrote which of its lines.

use std::fmt::Write as _;

use silt_types::ThreadId;

use crate::blame::LineOrigin;
use crate::delta::Author;

pub const NOTE_FORMAT: u32 = 1;
const PROMPT_CHARS: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub thread: ThreadId,
    /// The thread's permalink, when it has a server.
    pub link: Option<String>,
    pub title: Option<String>,
    /// Each turn that wrote a line of the commit, with its prompt's first line.
    pub turns: Vec<(usize, String)>,
    pub files: Vec<NoteFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteFile {
    pub path: String,
    pub runs: Vec<Run>,
}

/// Lines `start..=end` of a file as the commit has it, numbered from 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub start: u32,
    pub end: u32,
    /// Another thread of the family that wrote them; none for the note's own.
    pub thread: Option<ThreadId>,
    pub turn: Option<usize>,
    pub author: NoteAuthor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteAuthor {
    Person,
    Agent(String),
}

impl NoteAuthor {
    /// Content imported from git has no author worth a note.
    pub fn of(author: &Author) -> Option<Self> {
        match author {
            Author::User => Some(Self::Person),
            Author::Agent { agent, .. } => Some(Self::Agent(word(agent))),
            Author::Import => None,
            Author::Others { sessions } | Author::Agents { sessions } => {
                sessions.first().map(|session| Self::Agent(word(&session.agent)))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NoteError {
    #[error("not a Silt note")]
    NotSilt,
    #[error("Silt note format {0} is newer than this version reads")]
    Newer(u32),
    #[error("the note names no thread")]
    NoThread,
}

impl Note {
    pub fn render(&self) -> String {
        let mut text = format!("silt-note {NOTE_FORMAT}\n");
        match &self.link {
            Some(link) => _ = writeln!(text, "thread {} {}", self.thread, one_line(link)),
            None => _ = writeln!(text, "thread {}", self.thread),
        }
        if let Some(title) = &self.title {
            _ = writeln!(text, "title {}", clip(title));
        }
        for (turn, prompt) in &self.turns {
            let prompt = clip(prompt);
            if prompt.is_empty() {
                _ = writeln!(text, "turn {turn}");
            } else {
                _ = writeln!(text, "turn {turn} {prompt}");
            }
        }
        for file in self.files.iter().filter(|file| !file.path.chars().any(char::is_control)) {
            _ = writeln!(text, "file {}", file.path);
            for run in &file.runs {
                if run.start == run.end {
                    _ = write!(text, "{}", run.start);
                } else {
                    _ = write!(text, "{}-{}", run.start, run.end);
                }
                if let Some(thread) = run.thread {
                    _ = write!(text, " thread {thread}");
                }
                if let Some(turn) = run.turn {
                    _ = write!(text, " turn {turn}");
                }
                match &run.author {
                    NoteAuthor::Person => text.push_str(" person\n"),
                    NoteAuthor::Agent(agent) => _ = writeln!(text, " agent {agent}"),
                }
            }
        }
        text
    }

    /// Reads a note, skipping what it doesn't know, as later formats may
    /// add lines.
    pub fn parse(text: &str) -> Result<Self, NoteError> {
        let mut lines = text.lines();
        let format = lines
            .next()
            .and_then(|line| line.strip_prefix("silt-note "))
            .and_then(|format| format.trim().parse::<u32>().ok())
            .ok_or(NoteError::NotSilt)?;
        if format > NOTE_FORMAT {
            return Err(NoteError::Newer(format));
        }
        let (mut thread, mut link, mut title) = (None, None, None);
        let (mut turns, mut files) = (Vec::new(), Vec::<NoteFile>::new());
        for line in lines {
            let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
            match key {
                "thread" => {
                    let (id, url) = rest.split_once(' ').unwrap_or((rest, ""));
                    thread = id.parse().ok();
                    link = Some(url.to_owned()).filter(|url| !url.is_empty());
                }
                "title" => title = Some(rest.to_owned()),
                "turn" => {
                    let (turn, prompt) = rest.split_once(' ').unwrap_or((rest, ""));
                    if let Ok(turn) = turn.parse() {
                        turns.push((turn, prompt.to_owned()));
                    }
                }
                "file" => files.push(NoteFile { path: rest.to_owned(), runs: Vec::new() }),
                _ => {
                    if let (Some(file), Some(run)) = (files.last_mut(), parse_run(line)) {
                        file.runs.push(run);
                    }
                }
            }
        }
        Ok(Self { thread: thread.ok_or(NoteError::NoThread)?, link, title, turns, files })
    }

    /// The run holding line `line` of `path`, numbered as in the commit.
    pub fn run_at(&self, path: &str, line: u32) -> Option<&Run> {
        let file = self.files.iter().find(|file| file.path == path)?;
        file.runs.iter().find(|run| (run.start..=run.end).contains(&line))
    }

    /// The prompt of the note's own turn `turn`.
    pub fn prompt(&self, turn: usize) -> Option<&str> {
        self.turns.iter().find(|(number, _)| *number == turn).map(|(_, prompt)| prompt.as_str())
    }
}

fn parse_run(line: &str) -> Option<Run> {
    let mut words = line.split(' ');
    let range = words.next()?;
    let (start, end) = match range.split_once('-') {
        Some((start, end)) => (start.parse().ok()?, end.parse().ok()?),
        None => {
            let line = range.parse().ok()?;
            (line, line)
        }
    };
    let (mut thread, mut turn, mut author) = (None, None, None);
    while let Some(word) = words.next() {
        match word {
            "thread" => thread = words.next().and_then(|id| id.parse().ok()),
            "turn" => turn = words.next().and_then(|turn| turn.parse().ok()),
            "person" => author = Some(NoteAuthor::Person),
            "agent" => author = words.next().map(|agent| NoteAuthor::Agent(agent.to_owned())),
            _ => {}
        }
    }
    (start <= end && start > 0).then_some(Run { start, end, thread, turn, author: author? })
}

/// The runs of a committed file's changed lines that Silt wrote. `lines`
/// is the file as the thread had it, with each line's origin; lines of it
/// that differ from the commit's are matched by a line diff. `changed`
/// holds the commit's added or changed lines, numbered from 1.
pub fn runs_of(committed: &str, changed: &[u32], lines: &[(String, Option<LineOrigin>)], own: ThreadId) -> Vec<Run> {
    let theirs: Vec<&str> = committed.lines().collect();
    let ours: Vec<&str> = lines.iter().map(|(text, _)| text.as_str()).collect();
    let mut matched: Vec<Option<&LineOrigin>> = vec![None; theirs.len()];
    if theirs == ours {
        for (slot, (_, origin)) in matched.iter_mut().zip(lines) {
            *slot = origin.as_ref();
        }
    } else {
        let diff = similar::capture_diff_slices(similar::Algorithm::Myers, &ours, &theirs);
        for op in diff {
            if let similar::DiffOp::Equal { old_index, new_index, len } = op {
                for offset in 0..len {
                    matched[new_index + offset] = lines[old_index + offset].1.as_ref();
                }
            }
        }
    }
    let mut runs: Vec<Run> = Vec::new();
    for &line in changed {
        let Some(Some(origin)) = matched.get((line as usize).wrapping_sub(1)) else {
            continue;
        };
        let Some(author) = NoteAuthor::of(&origin.author) else {
            continue;
        };
        let thread = (origin.thread != own).then_some(origin.thread);
        match runs.last_mut() {
            Some(run) if run.end + 1 == line && run.thread == thread && run.turn == origin.turn && run.author == author => {
                run.end = line;
            }
            _ => runs.push(Run { start: line, end: line, thread, turn: origin.turn, author }),
        }
    }
    runs
}

fn clip(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default();
    let line = one_line(line);
    if line.chars().count() <= PROMPT_CHARS { line } else { line.chars().take(PROMPT_CHARS).collect::<String>() + "…" }
}

fn one_line(text: &str) -> String {
    text.chars().map(|ch| if ch.is_control() { ' ' } else { ch }).collect()
}

fn word(text: &str) -> String {
    text.chars().map(|ch| if ch.is_whitespace() || ch.is_control() { '-' } else { ch }).collect()
}

#[cfg(test)]
mod tests {
    use silt_types::{ActorId, DeltaId, Lamport};

    use super::*;

    fn origin(thread: u128, turn: usize, author: Author) -> Option<LineOrigin> {
        Some(LineOrigin {
            delta: DeltaId::new(ActorId(1), Lamport(1)),
            thread: ThreadId(thread),
            turn: Some(turn),
            author,
            time_ms: 0,
        })
    }

    fn agent() -> Author {
        Author::Agent { agent: "claude-code".into(), session: "s".into(), tool_use_id: None }
    }

    #[test]
    fn a_note_reads_back_as_written() {
        let note = Note {
            thread: ThreadId(7),
            link: Some("https://silt.example/app/t/07".into()),
            title: Some("Fix the\nlogin".into()),
            turns: vec![(3, "Fix the login redirect\nwhen it expired".into())],
            files: vec![NoteFile {
                path: "src/a b.rs".into(),
                runs: vec![
                    Run { start: 1, end: 4, thread: None, turn: Some(3), author: NoteAuthor::Agent("claude-code".into()) },
                    Run { start: 9, end: 9, thread: Some(ThreadId(8)), turn: None, author: NoteAuthor::Person },
                ],
            }],
        };
        let text = note.render();
        assert!(text.starts_with("silt-note 1\nthread "), "{text}");
        let read = Note::parse(&text).expect("parses");
        assert_eq!(read.title.as_deref(), Some("Fix the"));
        assert_eq!(read.turns, [(3, "Fix the login redirect".to_owned())]);
        assert_eq!(read.files, note.files);
        assert_eq!(read.run_at("src/a b.rs", 3).and_then(|run| run.turn), Some(3));
        assert_eq!(read.run_at("src/a b.rs", 5), None);
    }

    #[test]
    fn unknown_lines_are_skipped_and_newer_formats_refused() {
        let note =
            Note::parse(&format!("silt-note 1\nthread {}\nreviewed yes\nfile a\n2 turn 1 person\n2 sideways\n", ThreadId(1)))
                .expect("parses");
        assert_eq!(note.files[0].runs.len(), 1);
        assert_eq!(Note::parse("silt-note 9\n"), Err(NoteError::Newer(9)));
        assert_eq!(Note::parse("hello"), Err(NoteError::NotSilt));
    }

    #[test]
    fn runs_group_changed_lines_by_author_and_match_through_edits() {
        let own = ThreadId(1);
        let lines = vec![
            ("a".to_owned(), origin(1, 1, agent())),
            ("b".to_owned(), origin(1, 1, agent())),
            ("uncommitted".to_owned(), origin(1, 2, Author::User)),
            ("c".to_owned(), origin(2, 1, Author::User)),
            ("d".to_owned(), None),
        ];
        let runs = runs_of("a\nb\nc\nd\n", &[1, 2, 3, 4], &lines, own);
        assert_eq!(
            runs,
            [
                Run { start: 1, end: 2, thread: None, turn: Some(1), author: NoteAuthor::Agent("claude-code".into()) },
                Run { start: 3, end: 3, thread: Some(ThreadId(2)), turn: Some(1), author: NoteAuthor::Person },
            ],
            "the uncommitted line is matched around, and git's line is left out"
        );
    }
}
