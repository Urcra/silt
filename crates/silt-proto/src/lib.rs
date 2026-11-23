//! JSON bodies of the sync API (ADR 0009). Every request carries
//! [`PROTOCOL_HEADER`]; every response carries [`GENERATION_HEADER`].

use serde::{Deserialize, Serialize};
use silt_log::Delta;
use silt_types::{DeltaId, DeviceId, FamilyId, ThreadId};

pub mod artifact;
pub mod artifact_types;
pub mod edited;
pub mod frames;
pub mod page;
pub mod rooms;
pub mod svg;
pub mod theme;
pub mod view;

/// 4: comment replies and requested changes (ADR 0025). 5: reverts mark
/// rewound turns (ADR 0029). 6: a thread names its cloud session (ADR 0040).
/// 8: comments sent to the agent in a review (ADR 0055). 9: an agent's
/// sign-in, asked and answered from the web UI (ADR 0056). 10: chats, kept
/// out of every org, and moved into the project their folder becomes
/// (ADR 0059). 11: a thread's files marked seen, and the turn a person
/// reviewed it through (ADR 0067); tool calls that ran git, whose edits a
/// diff may leave out (ADR 0069). 12: a person's own design systems, synced
/// through each server they use (ADR 0071). 13: an artifact's page data
/// (`Op::Page`) and comments on a doc's text (ADR 0072, ADR 0077). 14:
/// rooms that span machines through `/v1/rooms` (ADR 0081). 15: a room's
/// presence sent as it changes, and comments on a sheet's cells. 16: a
/// design system's components (ADR 0086). 17: its components' guidelines,
/// cover and assets (ADR 0088). 18: edits by another agent in the same
/// clone, or by one of several (`Author::Others`, `Author::Agents`,
/// ADR 0089). 19: what a workspace's sandboxed agents may reach (ADR 0093).
/// 20: a person's own themes for Silt's UI, synced through each server
/// they use (ADR 0099). 21: the folders an org lets its members' agents
/// ask for, and the plugins it installs, allows and holds (ADR 0102,
/// ADR 0104). 22: the Auto preset, which a workspace's or an org's cap may
/// name (ADR 0114). 23: the person's devices by name in the listing, and
/// the machine holding each thread in its summary (ADR 0115).
pub const PROTOCOL: u32 = 23;
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
    Approved {
        login: LoginReply,
    },
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
    /// Where the registering machine fetches the repository from, without
    /// credentials, for others to clone it (ADR 0023).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_url: Option<String>,
    /// The thread this one recovers what diverged of (ADR 0028).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_of: Option<ThreadId>,
}

/// A value and when someone set it, so the newest of two settings wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Stamped<T> {
    pub value: T,
    pub at_ms: i64,
}

/// A thread put out of the way until `until`, or until it's woken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Snooze {
    /// None when not snoozed.
    pub until: Option<i64>,
    /// A new turn or merge request activity ends it early.
    pub wake_on_activity: bool,
}

/// What one person keeps about one thread for themselves (ADR 0024). A
/// change is one of these with only the fields it sets; [`ThreadState::merge`]
/// keeps the newest of each field, so changes made apart agree once both are
/// seen, in any order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ThreadState {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub thread: ThreadId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<Stamped<bool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snooze: Option<Stamped<Snooze>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settled: Option<Stamped<bool>>,
    /// When the person last looked at the thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen_ms: Option<Stamped<i64>>,
    /// The turn the person reviewed the thread's changes through (ADR 0067).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed_turn: Option<Stamped<u32>>,
}

fn newest<T: Ord + Copy>(mine: &mut Option<Stamped<T>>, theirs: Option<Stamped<T>>) -> bool {
    let Some(theirs) = theirs else {
        return false;
    };
    // Equal times fall to the larger value, so both sides pick the same.
    let newer = mine.is_none_or(|mine| (theirs.at_ms, theirs.value) > (mine.at_ms, mine.value));
    if newer {
        *mine = Some(theirs);
    }
    newer
}

impl ThreadState {
    pub fn new(thread: ThreadId) -> Self {
        Self { thread, pinned: None, snooze: None, settled: None, seen_ms: None, reviewed_turn: None }
    }

    /// Takes each of `other`'s fields that is newer; says whether any was.
    pub fn merge(&mut self, other: &ThreadState) -> bool {
        let pinned = newest(&mut self.pinned, other.pinned);
        let snooze = newest(&mut self.snooze, other.snooze);
        let settled = newest(&mut self.settled, other.settled);
        let seen = newest(&mut self.seen_ms, other.seen_ms);
        let reviewed = newest(&mut self.reviewed_turn, other.reviewed_turn);
        pinned || snooze || settled || seen || reviewed
    }
}

/// Whether a person has looked at a file of a merge request, or of a
/// thread (ADR 0067), at the version whose `hash` it had then (ADR 0024).
/// The newer mark wins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SeenMark {
    pub path: String,
    /// BLAKE3 of the file's text in the request, so a changed file shows
    /// as unseen again.
    pub hash: String,
    pub seen: bool,
    pub at_ms: i64,
    /// For a thread's file, the thread's version it was looked at, so what
    /// changed since can be shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

impl SeenMark {
    /// Whether `self` should replace `other` for the same file.
    pub fn newer_than(&self, other: &SeenMark) -> bool {
        (self.at_ms, self.seen) > (other.at_ms, other.seen)
    }
}

/// A person's marks on one merge request, by its id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MrSeen {
    pub mr: DeltaId,
    pub marks: Vec<SeenMark>,
}

/// A person's own design system, as their devices sync it through each
/// server they use (ADR 0071). The copy changed last wins; one removed
/// stays as a tombstone so the removal reaches every device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignSystemCopy {
    /// Its folder's name, which `design_system` takes.
    pub name: String,
    /// When it last changed on a device.
    pub at_ms: i64,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub readme: String,
    #[serde(default)]
    pub tokens: String,
    /// Its font files, by their name under `fonts/`, in base64.
    #[serde(default)]
    pub fonts: std::collections::BTreeMap<String, String>,
    /// Its `components.css` (ADR 0086).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub components_css: String,
    /// Its components' examples, by their name under `components/`.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub components: std::collections::BTreeMap<String, String>,
    /// Its components' guidelines, by name (ADR 0088).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub component_guides: std::collections::BTreeMap<String, String>,
    /// Its `cover.html`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cover: String,
    /// Its assets, by their path under `assets/` (`<Group>/<file>`), in
    /// base64.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub assets: std::collections::BTreeMap<String, String>,
}

/// The most a design system's copy may hold, its fonts in base64 included.
pub const DESIGN_SYSTEM_BYTES: usize = 16 << 20;

impl DesignSystemCopy {
    /// Whether a device may take it as it is: a system's name, font files
    /// and components named plainly, and no more than
    /// [`DESIGN_SYSTEM_BYTES`] and [`DESIGN_COMPONENTS`].
    pub fn valid(&self) -> bool {
        let named = |name: &str| {
            (1..=64).contains(&name.len())
                && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && !name.starts_with('-')
        };
        let fonts = self.fonts.keys().all(|file| {
            (1..=128).contains(&file.len())
                && !file.starts_with('.')
                && file.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        });
        let components = self.components.len() <= DESIGN_COMPONENTS
            && self.components.keys().all(|name| named(name))
            && self.components_css.len() <= DESIGN_COMPONENTS_CSS_BYTES
            && self.components.values().all(|html| html.len() <= DESIGN_COMPONENT_BYTES)
            && self.component_guides.keys().all(|name| self.components.contains_key(name))
            && self.component_guides.values().all(|guide| guide.len() <= DESIGN_COMPONENT_BYTES);
        // An asset's path is `<Group>/<file>`, each a plain name, so it
        // can't leave the system's `assets/` when written out.
        let plain = |part: &str| {
            (1..=64).contains(&part.len())
                && part.as_bytes()[0].is_ascii_alphanumeric()
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
                && !part.contains("..")
        };
        let assets = self.assets.len() <= DESIGN_ASSETS
            && self.assets.keys().all(|path| path.split_once('/').is_some_and(|(group, file)| plain(group) && plain(file)))
            && self.assets.values().all(|data| data.len() <= DESIGN_ASSET_BYTES.div_ceil(3) * 4);
        let size = self.readme.len()
            + self.tokens.len()
            + self.fonts.values().map(String::len).sum::<usize>()
            + self.components_css.len()
            + self.components.values().map(String::len).sum::<usize>()
            + self.component_guides.values().map(String::len).sum::<usize>()
            + self.cover.len()
            + self.assets.values().map(String::len).sum::<usize>();
        named(&self.name)
            && fonts
            && components
            && assets
            && self.cover.len() <= DESIGN_COVER_BYTES
            && size <= DESIGN_SYSTEM_BYTES
            && self.fonts.len() <= 64
    }
}

/// The most components a copy holds, and how large they may be; what
/// `silt_design` keeps of a system's folder (ADR 0086).
pub const DESIGN_COMPONENTS: usize = 48;
pub const DESIGN_COMPONENT_BYTES: usize = 16 << 10;
pub const DESIGN_COMPONENTS_CSS_BYTES: usize = 128 << 10;
/// A copy's cover and assets, as `silt_design` keeps them (ADR 0088).
pub const DESIGN_COVER_BYTES: usize = 64 << 10;
pub const DESIGN_ASSETS: usize = 64;
pub const DESIGN_ASSET_BYTES: usize = 1 << 20;

/// A person's own theme for Silt's UI, as their devices sync it through each
/// server they use (ADR 0099): the theme's file and the fonts it ships. The
/// copy changed last wins; one removed stays as a tombstone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeCopy {
    /// Its file's name without `.json`, or its folder's name.
    pub name: String,
    /// When it last changed on a device.
    pub at_ms: i64,
    #[serde(default)]
    pub deleted: bool,
    /// Its `theme.json`, or the file itself.
    #[serde(default)]
    pub json: String,
    /// The fonts it ships, by their name under `fonts/`, in base64.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub fonts: std::collections::BTreeMap<String, String>,
}

/// The most a theme's file may hold, as the daemon reads it.
pub const THEME_BYTES: usize = 256 << 10;

impl ThemeCopy {
    /// Whether a device may take it as it is: named as a file, its fonts
    /// named plainly, and no larger than a theme's file and fonts may be.
    pub fn valid(&self) -> bool {
        let named = (1..=64).contains(&self.name.len())
            && self.name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !self.name.starts_with('-');
        let base64_of = |bytes: u64| usize::try_from(bytes.div_ceil(3) * 4).unwrap_or(usize::MAX);
        let fonts = self.fonts.len() <= silt_theme::font::MOST_FACES
            && self.fonts.keys().all(|file| silt_theme::font::FontFile::parse(file).is_some_and(|font| font.name() == file))
            && self.fonts.values().all(|data| data.len() <= base64_of(silt_theme::font::FACE_BYTES))
            && self.fonts.values().map(String::len).sum::<usize>() <= base64_of(silt_theme::font::THEME_FONT_BYTES);
        named && fonts && self.json.len() <= THEME_BYTES
    }
}

/// A person's marks on one thread's files (ADR 0067).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadSeen {
    pub thread: ThreadId,
    pub marks: Vec<SeenMark>,
}

/// A project the caller can see (ADR 0023).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProjectInfo {
    /// The normalized remote that names it.
    pub repo: String,
    pub org: Option<String>,
    pub name: String,
    pub origin_url: Option<String>,
    /// Its own setting; none means its org's default.
    pub visibility: Option<Visibility>,
    pub default_agent: Option<String>,
    pub default_preset: Option<String>,
    #[serde(default)]
    pub git_notes: GitNotes,
    /// Whether its threads' pull requests get a comment from the server.
    #[serde(default)]
    pub forge_comments: bool,
    /// How many of its threads the caller can see.
    pub threads: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectsReply {
    pub projects: Vec<ProjectInfo>,
}

/// The name a project shows when nobody gave it one: its remote's last
/// path segment.
/// The code a page reads for a refusal (ADR 0072), from the start Silt's
/// own refusals give their message, so both serving modes answer alike:
/// `conflict`, `rate_limited` or `not_granted`; none for anything else.
pub fn refusal_of(message: &str) -> Option<&'static str> {
    if message.starts_with("conflict") {
        Some("conflict")
    } else if message.starts_with("rate limited") {
        Some("rate_limited")
    } else if message.starts_with("not_granted") {
        Some("not_granted")
    } else {
        None
    }
}

pub fn project_name(repo: &str) -> String {
    repo.trim_end_matches('/').rsplit(['/', ':']).next().filter(|name| !name.is_empty()).unwrap_or(repo).to_owned()
}

/// `url` without credentials, so it can be shared: no user or password
/// over HTTP, where they're tokens, and no password over SSH, where the
/// user (`git@`) is part of the address.
pub fn shareable_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    let Some((credentials, host)) = rest.split_once('@').filter(|(credentials, _)| !credentials.contains('/')) else {
        return url.to_owned();
    };
    if scheme.starts_with("http") {
        format!("{scheme}://{host}")
    } else {
        let user = credentials.split(':').next().unwrap_or_default();
        format!("{scheme}://{user}@{host}")
    }
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
    /// The listed threads that have a visibility of their own; the others
    /// follow their project's or org's.
    #[serde(default)]
    pub visibility: Vec<(ThreadId, Visibility)>,
    /// Agents running on the person's other online devices, for a daemon to
    /// show and pass commands on to (ADR 0026).
    #[serde(default)]
    pub running: Vec<RunningElsewhere>,
    /// The person's own devices by name, so a daemon can say which of them
    /// holds a thread's lease.
    #[serde(default)]
    pub devices: Vec<(DeviceId, String)>,
    /// The device that registered each listed thread, where the server
    /// knows it: where a thread no device has taken over lives.
    #[serde(default)]
    pub made_on: Vec<(ThreadId, DeviceId)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunningElsewhere {
    pub device: DeviceId,
    pub device_name: String,
    pub agent: AgentReport,
}

/// A command for the device running a thread's agent, from another of the
/// person's devices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayRequest {
    pub command: RemoteCommand,
    /// For a prompt: wait for the device if it's offline.
    #[serde(default)]
    pub when_back: bool,
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

/// Whether a project's commits get Silt's git notes (ADR 0042).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum GitNotes {
    #[default]
    Off,
    /// Written in each clone, and left there.
    Local,
    /// Written, and `refs/notes/silt` pushed to origin.
    Push,
}

impl std::fmt::Display for GitNotes {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Off => "off",
            Self::Local => "local",
            Self::Push => "push",
        })
    }
}

impl std::str::FromStr for GitNotes {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.trim() {
            "off" | "false" | "no" | "" => Ok(Self::Off),
            "local" | "true" | "yes" => Ok(Self::Local),
            "push" => Ok(Self::Push),
            _ => Err(format!("git notes are off, local or push, not {text:?}")),
        }
    }
}

/// Who sees a thread or a project's threads (ADR 0013). Written as
/// `private`, `org` or `team:<name>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
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

/// Moves a chat into `repo`, the project its folder became (ADR 0059);
/// owner only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveChat {
    pub repo: String,
    pub origin_url: Option<String>,
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
    Invalid {
        message: String,
    },
    Unauthorized,
    Forbidden {
        message: String,
    },
    NotFound,
    ProtocolMismatch {
        supported: Vec<u32>,
    },
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
    pub status: Option<ThreadStatus>,
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
    pub status: ThreadStatus,
    pub branch: Option<String>,
    pub kind: String,
    pub snippet: String,
    pub time_ms: i64,
    /// The doc or sheet whose text it's in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
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
    pub status: ThreadStatus,
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
    fn a_theme_copy_holds_only_what_a_theme_may() {
        let copy =
            |name: &str| ThemeCopy { name: name.into(), at_ms: 1, deleted: false, json: "{}".into(), fonts: Default::default() };
        assert!(copy("harbour-2").valid());
        for bad in ["", "Harbour", "../x", "-x", "a/b", "x.json"] {
            assert!(!copy(bad).valid(), "{bad}");
        }
        let mut fonted = copy("harbour");
        fonted.fonts.insert("sans.woff2".into(), "d09GMg==".into());
        assert!(fonted.valid());
        for bad in ["../sans.woff2", "fonts/sans.woff2", "sans.svg", ".sans.woff2"] {
            let mut bad_font = copy("harbour");
            bad_font.fonts.insert(bad.into(), "d09GMg==".into());
            assert!(!bad_font.valid(), "{bad}");
        }
        let mut big = copy("harbour");
        big.json = "x".repeat(THEME_BYTES + 1);
        assert!(!big.valid());
    }

    #[test]
    fn thread_state_keeps_the_newest_of_each_field_whatever_the_order() {
        let thread = ThreadId(7);
        let at = |value, at_ms| Some(Stamped { value, at_ms });
        let laptop = ThreadState { pinned: at(true, 10), settled: at(true, 30), ..ThreadState::new(thread) };
        let phone = ThreadState {
            pinned: at(false, 20),
            snooze: Some(Stamped { value: Snooze { until: Some(99), wake_on_activity: true }, at_ms: 5 }),
            ..ThreadState::new(thread)
        };
        let mut one = laptop.clone();
        assert!(one.merge(&phone));
        let mut other = phone.clone();
        assert!(other.merge(&laptop));
        assert_eq!(one, other);
        assert_eq!(one.pinned, at(false, 20));
        assert_eq!(one.settled, at(true, 30));
        assert!(!one.clone().merge(&laptop), "seeing an older change again changes nothing");

        let (mut tie_a, mut tie_b) = (
            ThreadState { pinned: at(false, 1), ..ThreadState::new(thread) },
            ThreadState { pinned: at(true, 1), ..ThreadState::new(thread) },
        );
        let a = tie_a.clone();
        tie_a.merge(&tie_b);
        tie_b.merge(&a);
        assert_eq!(tie_a, tie_b);
    }

    #[test]
    fn the_turn_reviewed_through_merges_as_the_other_fields_do() {
        let thread = ThreadId(7);
        let reviewed = |value, at_ms| ThreadState { reviewed_turn: Some(Stamped { value, at_ms }), ..ThreadState::new(thread) };
        let mut laptop = reviewed(3, 10);
        assert!(laptop.merge(&reviewed(5, 20)));
        assert!(!laptop.merge(&reviewed(4, 15)), "an older review changes nothing");
        assert_eq!(laptop.reviewed_turn.map(|turn| turn.value), Some(5));
        let json = serde_json::to_string(&ThreadState::new(thread)).expect("serializes");
        assert!(!json.contains("reviewed_turn"), "{json}");
    }

    #[test]
    fn a_seen_mark_names_its_version_only_for_a_threads_file() {
        let request = SeenMark { path: "a.rs".into(), hash: "h".into(), seen: true, at_ms: 1, version: None };
        let json = serde_json::to_string(&request).expect("serializes");
        assert!(!json.contains("version"), "{json}");
        let old: SeenMark =
            serde_json::from_str(r#"{"path":"a.rs","hash":"h","seen":true,"at_ms":1}"#).expect("an older mark reads");
        assert_eq!(old, request);
        let thread = SeenMark { version: Some("1@3".into()), ..request };
        assert_eq!(
            serde_json::from_str::<SeenMark>(&serde_json::to_string(&thread).expect("serializes")).expect("reads"),
            thread
        );
    }

    #[test]
    fn a_project_is_named_after_its_remote_and_shared_without_credentials() {
        assert_eq!(project_name("github.com/urcra/silt"), "silt");
        assert_eq!(project_name("example.com"), "example.com");
        assert_eq!(shareable_url("https://me:token@github.com/urcra/silt.git"), "https://github.com/urcra/silt.git");
        assert_eq!(shareable_url("https://ghp_x@github.com/urcra/silt"), "https://github.com/urcra/silt");
        assert_eq!(shareable_url("ssh://git:secret@host/silt.git"), "ssh://git@host/silt.git");
        assert_eq!(shareable_url("git@github.com:urcra/silt.git"), "git@github.com:urcra/silt.git");
        assert_eq!(shareable_url("/srv/git/silt.git"), "/srv/git/silt.git");
    }

    #[test]
    fn agent_states_read_as_they_are_written_and_as_older_peers_wrote_them() {
        for state in [
            AgentState::Starting,
            AgentState::Idle,
            AgentState::Working,
            AgentState::AwaitingPermission,
            AgentState::AwaitingInput,
            AgentState::Error,
            AgentState::Stopped,
        ] {
            let written = serde_json::to_string(&state).expect("encodes");
            assert_eq!(written, format!("\"{}\"", state.as_str()));
            assert_eq!(serde_json::from_str::<AgentState>(&written).expect("decodes"), state);
            assert_eq!(AgentState::parse(state.as_str()), Some(state));
        }
        assert_eq!(serde_json::from_str::<AgentState>("\"ready\"").expect("decodes"), AgentState::Idle);
        assert_eq!(serde_json::from_str::<AgentState>("\"exited\"").expect("decodes"), AgentState::Stopped);
    }

    #[test]
    fn conflicts_round_trip_with_full_actor_ids() {
        let reply = ErrorReply::Conflict { delta: DeltaId::new(ActorId(u128::MAX - 5), Lamport(34)) };
        let json = serde_json::to_string(&reply).expect("serializes");
        assert_eq!(serde_json::from_str::<ErrorReply>(&json).expect("deserializes"), reply);
    }
}

/// An org as one of its members sees it (ADR 0013).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct OrgInfo {
    pub name: String,
    pub role: OrgRole,
    /// The caller owns it or administers the server.
    #[serde(default)]
    pub may_change: bool,
    pub default_visibility: Visibility,
    pub members: Vec<OrgMember>,
    pub teams: Vec<TeamInfo>,
    pub projects: Vec<String>,
    /// Its threads may have read-only links.
    #[serde(default)]
    pub links: bool,
    /// What agents started in its projects may do, on every member's
    /// machine (ADR 0065).
    #[serde(default)]
    pub policy: silt_workspace::Policy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct OrgMember {
    pub user: String,
    pub role: OrgRole,
}

/// Threads written to within this long are [`ThreadStatus::Active`].
pub const ACTIVE_MS: i64 = 24 * 60 * 60 * 1000;

/// How a thread stands, for search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum ThreadStatus {
    /// Changed lately.
    Active,
    Idle,
    /// Merged into the thread it was forked from.
    Merged,
}

impl ThreadStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Idle => "idle",
            Self::Merged => "merged",
        }
    }
}

impl std::fmt::Display for ThreadStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ThreadStatus {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "active" => Ok(Self::Active),
            "idle" => Ok(Self::Idle),
            "merged" => Ok(Self::Merged),
            other => Err(format!("a thread's status is active, idle or merged, not {other:?}")),
        }
    }
}

/// What an agent the web UI started is doing (ADR 0022). Older peers'
/// `ready` and `exited` are read as `idle` and `stopped`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Starting,
    #[serde(alias = "ready")]
    Idle,
    Working,
    AwaitingPermission,
    AwaitingInput,
    Error,
    #[serde(alias = "exited")]
    Stopped,
}

impl AgentState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Idle => "idle",
            Self::Working => "working",
            Self::AwaitingPermission => "awaiting_permission",
            Self::AwaitingInput => "awaiting_input",
            Self::Error => "error",
            Self::Stopped => "stopped",
        }
    }

    /// The state named `name`, as `as_str` or an older peer spells it.
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "starting" => Self::Starting,
            "idle" | "ready" => Self::Idle,
            "working" => Self::Working,
            "awaiting_permission" => Self::AwaitingPermission,
            "awaiting_input" => Self::AwaitingInput,
            "error" => Self::Error,
            "stopped" | "exited" => Self::Stopped,
            _ => return None,
        })
    }

    /// Whether an agent in this state runs: it can be prompted or stopped.
    pub fn running(self) -> bool {
        !matches!(self, Self::Error | Self::Stopped)
    }

    /// Whether the person has to act for the agent to go on.
    pub fn waiting_on_person(self) -> bool {
        matches!(self, Self::AwaitingPermission | Self::AwaitingInput | Self::Error)
    }
}

impl std::fmt::Display for AgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Someone's place in an org.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum OrgRole {
    Owner,
    Member,
    /// A server admin outside the org, who may still change it.
    Admin,
}

impl OrgRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Member => "member",
            Self::Admin => "admin",
        }
    }
}

impl std::fmt::Display for OrgRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for OrgRole {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "owner" => Ok(Self::Owner),
            "member" => Ok(Self::Member),
            "admin" => Ok(Self::Admin),
            other => Err(format!("unknown org role {other:?}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum OrgChange {
    Create {
        org: String,
    },
    AddMember {
        org: String,
        user: String,
        owner: bool,
    },
    RemoveMember {
        org: String,
        user: String,
    },
    CreateTeam {
        org: String,
        team: String,
    },
    RemoveTeam {
        org: String,
        team: String,
    },
    AddToTeam {
        org: String,
        team: String,
        user: String,
    },
    RemoveFromTeam {
        org: String,
        team: String,
        user: String,
    },
    DefaultVisibility {
        org: String,
        visibility: Visibility,
    },
    /// Whether the org's threads may have read-only links (ADR 0027).
    Links {
        org: String,
        on: bool,
    },
    /// What agents started in the org's projects may do (ADR 0065).
    Policy {
        org: String,
        policy: Box<silt_workspace::Policy>,
    },
    /// Moves the project (a normalized remote) into `org`.
    MoveProject {
        repo: String,
        org: String,
    },
    /// What a project is called and what its new threads start with;
    /// `None` leaves a setting as it is, an empty string clears it.
    ProjectSettings {
        repo: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        default_agent: Option<String>,
        #[serde(default)]
        default_preset: Option<String>,
        #[serde(default)]
        git_notes: Option<GitNotes>,
        #[serde(default)]
        forge_comments: Option<bool>,
    },
}

/// Something a user should hear about (ADR 0013).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    pub id: i64,
    /// `mr_opened`, `mr_approved`, `mr_declined`, `mr_changes_requested`,
    /// `comment`, `reply` or `mention`; from what devices report,
    /// `permission`, `plan`, `finished`, `errored`, `offline` or `recovered`.
    pub kind: String,
    pub thread: ThreadId,
    /// Who caused it.
    pub actor: String,
    pub text: String,
    pub created_ms: i64,
    pub read: bool,
    #[serde(default)]
    pub action: Option<InboxAction>,
    /// Its kind notifies the person, not only lists it.
    #[serde(default)]
    pub notify: bool,
}

/// Every kind of notification, and whether it notifies unless the person
/// says otherwise (ADR 0028): what waits on them does, the rest is listed.
pub const NOTIFICATION_KINDS: [(&str, bool); 13] = [
    ("permission", true),
    ("plan", true),
    ("errored", true),
    ("offline", true),
    ("finished", false),
    ("recovered", false),
    ("mr_opened", false),
    ("mr_approved", false),
    ("mr_declined", false),
    ("mr_changes_requested", false),
    ("comment", false),
    ("reply", false),
    ("mention", false),
];

/// Whether one kind of notification notifies the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct KindSetting {
    pub kind: String,
    pub notify: bool,
}

/// What can be done from a notification in place (ADR 0028).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum InboxAction {
    /// Answer the agent's permission request with one of its options.
    Permission { id: u64, title: String, options: Vec<(String, String)> },
    /// Approve the agent's plan.
    Plan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxReply {
    pub notifications: Vec<Notification>,
}

/// Marks notifications read: `ids`, or every one with `all`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AgentActivity {
    pub name: String,
    pub state: AgentState,
    /// What it is saying right now, not recorded yet.
    #[serde(default)]
    pub streaming: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PresenceReply {
    /// Everyone else on the thread.
    pub people: Vec<Presence>,
}

/// An agent a daemon runs, as it reports it to the server (ADR 0026), so
/// browsers see what runs where without opening the thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AgentReport {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub thread: ThreadId,
    pub agent: String,
    /// The agent's registry id, so a browser can show its mark; empty from
    /// a daemon that predates it.
    #[serde(default)]
    pub agent_id: String,
    pub state: AgentState,
    /// What it waits on the person to answer.
    #[serde(default)]
    pub permissions: Vec<AskedPermission>,
    /// Questions it waits on the person to answer.
    #[serde(default)]
    pub questions: Vec<QuestionAsked>,
    /// Prompts waiting their turn.
    #[serde(default)]
    pub queued: u32,
    /// Which permission requests the daemon answers by itself, as the daemon
    /// names it.
    #[serde(default)]
    pub preset: String,
    /// The sign-in it waits on, when its own expired (ADR 0056).
    #[serde(default)]
    pub sign_in: Option<SignInAsked>,
    /// Commands and subagents it left running, which go on after its turn.
    #[serde(default)]
    pub background: u32,
}

/// An agent that needs signing in, as its card shows it (ADR 0056). What
/// it shows is never recorded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SignInAsked {
    /// The ways the agent lists, in its order.
    pub methods: Vec<SignInChoice>,
    /// The sign-in under way, when one is.
    pub running: Option<SignInShown>,
    /// Why the last one didn't sign the agent in.
    pub failed: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SignInChoice {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// The agent signs in by itself, on its machine, with nothing to show
    /// here.
    pub by_agent: bool,
}

/// What the sign-in's terminal shows that the person needs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SignInShown {
    /// The method running.
    pub method: String,
    /// The page to open, in any browser.
    pub link: Option<String>,
    /// The code to enter on that page.
    pub code: Option<String>,
    /// It waits for a code from the page to be pasted back.
    pub asks_code: bool,
    /// A code was pasted back, and the sign-in hasn't ended since.
    pub code_sent: bool,
    /// Its last lines, without escapes.
    pub tail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AskedPermission {
    pub id: u64,
    pub title: String,
    /// Option ids and labels.
    pub options: Vec<(String, String)>,
}

/// A question the agent waits on, as a form of fields to fill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct QuestionAsked {
    pub id: u64,
    pub message: String,
    pub fields: Vec<QuestionFieldAsked>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct QuestionFieldAsked {
    /// The key its answer goes under.
    pub key: String,
    pub title: String,
    pub description: Option<String>,
    /// Its choices; none for a free answer.
    pub options: Vec<ChoiceAsked>,
    pub multi: bool,
    /// The form can't be sent without it; the rest may be left empty.
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ChoiceAsked {
    /// What answering with it sends.
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

/// Every agent a daemon runs, and those it could start; sent when they
/// change and as a heartbeat.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentsReport {
    pub agents: Vec<AgentReport>,
    #[serde(default)]
    pub startable: Vec<Startable>,
}

/// An agent a device can start, by the id it knows it by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Startable {
    pub id: String,
    pub name: String,
}

/// What a person asks a daemon to do from elsewhere (ADR 0026). `Start` in
/// a thread that ran elsewhere continues it on this device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RemoteCommand {
    Start {
        agent: String,
        #[serde(default)]
        preset: Option<String>,
        #[serde(default)]
        mode: Option<String>,
    },
    Prompt {
        text: String,
        #[serde(default)]
        plan: bool,
    },
    Queue {
        text: String,
    },
    Steer {
        text: String,
    },
    Answer {
        id: u64,
        #[serde(default)]
        option: Option<String>,
    },
    /// Answers a question with each field's chosen values, or declines it.
    Reply {
        id: u64,
        #[serde(default)]
        answers: Option<Vec<(String, Vec<String>)>>,
    },
    Cancel,
    Stop,
    Preset {
        preset: String,
    },
    Mode {
        mode: String,
    },
    /// Picks one of a setting's choices, such as the agent's model.
    Setting {
        id: String,
        value: String,
    },
    /// A merge request as this device sees it, with its files and diff,
    /// which need the family's code.
    MrShow {
        mr: String,
        /// Leave out what git itself rewrote (ADR 0069).
        #[serde(default)]
        hide_git: bool,
        /// Leave out other agents' edits (ADR 0089).
        #[serde(default)]
        hide_others: bool,
    },
    /// Marks one of a merge request's files seen, or not, as it is now.
    MrSeen {
        mr: String,
        path: String,
        seen: bool,
    },
    /// A file of the thread at a version, for a permalink (ADR 0031): the
    /// server lacks the git texts most files start from.
    FileAt {
        path: String,
        at: Option<String>,
    },
    /// Signs the thread's agent in, one of the ways it lists (ADR 0056).
    SignIn {
        method: String,
    },
    /// A code from the sign-in's page, for the sign-in to take.
    SignInCode {
        code: String,
    },
    SignInCancel,
    /// A page's question for the device's agent, asked with no tools and no
    /// session (ADR 0072); the answer is `{text, truncated}`.
    Sample {
        question: String,
        /// What the page showed with it, which the server checked it
        /// declared.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<crate::artifact::SampleImage>,
        #[serde(default)]
        model: crate::artifact::SampleModel,
        /// Tools the page runs, which the device's agent calls through the
        /// server while it answers.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tools: Vec<crate::artifact::SampleTool>,
    },
    /// Stops the page's question this device answers for the command
    /// `relay`, ending its agent's run.
    StopSample {
        relay: i64,
    },
    /// The thread's files for a page that declares `files` (ADR 0078), as
    /// the device that holds the family reads them.
    PageFiles {
        artifact: String,
        asked: crate::page::FilesRequest,
    },
    /// A page's call of the person's page connectors, on the device that
    /// has them (ADR 0080): `{op, server, tool, input}`.
    PageMcp {
        artifact: String,
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        asked: serde_json::Value,
    },
}

/// What a person lets one of their devices do to the others (ADR 0026).
/// Missing ones are refused.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default)]
pub struct Capabilities {
    pub answer_permissions: bool,
    pub send_prompts: bool,
    pub run_agents: bool,
    pub review: bool,
    /// Ask one of the person's computers' agents a page's question (ADR 0072).
    pub sample: bool,
    /// Call the person's page connectors on one of their computers (ADR 0080).
    pub page_tools: bool,
}

impl Capabilities {
    pub const ALL: Self =
        Self { answer_permissions: true, send_prompts: true, run_agents: true, review: true, sample: true, page_tools: true };
    /// A paired phone's, unless its person gives it more (ADR 0028).
    pub const PHONE: Self =
        Self { answer_permissions: true, send_prompts: false, run_agents: false, review: true, sample: false, page_tools: false };

    /// Whether these let the device send `command`, and if not, what it
    /// would need, for saying so.
    pub fn allow(&self, command: &RemoteCommand) -> Result<(), &'static str> {
        let (allowed, needs) = match command {
            RemoteCommand::Answer { .. } | RemoteCommand::Reply { .. } => (self.answer_permissions, "answer permissions"),
            RemoteCommand::Prompt { .. } | RemoteCommand::Queue { .. } | RemoteCommand::Steer { .. } | RemoteCommand::Cancel => {
                (self.send_prompts, "send prompts")
            }
            RemoteCommand::Start { .. }
            | RemoteCommand::Stop
            | RemoteCommand::Preset { .. }
            | RemoteCommand::Mode { .. }
            | RemoteCommand::Setting { .. }
            // Which account the agent's work goes to (ADR 0056).
            | RemoteCommand::SignIn { .. }
            | RemoteCommand::SignInCode { .. }
            | RemoteCommand::SignInCancel => (self.run_agents, "start and stop agents"),
            RemoteCommand::MrShow { .. }
            | RemoteCommand::MrSeen { .. }
            | RemoteCommand::FileAt { .. }
            | RemoteCommand::PageFiles { .. } => {
                (self.review, "review merge requests")
            }
            RemoteCommand::Sample { .. } | RemoteCommand::StopSample { .. } => (self.sample, "ask an agent a page's question"),
            RemoteCommand::PageMcp { .. } => (self.page_tools, "call your page connectors"),
        };
        if allowed { Ok(()) } else { Err(needs) }
    }
}

/// A command waiting for the daemon it's meant for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlCommand {
    pub id: i64,
    pub thread: ThreadId,
    /// Who sent it, by name, and from which of their devices, if any.
    pub sender: String,
    #[serde(default)]
    pub from: Option<DeviceId>,
    /// What the sending device may do, for the daemon to check.
    #[serde(default)]
    pub capabilities: Capabilities,
    pub sent_ms: i64,
    pub command: RemoteCommand,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlReply {
    pub commands: Vec<ControlCommand>,
}

/// How a daemon answered a command: what it would have answered its own
/// web UI, or why not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandAnswer {
    pub ok: bool,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub data: serde_json::Value,
}

impl CommandAnswer {
    pub fn refused(message: impl Into<String>) -> Self {
        Self { ok: false, message: message.into(), data: serde_json::Value::Null }
    }
}
