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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadInfo {
    pub id: ThreadId,
    pub family: FamilyId,
    /// The repository's normalized origin URL, empty when it has none.
    pub repo: String,
    pub created_ms: i64,
    pub lease: Option<Lease>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseRequest {
    pub device: DeviceId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "kebab-case")]
pub enum ErrorReply {
    UnknownCursor,
    MissingParents { delta: DeltaId },
    Conflict { delta: DeltaId },
    Invalid { message: String },
    Unauthorized,
    NotFound,
    ProtocolMismatch { supported: Vec<u32> },
}
