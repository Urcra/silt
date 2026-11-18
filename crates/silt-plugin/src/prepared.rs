//! A call as its plugin prepared it: the action, its fields, the card and
//! the effects. An approval is bound to its hash, and the call runs only as
//! prepared.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::card::Card;
use crate::effect::Effect;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Prepared {
    pub plugin: String,
    pub version: String,
    pub tool: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    /// The fields rules match on, as the action declares them.
    pub fields: BTreeMap<String, String>,
    /// The agent's input, as the plugin took it.
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub input: Value,
    pub card: Card,
    pub effects: Vec<Effect>,
}

impl Prepared {
    /// What an approval is bound to: the same call, whatever order its
    /// fields were written in.
    pub fn hash(&self) -> String {
        let value = serde_json::to_value(self).unwrap_or(Value::Null);
        blake3::hash(canonical_json(&value).as_bytes()).to_hex().to_string()
    }

    /// Its effects are all ones Silt can check.
    pub fn bound(&self) -> bool {
        !self.effects.iter().any(Effect::is_opaque)
    }
}

/// JSON with every object's keys sorted, at every depth, and no spacing:
/// one text for one value, whatever map serde_json was built with.
pub fn canonical_json(value: &Value) -> String {
    let mut out = String::new();
    write(value, &mut out);
    out
}

fn write(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                write(&map[key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::card::Block;

    fn prepared(input: Value) -> Prepared {
        Prepared {
            plugin: "silt.github".into(),
            version: "1.0.0".into(),
            tool: "push".into(),
            action: "push".into(),
            environment: Some("acme".into()),
            fields: BTreeMap::from([("branch".into(), "ada/x".into()), ("repo".into(), "acme/api".into())]),
            input,
            card: Card { title: "Push".into(), blocks: vec![Block::Fields { pairs: vec![("branch".into(), "ada/x".into())] }] },
            effects: vec![Effect::GitPush {
                remote: "acme/api".into(),
                reference: "refs/heads/ada/x".into(),
                old: "a".into(),
                new: "b".into(),
            }],
        }
    }

    #[test]
    fn the_hash_ignores_key_order_and_sees_every_change() {
        let one = prepared(serde_json::from_str(r#"{"repo":"acme/api","branch":"ada/x","nested":{"b":1,"a":2}}"#).unwrap());
        let two = prepared(serde_json::from_str(r#"{"nested":{"a":2,"b":1},"branch":"ada/x","repo":"acme/api"}"#).unwrap());
        assert_eq!(one.hash(), two.hash());
        let mut other = one.clone();
        other.effects = vec![Effect::GitPush {
            remote: "acme/api".into(),
            reference: "refs/heads/ada/x".into(),
            old: "a".into(),
            new: "c".into(),
        }];
        assert_ne!(one.hash(), other.hash());
        let mut card = one.clone();
        card.card.title = "Push!".into();
        assert_ne!(one.hash(), card.hash(), "the card is part of what's approved");
    }

    #[test]
    fn canonical_json_sorts_at_every_depth() {
        assert_eq!(canonical_json(&json!({"b": [ {"d": 1, "c": 2} ], "a": "x"})), r#"{"a":"x","b":[{"c":2,"d":1}]}"#);
    }
}
