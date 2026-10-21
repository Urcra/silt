//! Permission presets (ADR 0022): the daemon answers an agent's permission
//! requests by itself when the thread's preset says how, and leaves the rest
//! to the person.

use crate::protocol::Preset;

/// Tools that change nothing outside the checkout's files, or nothing.
const QUIET_TOOLS: [&str; 3] = ["edit", "read", "search"];

/// The option to answer a request with, when `preset` answers it: the
/// agent's "allow once", else its "allow always".
pub(crate) fn decide<'a>(preset: Preset, tool_kind: Option<&str>, option_kinds: &'a [(String, String)]) -> Option<&'a str> {
    let allows = match preset {
        Preset::AskForEverything => false,
        Preset::FullAccess => true,
        Preset::AskBeforeCommands => tool_kind.is_some_and(|kind| QUIET_TOOLS.contains(&kind)),
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
    fn the_other_presets_ask_for_everything_or_nothing() {
        let options = options();
        assert_eq!(decide(Preset::AskForEverything, Some("edit"), &options), None);
        assert_eq!(decide(Preset::FullAccess, Some("execute"), &options), Some("yes"));
        let always_only = vec![("a".to_owned(), "allow_always".to_owned()), ("n".to_owned(), "reject_once".to_owned())];
        assert_eq!(decide(Preset::FullAccess, Some("execute"), &always_only), Some("a"));
        let reject_only = vec![("n".to_owned(), "reject_once".to_owned())];
        assert_eq!(decide(Preset::FullAccess, Some("execute"), &reject_only), None, "nothing to allow with");
    }
}
