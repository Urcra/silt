//! JSON bodies of the sync API (ADR 0009). Every request carries
//! [`PROTOCOL_HEADER`]; every response carries [`GENERATION_HEADER`].

use serde::{Deserialize, Serialize};
use silt_log::Delta;
use silt_types::{DeltaId, DeviceId, FamilyId, ThreadId};

pub const PROTOCOL: u32 = 3;
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

/// Signs a device in with a local account's password.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasswordLoginRequest {
    pub user: String,
    pub password: String,
    pub name: String,
}

/// Starts signing a device in through the browser (ADR 0013).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceStartRequest {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceStartReply {
    /// Secret the device polls with.
    pub device_code: String,
    /// What the person checks in the browser matches this device.
    pub user_code: String,
    /// Where the person approves the device, with the code filled in.
    pub verification_url: String,
    pub interval_secs: u64,
    pub expires_in_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevicePollRequest {
    pub device_code: String,
}

/// A plain enum: serde's tagged enums can't carry the `u128` device id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DevicePollReply {
    Pending,
    Approved { login: LoginReply },
    /// Expired, denied or unknown; start again.
    Gone,
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
    /// Users the owner lets write into it (ADR 0014); set in listings.
    #[serde(default)]
    pub collaborators: Vec<String>,
}

/// A device registering a thread. A thread meant to be private from the
/// start says so here, before any of its deltas reach the server, so no one
/// else can see it in between.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterThread {
    #[serde(flatten)]
    pub thread: ThreadInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,
}

/// Adds a collaborator to a thread, or removes one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareRequest {
    pub user: String,
    #[serde(default)]
    pub remove: bool,
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
    /// Threads in the trash; they stay hidden until restored or purged.
    #[serde(default)]
    pub deleted: Vec<ThreadId>,
    /// Threads purged for good: every device drops them (ADR 0013).
    #[serde(default)]
    pub purged: Vec<ThreadId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushRequest {
    pub deltas: Vec<Delta>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushReply {
    pub accepted: u64,
    pub seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullReply {
    pub deltas: Vec<(u64, Delta)>,
}

/// Who sees a thread or a project's threads (ADR 0013). Written as
/// `private`, `org` or `team:<name>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Visibility {
    /// Only its owner, and what merge requests offer to others.
    Private,
    /// Members of this team of the project's org.
    Team(String),
    /// Every member of the project's org.
    Org,
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Private => formatter.write_str("private"),
            Self::Org => formatter.write_str("org"),
            Self::Team(team) => write!(formatter, "team:{team}"),
        }
    }
}

impl std::str::FromStr for Visibility {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "private" => Ok(Self::Private),
            "org" => Ok(Self::Org),
            _ => match text.strip_prefix("team:") {
                Some(team) if !team.is_empty() => Ok(Self::Team(team.to_owned())),
                _ => Err(format!("visibility is private, org or team:<name>, not {text:?}")),
            },
        }
    }
}

impl TryFrom<String> for Visibility {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl From<Visibility> for String {
    fn from(visibility: Visibility) -> Self {
        visibility.to_string()
    }
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

impl std::fmt::Display for ErrorReply {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownCursor => formatter.write_str("the server no longer has the history this device synced"),
            Self::MissingParents { delta } => write!(formatter, "the server lacks what {delta} builds on"),
            Self::Conflict { delta } => write!(formatter, "the server holds a different {delta}"),
            Self::Invalid { message } | Self::Forbidden { message } => formatter.write_str(message),
            Self::Unauthorized => formatter.write_str("not signed in, or the sign-in was revoked; run `silt auth login`"),
            Self::NotFound => formatter.write_str("not found"),
            Self::ProtocolMismatch { supported } => write!(formatter, "the server speaks protocol {supported:?}"),
        }
    }
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

/// An org as one of its members sees it (ADR 0013).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgInfo {
    pub name: String,
    /// The caller's role: `owner`, `member`, or `admin` for a server admin
    /// outside the org.
    pub role: String,
    pub default_visibility: Visibility,
    pub members: Vec<OrgMember>,
    pub teams: Vec<TeamInfo>,
    pub projects: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgMember {
    pub user: String,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamInfo {
    pub name: String,
    pub members: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgsReply {
    pub orgs: Vec<OrgInfo>,
}

/// A change to orgs, teams or projects; the caller must own the org or
/// administer the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum OrgChange {
    Create { org: String },
    AddMember { org: String, user: String, owner: bool },
    RemoveMember { org: String, user: String },
    CreateTeam { org: String, team: String },
    RemoveTeam { org: String, team: String },
    AddToTeam { org: String, team: String, user: String },
    RemoveFromTeam { org: String, team: String, user: String },
    DefaultVisibility { org: String, visibility: Visibility },
    /// Moves the project (a normalized remote) into `org`.
    MoveProject { repo: String, org: String },
}

/// Something a user should hear about (ADR 0013).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    pub id: i64,
    /// `mr_opened`, `mr_approved`, `mr_declined`, `comment` or `mention`.
    pub kind: String,
    pub thread: ThreadId,
    /// Who caused it.
    pub actor: String,
    pub text: String,
    pub created_ms: i64,
    pub read: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxReply {
    pub notifications: Vec<Notification>,
}

/// Marks notifications read: `ids`, or every one with `all`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxRead {
    #[serde(default)]
    pub ids: Vec<i64>,
    #[serde(default)]
    pub all: bool,
}

/// What one viewer of a thread is doing (ADR 0014); sent every couple of
/// seconds while they look at it, forgotten soon after they stop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceUpdate {
    pub thread: ThreadId,
    /// The file they have open, if any.
    #[serde(default)]
    pub file: Option<String>,
    /// The line their cursor is on in that file, from 1, for following them.
    #[serde(default)]
    pub line: Option<u32>,
    /// Their draft of the next prompt.
    #[serde(default)]
    pub draft: String,
    /// They closed the thread.
    #[serde(default)]
    pub leaving: bool,
    /// The agent their machine runs in the thread, if any.
    #[serde(default)]
    pub agent: Option<AgentActivity>,
}

/// An agent at work in a thread, as its co-viewers see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentActivity {
    pub name: String,
    pub state: String,
    /// What it is saying right now, not recorded yet.
    #[serde(default)]
    pub streaming: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presence {
    pub user: String,
    pub device: String,
    pub file: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
    pub draft: String,
    #[serde(default)]
    pub agent: Option<AgentActivity>,
    /// Milliseconds since they were last heard from.
    pub age_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceReply {
    /// Everyone else on the thread.
    pub people: Vec<Presence>,
}
