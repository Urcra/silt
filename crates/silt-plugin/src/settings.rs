//! A plugin's settings, as its manifest declares them, and the value each
//! has: an org's, locked, over the person's, over the manifest's default.
//! A secret's value is never here; it's in the keyring, and only whether
//! one is kept is.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::manifest::{SettingKind, SettingScope, SettingSpec};

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

/// Where a value of `spec` for `environment` or `workspace` is kept in a
/// plugin's settings: `key` for the plugin, `key@environment` or
/// `key#workspace` for a setting of that scope. Why not, for a scope it
/// doesn't have.
pub fn setting_key(spec: &SettingSpec, environment: Option<&str>, workspace: Option<&str>) -> Result<String, String> {
    match (spec.scope, environment, workspace) {
        (_, Some(_), Some(_)) => Err("give an environment or a workspace, not both".into()),
        (SettingScope::Environment, Some(environment), None) => Ok(format!("{}@{environment}", spec.key)),
        (SettingScope::Workspace, None, Some(workspace)) => Ok(format!("{}#{workspace}", spec.key)),
        (_, None, None) => Ok(spec.key.clone()),
        (SettingScope::Plugin, ..) => Err(format!("{} is the plugin's alone, not per environment or workspace", spec.key)),
        (SettingScope::Environment, None, Some(_)) => Err(format!("{} is set per environment", spec.key)),
        (SettingScope::Workspace, Some(_), None) => Err(format!("{} is set per workspace", spec.key)),
    }
}

/// The value `spec` has in `environment` and `workspace`, from the
/// person's `settings` kept by [`setting_key`]: theirs for that scope, else
/// theirs for the plugin, else an org's or the default as
/// [`resolve_setting`] has it.
pub fn resolve_scoped(
    spec: &SettingSpec,
    settings: &BTreeMap<String, Value>,
    environment: Option<&str>,
    workspace: Option<&str>,
    org: Option<(&str, &Value)>,
) -> Option<SettingValue> {
    let scoped = match spec.scope {
        SettingScope::Environment => environment.map(|environment| format!("{}@{environment}", spec.key)),
        SettingScope::Workspace => workspace.map(|workspace| format!("{}#{workspace}", spec.key)),
        SettingScope::Plugin => None,
    };
    let yours = scoped.and_then(|key| settings.get(&key)).or_else(|| settings.get(&spec.key));
    resolve_setting(spec, yours, org)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

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

    #[test]
    fn a_setting_per_environment_falls_back_to_the_plugins() {
        let canary = SettingSpec { scope: SettingScope::Environment, ..spec(SettingKind::Number, Some(json!(10)), &[]) };
        assert_eq!(setting_key(&canary, Some("prod"), None).unwrap(), "k@prod");
        assert!(setting_key(&canary, None, Some("work")).is_err());
        assert!(setting_key(&spec(SettingKind::Number, None, &[]), Some("prod"), None).is_err());
        let settings = BTreeMap::from([("k".to_owned(), json!(5)), ("k@prod".to_owned(), json!(1))]);
        let value = |environment| resolve_scoped(&canary, &settings, environment, None, None).map(|value| value.value);
        assert_eq!(value(Some("prod")), Some(json!(1)));
        assert_eq!(value(Some("staging")), Some(json!(5)));
        assert_eq!(resolve_scoped(&canary, &BTreeMap::new(), Some("prod"), None, None).map(|value| value.value), Some(json!(10)));
    }
}
