//! A plugin's settings, as its manifest declares them, and the value each
//! has: an org's, locked, over the person's, over the manifest's default.
//! A secret's value is never here; it's in the keyring, and only whether
//! one is kept is.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::manifest::{SettingKind, SettingSpec};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case", tag = "from", content = "org")]
pub enum SettingSource {
    Default,
    Yours,
    /// An org's policy, which the person can't change.
    Org(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SettingValue {
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub value: Value,
    pub source: SettingSource,
}

/// Why `value` isn't a value of `spec`, if it isn't.
pub fn fits(spec: &SettingSpec, value: &Value) -> Result<(), String> {
    let ok = match spec.kind {
        SettingKind::String | SettingKind::Path => value.is_string(),
        SettingKind::Number => value.is_number(),
        SettingKind::Boolean => value.is_boolean(),
        SettingKind::Enum => value.as_str().is_some_and(|choice| spec.options.iter().any(|option| option == choice)),
        SettingKind::List => value.as_array().is_some_and(|items| items.iter().all(Value::is_string)),
        SettingKind::Secret => false,
    };
    if ok {
        return Ok(());
    }
    Err(match spec.kind {
        SettingKind::Enum => format!("is one of {}", spec.options.join(", ")),
        SettingKind::List => "is a list of text".into(),
        SettingKind::Secret => "is a secret, kept in the keyring rather than here".into(),
        SettingKind::Number => "is a number".into(),
        SettingKind::Boolean => "is true or false".into(),
        SettingKind::String | SettingKind::Path => "is text".into(),
    })
}

/// The value `spec` has, and where it comes from.
pub fn resolve_setting(spec: &SettingSpec, yours: Option<&Value>, org: Option<(&str, &Value)>) -> Option<SettingValue> {
    if let Some((name, value)) = org
        && fits(spec, value).is_ok()
    {
        return Some(SettingValue { value: value.clone(), source: SettingSource::Org(name.to_owned()) });
    }
    if let Some(value) = yours
        && fits(spec, value).is_ok()
    {
        return Some(SettingValue { value: value.clone(), source: SettingSource::Yours });
    }
    spec.default.as_ref().map(|value| SettingValue { value: value.clone(), source: SettingSource::Default })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::manifest::SettingScope;

    fn spec(kind: SettingKind, default: Option<Value>, options: &[&str]) -> SettingSpec {
        SettingSpec {
            key: "k".into(),
            label: "K".into(),
            description: "d".into(),
            kind,
            scope: SettingScope::Plugin,
            default,
            options: options.iter().map(|option| (*option).to_owned()).collect(),
        }
    }

    #[test]
    fn values_fit_their_kind() {
        assert!(fits(&spec(SettingKind::Enum, None, &["a", "b"]), &json!("a")).is_ok());
        assert!(fits(&spec(SettingKind::Enum, None, &["a", "b"]), &json!("c")).is_err());
        assert!(fits(&spec(SettingKind::List, None, &[]), &json!(["x", "y"])).is_ok());
        assert!(fits(&spec(SettingKind::List, None, &[]), &json!(["x", 1])).is_err());
        assert!(fits(&spec(SettingKind::Secret, None, &[]), &json!("hunter2")).is_err(), "never stored as a setting");
    }

    #[test]
    fn an_org_wins_then_yours_then_the_default() {
        let canary = spec(SettingKind::Number, Some(json!(10)), &[]);
        assert_eq!(resolve_setting(&canary, None, None), Some(SettingValue { value: json!(10), source: SettingSource::Default }));
        assert_eq!(resolve_setting(&canary, Some(&json!(5)), None).map(|value| value.source), Some(SettingSource::Yours));
        let locked = resolve_setting(&canary, Some(&json!(5)), Some(("acme", &json!(2)))).unwrap();
        assert_eq!((locked.value, locked.source), (json!(2), SettingSource::Org("acme".into())));
        assert_eq!(
            resolve_setting(&canary, Some(&json!("five")), None).map(|value| value.source),
            Some(SettingSource::Default),
            "a value that no longer fits"
        );
    }
}
