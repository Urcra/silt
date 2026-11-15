//! Rules for artifacts that both serving modes and the agent's tools apply
//! alike (ADR 0068, ADR 0073).

use serde::{Deserialize, Serialize};

/// The most edits one call takes.
pub const MAX_EDITS: usize = 64;
/// The largest artifact a version keeps, in bytes.
pub const ARTIFACT_BYTES: u64 = 10 << 20;
/// The most of a page's question to `sample`, in bytes.
pub const SAMPLE_BYTES: usize = 256 * 1024;

/// What a page's `sample` call asks, as one message to an agent's print
/// mode: a string as it is, or a conversation ending on the person's turn,
/// each turn marked.
pub fn sample_question(input: &serde_json::Value) -> Result<String, String> {
    use serde_json::Value;
    let question = match input {
        Value::String(text) => text.clone(),
        Value::Array(turns) => {
            let mut said = Vec::new();
            for turn in turns {
                let role = turn.get("role").and_then(Value::as_str).ok_or("each turn has a role")?;
                let content = turn.get("content").and_then(Value::as_str).ok_or("each turn's content is text")?;
                let tag = match role {
                    "user" => "user",
                    "assistant" => "assistant",
                    _ => return Err(format!("{role} isn't a role")),
                };
                said.push((tag, content));
            }
            if said.last().is_none_or(|(role, _)| *role != "user") {
                return Err("a conversation ends on the user's turn".into());
            }
            let earlier: String =
                said[..said.len() - 1].iter().map(|(role, content)| format!("<{role}>\n{content}\n</{role}>\n")).collect();
            let last = said[said.len() - 1].1;
            if earlier.is_empty() {
                last.to_owned()
            } else {
                format!("The conversation so far:\n{earlier}\nAnswer the user's latest message:\n{last}")
            }
        }
        _ => return Err("a question is text or a list of turns".into()),
    };
    if question.trim().is_empty() {
        return Err("there's nothing to ask".into());
    }
    if question.len() > SAMPLE_BYTES {
        return Err(format!("a question is at most {} KiB", SAMPLE_BYTES >> 10));
    }
    Ok(question)
}

/// The longest note on a version, in characters.
pub const NOTE_CHARS: usize = 200;
/// The longest description of an artifact, in characters.
pub const DESCRIPTION_CHARS: usize = 300;

/// The capabilities a page may declare (ADR 0072 and the ADRs after it).
pub const CAPABILITIES: &[&str] =
    &["artifact", "assets", "comments", "db", "dictation", "downloads", "files", "mcp", "room", "sample", "tts", "user"];

/// The capabilities a version declares, by name, each with its config.
pub type Capabilities = std::collections::BTreeMap<String, serde_json::Value>;

/// The largest declaration, as JSON.
const CAPABILITIES_BYTES: usize = 16 * 1024;

/// A declaration checked: known names, each with an object or `true` as
/// its config, `true` read as `{}`.
pub fn checked_capabilities(declared: &serde_json::Map<String, serde_json::Value>) -> Result<Capabilities, String> {
    if serde_json::to_string(declared).map_or(true, |text| text.len() > CAPABILITIES_BYTES) {
        return Err(format!("a capabilities declaration is at most {} KiB", CAPABILITIES_BYTES / 1024));
    }
    let mut checked = Capabilities::new();
    for (name, config) in declared {
        if !CAPABILITIES.contains(&name.as_str()) {
            return Err(format!("{name} isn't a capability; they're {}", CAPABILITIES.join(", ")));
        }
        let config = match config {
            serde_json::Value::Bool(true) => serde_json::json!({}),
            serde_json::Value::Object(_) => config.clone(),
            _ => return Err(format!("{name}'s config is an object, or true")),
        };
        checked.insert(name.clone(), config);
    }
    Ok(checked)
}

/// One change to a version's source: `old` occurs exactly once, and
/// becomes `new`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Edit {
    pub old: String,
    pub new: String,
}

/// Why an edit couldn't be made; nothing was changed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("no edits were given")]
    None,
    #[error("at most {MAX_EDITS} edits go in one call")]
    TooMany,
    #[error("edit {index}: its old text is empty")]
    Empty { index: usize },
    #[error("edit {index}: its old text isn't in the source")]
    Missing { index: usize },
    #[error("edit {index}: its old text occurs {count} times; give more of it so it occurs once")]
    Ambiguous { index: usize, count: usize },
}

/// `source` with `edits` made in order, each on the result of the ones
/// before it, all or none. Edits are numbered from 1 in errors.
pub fn apply_edits(source: &str, edits: &[Edit]) -> Result<String, EditError> {
    if edits.is_empty() {
        return Err(EditError::None);
    }
    if edits.len() > MAX_EDITS {
        return Err(EditError::TooMany);
    }
    let mut text = source.to_owned();
    for (at, edit) in edits.iter().enumerate() {
        let index = at + 1;
        if edit.old.is_empty() {
            return Err(EditError::Empty { index });
        }
        let count = text.matches(edit.old.as_str()).count();
        match count {
            0 => return Err(EditError::Missing { index }),
            1 => text = text.replacen(edit.old.as_str(), &edit.new, 1),
            count => return Err(EditError::Ambiguous { index, count }),
        }
    }
    Ok(text)
}

/// A note or description trimmed, empty as none, refused past `limit`
/// characters.
pub fn short_text(text: Option<&str>, limit: usize, what: &str) -> Result<Option<String>, String> {
    let Some(text) = text.map(str::trim).filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    if text.chars().count() > limit {
        return Err(format!("{what} is at most {limit} characters"));
    }
    Ok(Some(text.to_owned()))
}

#[cfg(test)]
mod sample_tests {
    use super::sample_question;
    use serde_json::json;

    #[test]
    fn a_question_is_text_or_turns_ending_on_the_users() {
        assert_eq!(sample_question(&json!("Summarise this")).as_deref(), Ok("Summarise this"));
        let turns = json!([{"role": "user", "content": "Hi"}, {"role": "assistant", "content": "Hello"}, {"role": "user", "content": "And?"}]);
        let asked = sample_question(&turns).expect("turns");
        assert!(asked.contains("<assistant>\nHello\n</assistant>") && asked.ends_with("And?"));
        assert!(sample_question(&json!([{"role": "assistant", "content": "Hello"}])).is_err());
        assert!(sample_question(&json!([{"role": "system", "content": "x"}])).is_err());
        assert!(sample_question(&json!("  ")).is_err());
        assert!(sample_question(&json!(3)).is_err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(old: &str, new: &str) -> Edit {
        Edit { old: old.into(), new: new.into() }
    }

    #[test]
    fn edits_apply_in_order_all_or_none() {
        let source = "<h1>Rescan</h1><p>v1</p>";
        assert_eq!(
            apply_edits(source, &[edit("v1", "v2"), edit("<h1>Rescan", "<h1>Rescan benchmark")]).as_deref(),
            Ok("<h1>Rescan benchmark</h1><p>v2</p>")
        );
        assert_eq!(
            apply_edits(source, &[edit("v1", "v2"), edit("v1", "v3")]),
            Err(EditError::Missing { index: 2 }),
            "an edit sees the ones before it"
        );
        assert_eq!(apply_edits("a a", &[edit("a", "b")]), Err(EditError::Ambiguous { index: 1, count: 2 }));
        assert_eq!(apply_edits(source, &[edit("", "x")]), Err(EditError::Empty { index: 1 }));
        assert_eq!(apply_edits(source, &[]), Err(EditError::None));
        assert_eq!(apply_edits(source, &vec![edit("v1", "v1"); MAX_EDITS + 1]), Err(EditError::TooMany));
        assert_eq!(apply_edits(source, &[edit("</p>", "")]).as_deref(), Ok("<h1>Rescan</h1><p>v1"), "an empty new text deletes");
    }

    #[test]
    fn a_declaration_names_known_capabilities_with_objects() {
        let declared = serde_json::json!({"db": {"rules": []}, "downloads": true});
        let checked = checked_capabilities(declared.as_object().expect("an object")).expect("checked");
        assert_eq!(checked["downloads"], serde_json::json!({}));
        assert_eq!(checked["db"], serde_json::json!({"rules": []}));
        assert!(checked_capabilities(serde_json::json!({"camera": {}}).as_object().expect("an object")).is_err());
        assert!(checked_capabilities(serde_json::json!({"db": 3}).as_object().expect("an object")).is_err());
    }

    #[test]
    fn notes_are_trimmed_and_bounded() {
        assert_eq!(short_text(Some("  Darker axis  "), NOTE_CHARS, "a note"), Ok(Some("Darker axis".into())));
        assert_eq!(short_text(Some("   "), NOTE_CHARS, "a note"), Ok(None));
        assert_eq!(short_text(None, NOTE_CHARS, "a note"), Ok(None));
        assert!(short_text(Some(&"x".repeat(NOTE_CHARS + 1)), NOTE_CHARS, "a note").is_err());
    }
}
