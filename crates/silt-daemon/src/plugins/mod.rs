//! Plugins (ADR 0093, ADR 0103): what a sandboxed agent reaches production
//! and other systems through, each a WebAssembly component Silt runs on
//! this machine, outside every sandbox, with credentials the agent never
//! holds. A call its rules say asks waits as an approval, bound to the hash
//! of what the plugin prepared: only that runs, once, and only after the
//! person says so.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use silt_access::{DataClass, Schema};

mod calls;
pub(crate) mod capabilities;
pub mod install;
pub mod installed;
mod local;
pub(crate) mod wasm;

pub use calls::{
    AccessView, ApprovalState, ApprovalView, Approvals, CallView, FolderView, GrantScope, GrantView, NetworkRefusal, RelayView,
};

/// A tool a plugin offers agents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PluginTool {
    pub name: String,
    pub description: String,
    #[cfg_attr(test, ts(type = "unknown"))]
    pub input_schema: Value,
}

/// The GraphQL endpoints a workspace's sandboxes may reach straight
/// through the gateway: GraphQL API's environments set `direct`, read the
/// same way, queries passing, mutations only where allowed, and its data
/// rules applied with the schema it keeps.
pub fn direct_graphql(data_dir: &Path, workspace: &str) -> Vec<silt_gateway::GraphqlRule> {
    let Some(graphql) = installed::find(data_dir, "silt.graphql").filter(|graphql| graphql.on_in(workspace)) else {
        return Vec::new();
    };
    let value = |key: &str, environment: &str| {
        graphql.settings.get(&format!("{key}@{environment}")).or_else(|| graphql.settings.get(key))
    };
    let mut rules = Vec::new();
    for environment in graphql.environments.keys() {
        if value("direct", environment) != Some(&Value::Bool(true)) {
            continue;
        }
        let Some(url) = value("endpoint", environment).and_then(Value::as_str).and_then(|url| reqwest::Url::parse(url).ok())
        else {
            continue;
        };
        let Some(host) = url.host_str() else { continue };
        let data = value("data", environment)
            .and_then(Value::as_array)
            .map(|named| named.iter().filter(|name| name.as_str() == Some("location")).map(|_| DataClass::Location).collect())
            .unwrap_or_default();
        let changes = graphql.rules.iter().any(|rule| {
            rule.action == "mutate"
                && rule.rule == silt_plugin::Rule::Allow
                && rule.pattern.as_ref().is_none_or(|pattern| pattern.matches(environment))
        });
        rules.push(silt_gateway::GraphqlRule {
            host: host.to_owned(),
            path: url.path().to_owned(),
            changes,
            data,
            schema: schema_of(data_dir, environment).map(std::sync::Arc::new),
            plugin: "silt.graphql".into(),
        });
    }
    rules
}

/// The schema GraphQL API keeps for `environment`, as `graphql_schema`
/// fetched it.
pub fn schema_of(data_dir: &Path, environment: &str) -> Option<Schema> {
    if !environment.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c)) {
        return None;
    }
    let path = installed::dir(data_dir, "silt.graphql").join("data").join(format!("{environment}.schema.json"));
    Schema::from_introspection(&serde_json::from_slice(&std::fs::read(path).ok()?).ok()?).ok()
}

/// How a call ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum Outcome {
    Done {
        output: String,
    },
    /// Refused here, by a rule or the person; nothing of it was shown.
    Refused {
        why: String,
    },
    /// It failed before anything was sent.
    Failed {
        error: String,
    },
    /// Sent, with no end seen: it may or may not have happened. Never sent
    /// again by itself.
    Unknown {
        why: String,
    },
    /// Waiting for the person.
    Pending {
        approval: u64,
    },
}

impl Outcome {
    /// What the agent reads.
    pub fn said(&self) -> String {
        match self {
            Self::Done { output } => output.clone(),
            Self::Refused { why } => format!("Refused: {why}"),
            Self::Failed { error } => format!("Failed before anything was sent: {error}"),
            Self::Unknown { why } => format!(
                "Outcome unknown: {why}. It may or may not have happened; Silt won't send it again by itself. Check before asking again."
            ),
            Self::Pending { approval } => format!(
                "Waiting for the person to approve it (approval {approval}). Call silt_plugin_wait with this approval to wait on; don't send it again."
            ),
        }
    }
}
