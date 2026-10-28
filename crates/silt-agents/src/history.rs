//! A thread's transcripts read the way its history shows them, the same on
//! a machine and on a server: a subagent's entries are marked with its
//! transcript, and that transcript is tied to the call that started it.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::tool::{self, Shown, ToolKind};
use crate::{Entry, Task, TaskKind, TaskStatus, Transcripts};

/// Reads the lines of a log's sessions, in order.
#[derive(Debug, Default)]
pub struct HistoryReader {
    transcripts: Transcripts,
    /// Per session, the calls that started a subagent no transcript is tied
    /// to yet, oldest first.
    waiting: HashMap<String, Vec<Waiting>>,
    /// Subagent transcripts already tied to their call, by session and name.
    tied: HashSet<(String, String)>,
}

#[derive(Debug)]
struct Waiting {
    call: String,
    prompt: Option<String>,
}

impl HistoryReader {
    /// What one line of a session adds to its history. `side` names the
    /// subagent transcript the line is from; none for the session's own.
    pub fn line(&mut self, agent: &str, session: &str, side: Option<&str>, line: &str) -> Vec<Shown> {
        self.line_with(agent, session, side, line, |_| {})
    }

    /// [`Self::line`], showing `each` every entry the line holds first, as
    /// an index that also searches them reads it.
    pub fn line_with(
        &mut self,
        agent: &str,
        session: &str,
        side: Option<&str>,
        line: &str,
        mut each: impl FnMut(&Entry),
    ) -> Vec<Shown> {
        let key = side.map_or_else(|| "main".to_owned(), |name| format!("subagent:{name}"));
        let read = self.transcripts.push(agent, session, &key, line);
        let side = side.map(str::to_owned).or(read.side);
        let mut shown = Vec::new();
        for entry in read.entries {
            each(&entry);
            match &side {
                None => self.note(session, &entry),
                Some(name) => shown.extend(self.tie(session, name, &entry)),
            }
            if let Some(mut entry) = tool::shown(entry) {
                entry.side.clone_from(&side);
                shown.push(entry);
            }
        }
        shown
    }

    /// Keeps what the session's own transcript says of its subagents.
    fn note(&mut self, session: &str, entry: &Entry) {
        match entry {
            Entry::ToolCall { id, name, input, kind }
                if tool::describe(name, kind.as_deref(), input).kind == ToolKind::Subagent =>
            {
                let prompt = ["prompt", "task", "message"].iter().find_map(|key| input.get(*key).and_then(Value::as_str));
                self.waiting
                    .entry(session.to_owned())
                    .or_default()
                    .push(Waiting { call: id.clone(), prompt: prompt.map(str::to_owned) });
            }
            Entry::Task(Task { call: Some(call), side: Some(side), .. }) => {
                if let Some(waiting) = self.waiting.get_mut(session) {
                    waiting.retain(|waiting| &waiting.call != call);
                }
                self.tied.insert((session.to_owned(), side.clone()));
            }
            _ => {}
        }
    }

    /// The news that ties a subagent's transcript to the call that started
    /// it, when its first entry comes before the session's own says which
    /// call that was: the call it is named by, else the call that gave it
    /// its first prompt, else the oldest waiting one.
    fn tie(&mut self, session: &str, name: &str, entry: &Entry) -> Option<Shown> {
        if matches!(entry, Entry::Other { .. } | Entry::Usage { .. }) || !self.tied.insert((session.to_owned(), name.to_owned()))
        {
            return None;
        }
        let waiting = self.waiting.get_mut(session).filter(|waiting| !waiting.is_empty())?;
        let prompt = match entry {
            Entry::User { text } => Some(text.trim()),
            _ => None,
        };
        // A subagent written into the session's own file is named by its call.
        let at = waiting
            .iter()
            .position(|waiting| waiting.call == name)
            .or_else(|| {
                prompt.and_then(|prompt| {
                    waiting.iter().position(|waiting| waiting.prompt.as_deref().map(str::trim) == Some(prompt))
                })
            })
            .unwrap_or(0);
        let call = waiting.remove(at).call;
        tool::shown(Entry::Task(Task {
            id: name.to_owned(),
            call: Some(call),
            kind: Some(TaskKind::Subagent),
            status: TaskStatus::Running,
            side: Some(name.to_owned()),
            summary: None,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(id: &str, prompt: &str) -> String {
        serde_json::json!({"type": "assistant", "message": {"role": "assistant", "content": [
            {"type": "tool_use", "id": id, "name": "Agent", "input": {"description": "look", "prompt": prompt}}
        ]}})
        .to_string()
    }

    fn prompt(text: &str) -> String {
        serde_json::json!({"type": "user", "isSidechain": true, "message": {"role": "user", "content": text}}).to_string()
    }

    #[test]
    fn a_subagent_is_tied_to_the_call_that_gave_it_its_prompt() {
        let mut reader = HistoryReader::default();
        assert!(reader.line("claude-code", "s", None, &call("t1", "Read the lexer")).iter().all(|shown| shown.side.is_none()));
        reader.line("claude-code", "s", None, &call("t2", "Read the parser"));
        let shown = reader.line("claude-code", "s", Some("agent-b"), &prompt("Read the parser"));
        let task = shown[0].task.as_ref().expect("the tie comes first");
        assert_eq!((task.call.as_deref(), task.side.as_deref(), shown[0].side.as_deref()), (Some("t2"), Some("agent-b"), None));
        assert_eq!((shown[1].kind.as_str(), shown[1].side.as_deref()), ("user", Some("agent-b")));
        let shown = reader.line("claude-code", "s", Some("agent-a"), &prompt("Something else"));
        assert_eq!(shown[0].task.as_ref().and_then(|task| task.call.as_deref()), Some("t1"));
        assert_eq!(reader.line("claude-code", "s", Some("agent-a"), &prompt("Again")).len(), 1, "tied once");
    }

    #[test]
    fn a_subagent_the_session_already_tied_is_not_tied_again() {
        let mut reader = HistoryReader::default();
        reader.line("claude-code", "s", None, &call("t1", "Read the lexer"));
        let result = serde_json::json!({"type": "user", "toolUseResult": {"agentId": "a1", "status": "async_launched"},
            "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "launched"}]}});
        let shown = reader.line("claude-code", "s", None, &result.to_string());
        assert!(shown.iter().any(|shown| shown.task.as_ref().is_some_and(|task| task.side.as_deref() == Some("agent-a1"))));
        let shown = reader.line("claude-code", "s", Some("agent-a1"), &prompt("Read the lexer"));
        assert_eq!(shown.len(), 1);
        assert!(shown[0].task.is_none());
    }
}
