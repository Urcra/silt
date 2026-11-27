//! `_session/steering`, as Codex's and Claude's adapters serve it, behind
//! `FAKE_ACP_STEERING`: unset, the agent neither advertises nor answers it;
//! `injected` (or `startedNewTurn`) adds a steer to the turn running and
//! answers so, or `promptRequired` when none runs; `promptRequired`,
//! `failed` and `error` always answer that, the last with a JSON-RPC error.
//!
//! A prompt saying `Long` runs a command that takes a few seconds, as a
//! build would. A steer added meanwhile is heard while it runs, and it runs
//! to its end; a cancel kills it.
//!
//! With `FAKE_ACP_PROMPT_QUEUEING` set, the v2 agent advertises
//! `claudeCode.promptQueueing`, as Claude's adapter does, and a prompt sent
//! while the command of a `Long` prompt runs waits until it ends: then it's
//! echoed as a `user_message`, answered, and heard in the same turn. A
//! cancel before that answers it `-32800`, as the adapter does.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

/// How long the command of a `Long` prompt runs.
pub(crate) const LONG: Duration = Duration::from_secs(3);

static RUNNING: AtomicBool = AtomicBool::new(false);
static CANCELLED: AtomicBool = AtomicBool::new(false);
static HEARD: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn wanted() -> Option<String> {
    std::env::var("FAKE_ACP_STEERING").ok().filter(|outcome| !outcome.is_empty())
}

/// What `initialize`'s `_meta` says.
pub(crate) fn meta() -> Option<Value> {
    wanted().map(|_| json!({"steering": {"supported": true}}))
}

/// The answer to an extension request: `_session/steering` when steering is
/// on, else none, which the agent refuses.
pub(crate) fn answer(method: &str, params: &Value) -> agent_client_protocol::Result<Value> {
    let Some(wanted) = wanted().filter(|_| method == "_session/steering") else {
        return Err(agent_client_protocol::Error::method_not_found());
    };
    let outcome = match wanted.as_str() {
        "injected" | "startedNewTurn" if RUNNING.load(Ordering::SeqCst) => {
            let text = params["prompt"].as_array().into_iter().flatten().filter_map(|block| block["text"].as_str()).collect();
            HEARD.lock().expect("lock").push(text);
            wanted
        }
        "injected" | "startedNewTurn" | "promptRequired" => "promptRequired".to_owned(),
        "failed" => "failed".to_owned(),
        _ => return Err(agent_client_protocol::Error::internal_error().data("the steer went nowhere")),
    };
    Ok(json!({"outcome": outcome}))
}

/// Whether the command of a `Long` prompt runs.
pub(crate) fn running() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

/// Whether a prompt sent while the agent works waits for its next step, as
/// Claude Code queues one, behind `FAKE_ACP_PROMPT_QUEUEING` (v2 only).
pub(crate) fn queues_prompts() -> bool {
    std::env::var_os("FAKE_ACP_PROMPT_QUEUEING").is_some_and(|set| !set.is_empty())
}

pub(crate) fn cancel() {
    CANCELLED.store(true, Ordering::SeqCst);
}

/// How the command of a `Long` prompt ran.
pub(crate) enum Ran {
    Ended,
    Killed,
}

/// Runs the command of a `Long` prompt: `heard` is called for each steer as
/// it comes in.
pub(crate) async fn long(mut heard: impl FnMut(&str) -> agent_client_protocol::Result<()>) -> agent_client_protocol::Result<Ran> {
    CANCELLED.store(false, Ordering::SeqCst);
    RUNNING.store(true, Ordering::SeqCst);
    let started = std::time::Instant::now();
    let ran = loop {
        if CANCELLED.load(Ordering::SeqCst) {
            break Ran::Killed;
        }
        let new: Vec<String> = std::mem::take(&mut *HEARD.lock().expect("lock"));
        for steer in new {
            heard(&steer)?;
        }
        if started.elapsed() >= LONG {
            break Ran::Ended;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    RUNNING.store(false, Ordering::SeqCst);
    Ok(ran)
}
