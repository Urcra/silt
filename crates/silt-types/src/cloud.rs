use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// A Claude Code cloud session (ADR 0040). claude.ai calls it
/// `session_<suffix>` and the container `cse_<suffix>`; both name one
/// session, so only the suffix is kept. It comes from peers and becomes an
/// argument to `claude`, so the suffix is letters and digits only.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CloudSessionId(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid cloud session {0:?}: expected session_… or cse_… with letters and digits")]
pub struct ParseCloudSessionError(String);

const MAX_SUFFIX: usize = 64;

impl CloudSessionId {
    /// Its page on claude.ai.
    pub fn url(&self) -> String {
        format!("https://claude.ai/code/{self}")
    }
}

impl fmt::Display for CloudSessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "session_{}", self.0)
    }
}

impl fmt::Debug for CloudSessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CloudSessionId({self})")
    }
}

/// Takes `session_…`, `cse_…`, or a claude.ai/code link to either.
impl FromStr for CloudSessionId {
    type Err = ParseCloudSessionError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || ParseCloudSessionError(text.to_owned());
        let id = text
            .strip_prefix("https://")
            .unwrap_or(text)
            .strip_prefix("claude.ai/code/")
            .map_or(text, |path| path.split(['?', '#', '/']).next().unwrap_or(path));
        let suffix = id.strip_prefix("session_").or_else(|| id.strip_prefix("cse_")).ok_or_else(invalid)?;
        if suffix.is_empty() || suffix.len() > MAX_SUFFIX || !suffix.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err(invalid());
        }
        Ok(Self(suffix.to_owned()))
    }
}

impl TryFrom<String> for CloudSessionId {
    type Error = ParseCloudSessionError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl From<CloudSessionId> for String {
    fn from(id: CloudSessionId) -> Self {
        id.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_prefixes_and_links_name_one_session() {
        let id: CloudSessionId = "session_01ExampleCloudSession1".parse().expect("session id");
        for text in [
            "cse_01ExampleCloudSession1",
            "https://claude.ai/code/session_01ExampleCloudSession1?from=cli&m=0",
            "claude.ai/code/cse_01ExampleCloudSession1",
        ] {
            assert_eq!(text.parse::<CloudSessionId>().as_ref(), Ok(&id), "{text}");
        }
        assert_eq!(id.to_string(), "session_01ExampleCloudSession1");
        assert_eq!(id.url(), "https://claude.ai/code/session_01ExampleCloudSession1");
    }

    #[test]
    fn rejects_what_could_become_another_argument() {
        let long = format!("session_{}", "a".repeat(MAX_SUFFIX + 1));
        for text in ["", "session_", "01Example", "cse_-p", "session_a b", "session_a/../b", "session_é", "--cloud", &long] {
            assert!(text.parse::<CloudSessionId>().is_err(), "{text:?}");
        }
    }

    #[test]
    fn serializes_as_its_claude_ai_name_and_checks_what_it_reads() {
        let id: CloudSessionId = "cse_01ABC".parse().expect("id");
        assert_eq!(serde_json::to_string(&id).expect("json"), "\"session_01ABC\"");
        assert!(serde_json::from_str::<CloudSessionId>("\"session_01 ABC\"").is_err());
        let bytes = postcard::to_allocvec(&id).expect("postcard");
        assert_eq!(postcard::from_bytes::<CloudSessionId>(&bytes).expect("back"), id);
    }
}
