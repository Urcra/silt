//! The canonical form of a plugin call an approval is bound to (ADR 0093):
//! the person approves its hash, and only the copy with that hash runs.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Canonical {
    pub plugin: String,
    pub tool: String,
    /// Such as `production`.
    pub environment: String,
    /// Where it goes: an endpoint, a host, a cluster.
    pub target: String,
    /// Everything it sends, as it will be sent.
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub input: serde_json::Value,
}

impl Canonical {
    /// Its hash: the same request always gives the same one, and any change
    /// to what it sends gives another.
    pub fn hash(&self) -> String {
        // serde_json's maps keep their keys sorted, so the bytes are canonical.
        let bytes = serde_json::to_vec(self).unwrap_or_default();
        blake3::hash(&bytes).to_hex().to_string()
    }
}

/// `text` as a person must see it to approve it: control, bidirectional and
/// zero-width characters, which can make text read as something it isn't,
/// shown by their code point.
pub fn shown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        let hidden = (character.is_control() && !matches!(character, '\n' | '\t'))
            || matches!(character, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}' | '\u{00AD}');
        if hidden {
            out.push_str(&format!("⟨U+{:04X}⟩", u32::from(character)));
        } else {
            out.push(character);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_follows_what_is_sent() {
        let call = |cars: &[&str]| Canonical {
            plugin: "fleet-api".into(),
            tool: "mutate".into(),
            environment: "production".into(),
            target: "https://api.fleet.example/graphql".into(),
            input: serde_json::json!({"query": "mutation { setOta(on: true) }", "variables": {"cars": cars}}),
        };
        assert_eq!(call(&["car-1"]).hash(), call(&["car-1"]).hash());
        assert_ne!(call(&["car-1"]).hash(), call(&["car-1", "car-2"]).hash());
    }

    #[test]
    fn what_hides_meaning_is_shown() {
        assert_eq!(shown("rm -rf /tmp/x\u{202E}txt.exe"), "rm -rf /tmp/x⟨U+202E⟩txt.exe");
        assert_eq!(shown("echo hi\u{1b}[2K"), "echo hi⟨U+001B⟩[2K");
        assert_eq!(shown("plain\nlines"), "plain\nlines");
    }
}
