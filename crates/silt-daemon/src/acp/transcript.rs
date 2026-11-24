//! Claude Code's transcript of the session Claude's adapter runs, read for
//! the ends of commands left in the background: the adapter reports only
//! those of the Bash tool, not of PowerShell's, so one PowerShell left
//! running would show running for good (claude-agent-acp 0.109). Nor does
//! it always report a subagent's end (0.110).

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use silt_agents::Task;

#[derive(Debug, Clone)]
pub(super) struct Transcript {
    session: String,
    /// Where it is, once Claude Code has written it.
    path: Option<PathBuf>,
    /// How far it has been read, to the end of a whole line.
    read: u64,
}

impl Transcript {
    /// The transcript of Claude's session `session` in `cwd`, read from its
    /// end as it is now: what it said before was of an earlier run.
    pub(super) fn of(cwd: &Path, session: &str) -> Self {
        let path = silt_agents::claude::transcript_of(cwd, session);
        let read = path.as_deref().and_then(|path| std::fs::metadata(path).ok()).map_or(0, |metadata| metadata.len());
        Self { session: session.to_owned(), path, read }
    }

    /// The tasks it reported ended since last read. It reads files, so it
    /// blocks.
    pub(super) fn read_ended(&mut self, cwd: &Path) -> io::Result<Vec<Task>> {
        if self.path.is_none() {
            self.path = silt_agents::claude::transcript_of(cwd, &self.session);
        }
        let Some(path) = &self.path else { return Ok(Vec::new()) };
        let mut file = File::open(path)?;
        let length = file.metadata()?.len();
        if length < self.read {
            self.read = 0;
        }
        file.seek(SeekFrom::Start(self.read))?;
        let mut added = Vec::new();
        file.take(length - self.read).read_to_end(&mut added)?;
        let whole = added.iter().rposition(|byte| *byte == b'\n').map_or(0, |end| end + 1);
        self.read += whole as u64;
        Ok(silt_agents::claude::tasks_ended(&String::from_utf8_lossy(&added[..whole])))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use silt_agents::TaskStatus;

    use super::*;

    fn ended(transcript: &mut Transcript, cwd: &Path) -> Vec<(String, TaskStatus)> {
        transcript.read_ended(cwd).unwrap().into_iter().map(|task| (task.id, task.status)).collect()
    }

    #[test]
    fn only_what_was_added_since_is_read_and_a_line_being_written_waits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        let report = |id: &str| {
            let text = format!("<task-notification><task-id>{id}</task-id><status>completed</status></task-notification>");
            serde_json::json!({"type": "user", "message": {"role": "user", "content": text}}).to_string()
        };
        std::fs::write(&path, format!("{}\n", report("before"))).unwrap();
        let mut transcript = Transcript { session: "s".into(), path: Some(path.clone()), read: 0 };
        transcript.read = std::fs::metadata(&path).unwrap().len();
        let cwd = dir.path();
        assert_eq!(ended(&mut transcript, cwd), []);

        let line = report("b1");
        let (start, rest) = line.split_at(20);
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(start.as_bytes()).unwrap();
        assert_eq!(ended(&mut transcript, cwd), [], "half a line");
        writeln!(file, "{rest}").unwrap();
        assert_eq!(ended(&mut transcript, cwd), [("b1".to_owned(), TaskStatus::Completed)]);
        assert_eq!(ended(&mut transcript, cwd), [], "read once");

        std::fs::write(&path, format!("{}\n", report("b2"))).unwrap();
        assert_eq!(ended(&mut transcript, cwd), [("b2".to_owned(), TaskStatus::Completed)], "written anew");
    }
}
