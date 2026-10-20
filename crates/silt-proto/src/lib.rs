//! JSON bodies of the sync API (ADR 0009). Every request carries
//! [`PROTOCOL_HEADER`]; every response carries [`GENERATION_HEADER`].

use serde::{Deserialize, Serialize};
use silt_log::Delta;
use silt_types::{DeltaId, DeviceId, FamilyId, ThreadId};

pub const PROTOCOL: u32 = 1;
pub const PROTOCOL_HEADER: &str = "silt-protocol";
pub const GENERATION_HEADER: &str = "silt-generation";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginRequest {
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginReply {
    pub device: DeviceId,
    pub token: String,
    /// The user the device belongs to.
    pub user: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadInfo {
    pub id: ThreadId,
    pub family: FamilyId,
    /// The repository's normalized origin URL, empty when it has none.
    pub repo: String,
    pub created_ms: i64,
    pub lease: Option<Lease>,
    /// The owning user's name; set by the server in listings.
    #[serde(default)]
    pub owner: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    pub device: DeviceId,
    pub epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FamilyInfo {
    pub id: FamilyId,
    pub seq: u64,
    /// Changes when what this user may see of the family changes; the client
    /// then reads the family again from the start.
    #[serde(default)]
    pub view: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing {
    pub families: Vec<FamilyInfo>,
    pub threads: Vec<ThreadInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushRequest {
    pub deltas: Vec<Delta>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushReply {
    pub accepted: usize,
    pub seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullReply {
    pub deltas: Vec<(u64, Delta)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    /// Only the owner's devices see it.
    Private,
    /// Every user of the server sees it.
    Team,
}

/// Sets a project's visibility (`repo`) or a thread's own (`thread`, owner
/// only); exactly one of them is given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibilityRequest {
    pub repo: Option<String>,
    pub thread: Option<ThreadId>,
    pub visibility: Visibility,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "kebab-case")]
pub enum ErrorReply {
    UnknownCursor,
    MissingParents {
        #[serde(with = "delta_id_text")]
        delta: DeltaId,
    },
    Conflict {
        #[serde(with = "delta_id_text")]
        delta: DeltaId,
    },
    Invalid { message: String },
    Unauthorized,
    Forbidden { message: String },
    NotFound,
    ProtocolMismatch { supported: Vec<u32> },
}

/// Filters for `GET /v1/search`; every one is optional except `q`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub repo: Option<String>,
    /// The owning user's name.
    pub author: Option<String>,
    /// `active`, `idle` or `merged`.
    pub status: Option<String>,
    pub branch: Option<String>,
    /// A path, or a directory to match everything under it.
    pub file: Option<String>,
    /// Milliseconds since the Unix epoch.
    pub since: Option<i64>,
    pub until: Option<i64>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    pub thread: ThreadId,
    pub repo: String,
    pub owner: String,
    pub status: String,
    pub branch: Option<String>,
    pub kind: String,
    pub snippet: String,
    pub time_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchReply {
    pub hits: Vec<SearchHit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InFlightQuery {
    pub repo: String,
    /// A path, a directory, or empty for the whole repository.
    #[serde(default)]
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InFlightThread {
    pub thread: ThreadId,
    pub owner: String,
    pub status: String,
    pub paths: Vec<String>,
    pub last_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InFlightReply {
    pub threads: Vec<InFlightThread>,
}

/// Delta ids as `actor.lamport` text: internally tagged enums buffer their
/// fields, and that loses the 128-bit actor ids JSON numbers would carry.
mod delta_id_text {
    use std::str::FromStr;

    use serde::{Deserialize, Deserializer, Serializer};
    use silt_types::DeltaId;

    pub fn serialize<S: Serializer>(id: &DeltaId, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(id)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<DeltaId, D::Error> {
        let text = String::deserialize(deserializer)?;
        DeltaId::from_str(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use silt_types::{ActorId, Lamport};

    use super::*;

    #[test]
    fn conflicts_round_trip_with_full_actor_ids() {
        let reply = ErrorReply::Conflict {
            delta: DeltaId::new(ActorId(u128::MAX - 5), Lamport(34)),
        };
        let json = serde_json::to_string(&reply).expect("serializes");
        assert_eq!(serde_json::from_str::<ErrorReply>(&json).expect("deserializes"), reply);
    }
}
