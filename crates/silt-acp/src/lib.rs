//! Runs coding agents over the Agent Client Protocol for the web UI
//! (ADR 0005, ADR 0014): finding them in the ACP registry, starting one in
//! a checkout, and relaying its session to the daemon over channels.

mod host;
mod modes;
mod processes;
mod prompt;
pub mod registry;
mod sign_in;
mod steering;
mod v1;
mod v2;

/// Claude Code's own artifact tools, which publish to claude.ai. A session
/// Silt starts has them off, so an agent's pages are Silt's
/// (`silt_artifact`), kept with the thread (ADR 0068).
pub const CLAUDE_TOOLS_OFF: &[&str] = &["Artifact", "ArtifactData", "ArtifactComments"];

/// What every session Silt starts tells its agent: Claude's adapter reads
/// `claudeCode.options` as Claude Code's own options; other agents ignore it.
/// Its `settings` are Claude Code's flag settings, which keep its memory in
/// `memory` when given (ADR 0135).
fn session_meta(memory: Option<&std::path::Path>) -> serde_json::Map<String, serde_json::Value> {
    let mut meta = serde_json::json!({"claudeCode": {"options": {"disallowedTools": CLAUDE_TOOLS_OFF}}});
    if let Some(memory) = memory {
        meta["claudeCode"]["options"]["settings"] = serde_json::json!({"autoMemoryDirectory": memory.to_string_lossy()});
    }
    meta.as_object().cloned().unwrap_or_default()
}

pub use host::{
    AgentHandle, Command, Event, HostError, McpStart, Permission, Question, QuestionChoice, QuestionField, Start, StopReason,
    TaskNews, Update, start,
};
pub use modes::{Choice, Modes, Setting, SlashCommand, built_in, plan_mode, presents_plans};
pub use prompt::{Accepts, Attachment};
pub use sign_in::{SignInHow, SignInMethod};
pub use steering::Steered;

#[cfg(test)]
mod meta_tests {
    #[test]
    fn a_session_turns_claude_codes_own_artifact_tools_off() {
        let meta = super::session_meta(None);
        let off = &meta["claudeCode"]["options"]["disallowedTools"];
        assert_eq!(off, &serde_json::json!(["Artifact", "ArtifactData", "ArtifactComments"]));
        assert_eq!(meta["claudeCode"]["options"].get("settings"), None);
    }

    #[test]
    fn a_session_keeps_claude_codes_memory_where_silt_says() {
        let meta = super::session_meta(Some(std::path::Path::new("/c/projects/-repo/memory")));
        let settings = &meta["claudeCode"]["options"]["settings"];
        assert_eq!(settings, &serde_json::json!({"autoMemoryDirectory": "/c/projects/-repo/memory"}));
    }
}
