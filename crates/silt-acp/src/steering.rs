//! Adding to a turn the agent is running, over the `_session/steering`
//! extension request that Codex's and Claude's adapters serve (ADR 0111).
//! Silt asks for `idleBehavior: "promptRequired"`, so a steer that finds no
//! turn running comes back to it, to go through `session/prompt` like any
//! other prompt.

use agent_client_protocol::UntypedMessage;
use serde_json::{Map, Value, json};

const METHOD: &str = "_session/steering";

/// How the agent took a steer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Steered {
    /// It went into the turn running.
    Injected,
    /// No turn was running and the agent started one with it, as one that
    /// ignores `promptRequired` does.
    StartedNewTurn,
    /// No turn was running: it is Silt's to send as a prompt.
    PromptRequired,
    /// The agent couldn't add it.
    Failed,
    /// The agent refused the request, or answered with something else.
    Error(String),
}

/// Whether an `initialize` response's `_meta` says the agent takes
/// `_session/steering`.
pub(crate) fn supported(meta: Option<&Map<String, Value>>) -> bool {
    meta.and_then(|meta| meta.get("steering")?.get("supported")?.as_bool()) == Some(true)
}

/// Whether an ACP v2 agent's capabilities say a prompt sent while it works
/// waits in its own queue and joins the turn at its next step, as Claude
/// Code's does at priority `next`, rather than cancelling the turn.
pub(crate) fn queues_prompts(meta: Option<&Map<String, Value>>) -> bool {
    meta.and_then(|meta| meta.get("claudeCode")?.get("promptQueueing")?.as_bool()) == Some(true)
}

pub(crate) fn request(session: &str, text: &str) -> Result<UntypedMessage, agent_client_protocol::Error> {
    UntypedMessage::new(
        METHOD,
        json!({
            "sessionId": session,
            "prompt": [{"type": "text", "text": text}],
            "_meta": {"steering": {"idleBehavior": "promptRequired"}},
        }),
    )
}

pub(crate) fn outcome(answer: Result<Value, agent_client_protocol::Error>) -> Steered {
    let answer = match answer {
        Ok(answer) => answer,
        Err(error) => return Steered::Error(error.to_string()),
    };
    match answer.get("outcome").and_then(Value::as_str) {
        Some("injected") => Steered::Injected,
        Some("startedNewTurn") => Steered::StartedNewTurn,
        Some("promptRequired") => Steered::PromptRequired,
        Some("failed") => Steered::Failed,
        _ => Steered::Error(format!("the agent answered a steer with {answer}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(value: Value) -> Map<String, Value> {
        value.as_object().cloned().expect("an object")
    }

    #[test]
    fn only_an_agent_that_says_so_takes_steers() {
        assert!(supported(Some(&meta(json!({"steering": {"supported": true}})))));
        assert!(!supported(Some(&meta(json!({"steering": {"supported": false}})))));
        assert!(!supported(Some(&meta(json!({"claudeCode": {"promptQueueing": true}})))));
        assert!(!supported(None));
    }

    #[test]
    fn claude_code_queues_a_prompt_sent_while_it_works() {
        assert!(queues_prompts(Some(&meta(json!({"claudeCode": {"promptQueueing": true}})))));
        assert!(!queues_prompts(Some(&meta(json!({"claudeCode": {}})))));
        assert!(!queues_prompts(Some(&meta(json!({"steering": {"supported": true}})))));
        assert!(!queues_prompts(None));
    }

    #[test]
    fn a_steer_asks_to_get_back_what_finds_no_turn() {
        let sent = request("s-1", "use the other file").expect("a request");
        assert_eq!(sent.method, "_session/steering");
        assert_eq!(
            sent.params,
            json!({
                "sessionId": "s-1",
                "prompt": [{"type": "text", "text": "use the other file"}],
                "_meta": {"steering": {"idleBehavior": "promptRequired"}},
            })
        );
    }

    #[test]
    fn each_outcome_is_read() {
        let read = |answer: Value| outcome(Ok(answer));
        assert_eq!(read(json!({"outcome": "injected"})), Steered::Injected);
        assert_eq!(read(json!({"outcome": "startedNewTurn"})), Steered::StartedNewTurn);
        assert_eq!(read(json!({"outcome": "promptRequired", "reason": "noRunningTurn"})), Steered::PromptRequired);
        assert_eq!(read(json!({"outcome": "failed"})), Steered::Failed);
        assert!(matches!(read(json!({"outcome": "later"})), Steered::Error(_)));
        let refused = outcome(Err(agent_client_protocol::Error::method_not_found()));
        assert!(matches!(&refused, Steered::Error(why) if why.contains("not found")), "{refused:?}");
    }
}
