//! What crosses the `silt:plugin@1` world as JSON: the tools a plugin
//! offers, the call it prepares, the context it's given, and what it
//! returns. The world's capability interfaces are typed WIT; these are the
//! plugin's own descriptions, which the host checks before drawing or
//! running anything.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::card::Card;
use crate::effect::Effect;

/// A tool the plugin offers agents, mapped to one of its actions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Tool {
    /// Unique within the plugin; agents see it prefixed with the plugin's.
    pub name: String,
    pub description: String,
    /// A JSON Schema for its input.
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub input_schema: Value,
    /// The action its calls are, unless `prepare` says which.
    pub action: String,
}

/// A call as the plugin prepared it. The host adds what the plugin can't
/// choose: the plugin, its version, the tool and the agent's input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Call {
    pub action: String,
    /// The fields its action's rules may match on.
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    pub card: Card,
    #[serde(default)]
    pub effects: Vec<Effect>,
}

/// What a plugin is told about the call it's handling.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Context {
    pub thread: String,
    pub workspace: String,
    /// The environment the host picked from the call's fields, once known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    /// The environments configured, with their patterns, so `prepare` can
    /// say which one a call goes to.
    #[serde(default)]
    pub environments: BTreeMap<String, Vec<String>>,
}

/// What a call returns to the agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Output {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub data: Option<Value>,
}

impl Output {
    pub fn text(text: impl Into<String>) -> Self {
        Self { text: text.into(), data: None }
    }
}
