//! Permission presets (ADR 0022): the daemon answers an agent's permission
//! requests by itself when the thread's preset says how, and leaves the rest
//! to the person.

use crate::protocol::Preset;

/// Tools that change nothing outside the checkout's files, or nothing.
const QUIET_TOOLS: [&str; 3] = ["edit", "read", "search"];

/// Tools that change nothing at all, which a plan needs to look around.
const LOOKING_TOOLS: [&str; 2] = ["read", "search"];

/// The Silt tool a call is, by its title as agents give it: `silt_ask`,
/// `mcp__silt__silt_ask` or `silt_ask (silt MCP Server)`.
pub(crate) fn silt_tool(title: &str) -> Option<&str> {
    let name = title.split_whitespace().next()?;
    let name = name.rsplit("__").next()?;
    name.starts_with("silt_").then_some(name)
}

/// What kind of tool one of Silt's own counts as: asking another agent is
/// a command (ADR 0117), and taking its changes an edit.
pub(crate) fn silt_tool_kind(name: &str) -> Option<&'static str> {
    match name {
        "silt_ask" => Some("execute"),
        "silt_ask_take" => Some("edit"),
        "silt_ask_stop" | "silt_ask_wait" | "silt_askable" => Some("read"),
        _ => None,
    }
}

/// The option to answer a request with, when `preset` answers it: the
/// agent's "allow once", else its "allow always".
#[cfg(test)]
pub(crate) fn decide<'a>(preset: Preset, tool_kind: Option<&str>, option_kinds: &'a [(String, String)]) -> Option<&'a str> {
    decide_with(preset, tool_kind, false, option_kinds)
}

/// As [`decide`], where `asks` says the call asks another agent: a command
/// Auto lets through, as it does what the agent's own check finds safe.
pub(crate) fn decide_with<'a>(
    preset: Preset,
    tool_kind: Option<&str>,
    asks: bool,
    option_kinds: &'a [(String, String)],
) -> Option<&'a str> {
    let allows = match preset {
        Preset::Auto if asks => true,
        Preset::AskForEverything => false,
        Preset::Plan => tool_kind.is_some_and(|kind| LOOKING_TOOLS.contains(&kind)),
        Preset::FullAccess => true,
        // What an agent's auto mode still asks about is what its own check
        // found risky, so only what Ask before commands lets through goes.
        Preset::AskBeforeCommands | Preset::Auto => tool_kind.is_some_and(|kind| QUIET_TOOLS.contains(&kind)),
    };
    if !allows {
        return None;
    }
    ["allow_once", "allow_always"]
        .iter()
        .find_map(|wanted| option_kinds.iter().find(|(_, kind)| kind == wanted).map(|(id, _)| id.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> Vec<(String, String)> {
        [("yes", "allow_once"), ("always", "allow_always"), ("no", "reject_once")]
            .map(|(id, kind)| (id.to_owned(), kind.to_owned()))
            .to_vec()
    }

    #[test]
    fn edits_go_through_by_default_and_commands_ask() {
        let options = options();
        assert_eq!(decide(Preset::AskBeforeCommands, Some("edit"), &options), Some("yes"));
        assert_eq!(decide(Preset::AskBeforeCommands, Some("read"), &options), Some("yes"));
        assert_eq!(decide(Preset::AskBeforeCommands, Some("execute"), &options), None);
        assert_eq!(decide(Preset::AskBeforeCommands, None, &options), None, "an unnamed tool asks");
    }

    #[test]
    fn asking_another_agent_is_a_command_that_auto_lets_through() {
        let options = options();
        assert_eq!(silt_tool("mcp__silt__silt_ask"), Some("silt_ask"));
        assert_eq!(silt_tool("silt_ask_take (silt MCP Server)"), Some("silt_ask_take"));
        assert_eq!(silt_tool("Bash"), None);
        assert_eq!(silt_tool_kind("silt_ask"), Some("execute"));
        assert_eq!(decide_with(Preset::AskBeforeCommands, Some("execute"), true, &options), None, "Edits asks the person");
        assert_eq!(decide_with(Preset::Plan, Some("execute"), true, &options), None);
        assert_eq!(decide_with(Preset::Auto, Some("execute"), true, &options), Some("yes"));
        assert_eq!(decide_with(Preset::FullAccess, Some("execute"), true, &options), Some("yes"));
        assert_eq!(decide_with(Preset::Auto, Some("execute"), false, &options), None, "any other command Auto still asks about");
    }

    #[test]
    fn the_other_presets_ask_for_everything_or_nothing() {
        let options = options();
        assert_eq!(decide(Preset::AskForEverything, Some("edit"), &options), None);
        assert_eq!(decide(Preset::Plan, Some("read"), &options), Some("yes"), "a plan looks around");
        assert_eq!(decide(Preset::Plan, Some("edit"), &options), None, "a plan asks before it edits");
        assert_eq!(decide(Preset::FullAccess, Some("execute"), &options), Some("yes"));
        assert_eq!(decide(Preset::Auto, Some("edit"), &options), Some("yes"));
        assert_eq!(decide(Preset::Auto, Some("execute"), &options), None, "what auto mode asks about, the person answers");
        let always_only = vec![("a".to_owned(), "allow_always".to_owned()), ("n".to_owned(), "reject_once".to_owned())];
        assert_eq!(decide(Preset::FullAccess, Some("execute"), &always_only), Some("a"));
        let reject_only = vec![("n".to_owned(), "reject_once".to_owned())];
        assert_eq!(decide(Preset::FullAccess, Some("execute"), &reject_only), None, "nothing to allow with");
    }
}
