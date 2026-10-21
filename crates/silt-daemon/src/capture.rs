//! The capture step for an agent's hook (ADR 0018): read what the agent
//! wrote to its transcript since, take in the checkout's changes, then
//! record the hook's turn mark. It needs no event loop or timer, so a hook
//! can run it in its own process; the daemon runs the same step.
//!
//! Who made an edit follows from when it is found. Changes found at
//! `PostToolUse`, before the tool's finish is recorded, are the tool's;
//! changes found at any other hook were made since the last one, by the
//! person.

use std::path::{Path, PathBuf};
use std::time::Duration;

use silt_agents::Capture;
use silt_agents::claude::HookInput;
use silt_log::TurnMark;
use silt_store::Store;

use crate::capture_lock::{CaptureLock, Turn};
use crate::core::{Core, CoreError};
use crate::protocol::HookReply;

/// How long a hook waits for its turn at writing before it gives up, so a
/// daemon stopped while writing can't hang the agent.
const TURN_WAIT: Duration = Duration::from_secs(10);

/// Runs a hook's capture step in this process, without the daemon, for the
/// data in `data_dir`.
pub async fn capture_here(data_dir: &Path, event: &str, input: serde_json::Value) -> Result<HookReply, CoreError> {
    let input: HookInput = serde_json::from_value(input).map_err(CoreError::HookInput)?;
    let lock = CaptureLock::open(data_dir)?;
    let _turn = turn_in_time(&lock).await?;
    let store = Store::open(data_dir).await?;
    let Some(record) = store.sessions().await?.into_iter().find(|record| record.session_id == input.session_id) else {
        return Ok(HookReply::default());
    };
    let actor = crate::server::load_actor(data_dir)?;
    let mut core = Core::for_session(store, actor, data_dir, record).await?;
    core.capture(event, &input).await
}

/// Waits for this process's turn at writing, for at most `TURN_WAIT`.
pub(crate) async fn turn_in_time(lock: &CaptureLock) -> Result<Turn<'_>, CoreError> {
    Ok(tokio::time::timeout(TURN_WAIT, lock.turn()).await.map_err(|_| CoreError::LockTimeout)??)
}

/// Tools that change only the files their input names.
const FILE_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];
/// Tools that change no file.
const READING_TOOLS: [&str; 8] = ["Read", "Grep", "Glob", "LS", "NotebookRead", "WebFetch", "WebSearch", "TodoWrite"];

/// What a hook compares with the checkout's recorded disk state.
#[derive(Debug, PartialEq, Eq)]
enum Scan {
    Nothing,
    /// As the tool named them: absolute, or relative to the agent's cwd.
    Paths(Vec<PathBuf>),
    Checkout,
}

impl Core {
    /// Captures what `event` of the session in `input` marks, and returns
    /// what the agent should hear.
    pub(crate) async fn capture(&mut self, event: &str, input: &HookInput) -> Result<HookReply, CoreError> {
        let session_id = input.session_id.as_str();
        let Some(session) = self.session(session_id) else {
            return Ok(HookReply::default());
        };
        let (thread, checkout, hooked, cwd) = (
            session.record.thread,
            session.record.checkout_id,
            session.agent.capture() == Capture::Hooks,
            session.agent_cwd.clone(),
        );
        // Agents without hooks mark their turns in their transcripts.
        let mark = hooked.then(|| mark_of(event, input)).flatten();
        let compacted = event == "SessionStart" && input.source.as_deref() == Some("compact");
        if hooked && mark.is_none() && !compacted {
            return Ok(HookReply::default());
        }
        self.tail_session(session_id).await?;
        if compacted && hooked {
            let context = format!("{} This session's thread is {thread}.", silt_mcp::INSTRUCTIONS);
            return Ok(HookReply { additional_context: Some(context) });
        }
        if let Some(mark) = mark {
            if self.checkouts.contains_key(&checkout) {
                match scan_for(event, input) {
                    Scan::Nothing => {}
                    Scan::Paths(paths) => self.ingest_paths(checkout, absolute(&cwd, paths)).await?,
                    Scan::Checkout => self.rescan(checkout).await?,
                }
            }
            self.record_turn(session_id, mark).await?;
        }
        let additional_context = if event == "UserPromptSubmit" { self.prompt_context(thread).await } else { None };
        Ok(HookReply { additional_context })
    }
}

/// What to compare at `event`: after a file tool the files it named, after
/// a tool that changes none nothing, after any other tool and at the turn's
/// start and end the whole checkout. A tool starting has nothing of its
/// own to find yet.
fn scan_for(event: &str, input: &HookInput) -> Scan {
    match event {
        "UserPromptSubmit" | "Stop" => Scan::Checkout,
        "PostToolUse" => {
            let tool = input.tool_name.as_deref().unwrap_or_default();
            if READING_TOOLS.contains(&tool) {
                return Scan::Nothing;
            }
            let named = ["file_path", "notebook_path"]
                .iter()
                .find_map(|key| input.tool_input.as_ref()?.get(key)?.as_str().map(PathBuf::from));
            match named {
                Some(path) if FILE_TOOLS.contains(&tool) => Scan::Paths(vec![path]),
                _ => Scan::Checkout,
            }
        }
        _ => Scan::Nothing,
    }
}

fn absolute(cwd: &Path, paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths.into_iter().map(|path| if path.is_absolute() { path } else { cwd.join(path) }).collect()
}

/// The turn mark a hook stands for.
fn mark_of(event: &str, input: &HookInput) -> Option<TurnMark> {
    let tool_use_id = || input.tool_use_id.clone().unwrap_or_default();
    Some(match event {
        "UserPromptSubmit" => TurnMark::PromptSubmitted,
        "PreToolUse" => TurnMark::ToolStarted { tool_use_id: tool_use_id(), tool: input.tool_name.clone().unwrap_or_default() },
        "PostToolUse" => TurnMark::ToolFinished { tool_use_id: tool_use_id() },
        "Stop" => TurnMark::Stopped,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn tool(name: &str, input: serde_json::Value) -> HookInput {
        serde_json::from_value(json!({
            "session_id": "s", "hook_event_name": "PostToolUse", "tool_name": name, "tool_use_id": "t", "tool_input": input
        }))
        .expect("parses")
    }

    #[test]
    fn a_hook_compares_what_could_have_changed() {
        let edit = tool("Edit", json!({ "file_path": "/repo/src/main.rs", "old_string": "a", "new_string": "b" }));
        assert_eq!(scan_for("PostToolUse", &edit), Scan::Paths(vec!["/repo/src/main.rs".into()]));
        let notebook = tool("NotebookEdit", json!({ "notebook_path": "nb.ipynb" }));
        assert_eq!(scan_for("PostToolUse", &notebook), Scan::Paths(vec!["nb.ipynb".into()]));
        assert_eq!(scan_for("PostToolUse", &tool("Read", json!({ "file_path": "/repo/a" }))), Scan::Nothing);
        assert_eq!(scan_for("PostToolUse", &tool("Bash", json!({ "command": "make" }))), Scan::Checkout);
        assert_eq!(scan_for("PostToolUse", &tool("Edit", json!({}))), Scan::Checkout, "a file tool naming no file");
        assert_eq!(scan_for("PostToolUse", &tool("mcp__x__write", json!({ "file_path": "a" }))), Scan::Checkout);
        assert_eq!(scan_for("PreToolUse", &edit), Scan::Nothing);
        assert_eq!(scan_for("Stop", &edit), Scan::Checkout);
        assert_eq!(scan_for("UserPromptSubmit", &edit), Scan::Checkout);
        assert_eq!(absolute(Path::new("/repo/sub"), vec!["a".into(), "/b".into()]), [PathBuf::from("/repo/sub/a"), "/b".into()]);
    }
}
