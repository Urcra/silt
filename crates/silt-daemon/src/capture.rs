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
use crate::core::{Ahead, Core, CoreError, now_ms};
use crate::protocol::HookReply;

/// How long a hook waits for its turn at writing before it gives up, so a
/// daemon stopped while writing can't hang the agent.
const TURN_WAIT: Duration = Duration::from_secs(10);

/// What a hook captured in its own process.
#[derive(Debug, Default)]
pub struct Captured {
    pub reply: HookReply,
    /// The session's thread, when Silt records the session.
    pub thread: Option<String>,
    /// Another writer kept its turn too long; the hook was left for the
    /// next writer to capture.
    pub deferred: bool,
}

/// Runs a hook's capture step in this process, without the daemon, for the
/// data in `data_dir`.
pub async fn capture_here(data_dir: &Path, event: &str, input: serde_json::Value) -> Result<Captured, CoreError> {
    let parsed: HookInput = serde_json::from_value(input.clone()).map_err(CoreError::HookInput)?;
    let lock = CaptureLock::open(data_dir)?;
    let _turn = match turn_in_time(&lock).await {
        Err(CoreError::LockTimeout) => {
            crate::spool::spool(data_dir, event, &input)?;
            return Ok(Captured { deferred: true, ..Captured::default() });
        }
        turn => turn?,
    };
    let store = Store::open(data_dir).await?;
    let Some(record) = store.sessions().await?.into_iter().find(|record| record.session_id == parsed.session_id) else {
        return Ok(Captured::default());
    };
    let thread = record.thread.to_string();
    let actor = crate::server::load_actor(data_dir)?;
    let scan = scan_for(event, &parsed);
    let ahead = Ahead { compares: scan == Scan::Checkout, records: scan != Scan::Nothing };
    let mut core = Core::for_session(store, actor, data_dir, record, ahead).await?;
    core.capture_spooled().await?;
    let reply = core.capture(event, &parsed).await?;
    // A long family takes a while to free, and the agent is waiting on the
    // hook: the hook's process exits without waiting for it.
    let families = std::mem::replace(&mut core.families, crate::families::Families::new(None));
    std::thread::spawn(move || drop(families));
    let store = core.store.clone();
    drop(core);
    store.close().await;
    Ok(Captured { reply, thread: Some(thread), deferred: false })
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
        let (thread, checkout, hooked, cwd, agent) = (
            session.record.thread,
            session.record.checkout_id,
            session.agent.capture() == Capture::Hooks,
            session.agent_cwd.clone(),
            session.record.agent.clone(),
        );
        // Whether the agent is still around, as its peers are told (ADR 0090).
        match event {
            "SessionEnd" => {
                // `/clear` starts the same agent over.
                if input.reason.as_deref() != Some("clear") {
                    self.store.session_ended(&agent, session_id, now_ms()).await?;
                }
                return Ok(HookReply::default());
            }
            "SessionStart" => self.store.session_revived(&agent, session_id, now_ms()).await?,
            _ => self.store.session_seen(&agent, session_id, now_ms()).await?,
        }
        match event {
            "UserPromptSubmit" => {
                self.store.session_idle(&agent, session_id, false).await?;
                if let Some(doing) = input.prompt.as_deref().and_then(crate::peers::doing_of) {
                    self.store.session_doing(&agent, session_id, &doing).await?;
                }
            }
            "Stop" => {
                self.store.session_idle(&agent, session_id, true).await?;
                // A terminal's agent granted folders this turn is restarted
                // with them (ADR 0102).
                crate::folders::turn_ended(&self.data_dir, &thread.to_string(), now_ms())?;
            }
            _ => {}
        }
        let source = input.source.as_deref();
        if event == "SessionStart" && hooked && matches!(source, Some("startup" | "resume" | "clear")) {
            let greeting = self.greet(thread, source != Some("clear")).await?;
            return Ok(HookReply { additional_context: greeting });
        }
        // Agents without hooks mark their turns in their transcripts.
        let mark = hooked.then(|| mark_of(event, input)).flatten();
        let compacted = event == "SessionStart" && source == Some("compact");
        if hooked && mark.is_none() && !compacted {
            return Ok(HookReply::default());
        }
        self.tail_session(session_id).await?;
        if compacted && hooked {
            let mut context = format!("{} This session's thread is {thread}.", silt_mcp::INSTRUCTIONS);
            if let Some(peers) = self.greet(thread, false).await? {
                context = format!("{context}\n\n{peers}");
            }
            return Ok(HookReply { additional_context: Some(context) });
        }
        if let Some(mark) = mark {
            if self.checkouts.contains_key(&checkout) {
                // Other agents working in the same clone take in the same
                // edits, each into its own thread (ADR 0089).
                let rows: Vec<_> = std::iter::once(checkout).chain(self.sharing(checkout)).collect();
                match scan_for(event, input) {
                    Scan::Nothing => {}
                    Scan::Paths(paths) => {
                        let paths = absolute(&cwd, paths);
                        self.deciding = Some(session_id.to_owned());
                        let ingested = async {
                            for row in rows {
                                self.ingest_paths(row, paths.clone()).await?;
                            }
                            Ok::<_, CoreError>(())
                        }
                        .await;
                        self.deciding = None;
                        ingested?;
                    }
                    Scan::Checkout => {
                        for row in rows {
                            self.rescan(row).await?;
                        }
                    }
                }
            }
            self.record_turn(session_id, mark).await?;
        }
        // Other agents' messages reach a busy agent after its next tool call.
        let additional_context = match event {
            "UserPromptSubmit" => {
                let parts: Vec<String> =
                    [self.prompt_context(thread).await, self.take_messages(thread).await?].into_iter().flatten().collect();
                (!parts.is_empty()).then(|| parts.join("\n\n"))
            }
            "PostToolUse" => self.take_messages(thread).await?,
            _ => None,
        };
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
