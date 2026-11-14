use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Preset;
use crate::policy::Policy;

/// A workspace's id: a slug, as it's used in file and URL paths.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(type = "string"))]
pub struct WorkspaceId(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} isn't a workspace id: lower-case letters, digits and dashes, starting with a letter or digit, at most 40")]
pub struct BadWorkspaceId(String);

impl WorkspaceId {
    pub fn new(id: impl Into<String>) -> Result<Self, BadWorkspaceId> {
        let id = id.into();
        let mut chars = id.chars();
        let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
        let rest_ok = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if first_ok && rest_ok && id.len() <= 40 { Ok(Self(id)) } else { Err(BadWorkspaceId(id)) }
    }

    /// The id a name suggests: "Work stuff" is `work-stuff`.
    pub fn from_name(name: &str) -> Result<Self, BadWorkspaceId> {
        let mut slug = String::new();
        for c in name.trim().chars().flat_map(char::to_lowercase) {
            if c.is_ascii_alphanumeric() {
                slug.push(c);
            } else if !slug.is_empty() && !slug.ends_with('-') {
                slug.push('-');
            }
        }
        let slug = slug.trim_end_matches('-');
        Self::new(slug.chars().take(40).collect::<String>().trim_end_matches('-'))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for WorkspaceId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for WorkspaceId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The server a workspace's threads sync with, and the org they belong to
/// there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ServerBinding {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org: Option<String>,
}

/// What puts a project in a workspace without the person choosing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case", tag = "kind", content = "pattern")]
pub enum MatchRule {
    /// A project's normalized remote, `/`-separated: `*` is one segment and
    /// a trailing `**` is any number of them (`github.com/acme/*`).
    Remote(String),
    /// A clone at or under this folder, absolute and `/`-separated.
    Folder(String),
}

/// How well a rule matched; the higher, the more specific.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Specificity {
    remote: bool,
    literal_segments: usize,
    exact: bool,
}

impl MatchRule {
    /// A folder rule for `folder`, which already covers everything under
    /// it: a trailing `/*` or `/**`, as one written like a remote pattern
    /// has, means the folder itself.
    pub fn folder(folder: &str) -> Self {
        let mut folder = folder.trim_end_matches('/');
        while let Some(parent) = folder.strip_suffix("/**").or_else(|| folder.strip_suffix("/*")) {
            folder = parent.trim_end_matches('/');
        }
        Self::Folder(folder.to_owned())
    }

    fn matches(&self, project: &str, clone: Option<&str>) -> Option<Specificity> {
        match self {
            Self::Remote(pattern) => remote_matches(pattern, project),
            Self::Folder(folder) => clone.and_then(|clone| folder_matches(folder, clone)),
        }
    }
}

fn remote_matches(pattern: &str, project: &str) -> Option<Specificity> {
    if project.starts_with('/') || (project.contains(':') && !project.contains('/')) {
        return None;
    }
    let pattern: Vec<&str> = pattern.trim_matches('/').split('/').collect();
    let project: Vec<&str> = project.split('/').collect();
    let (last, fixed) = pattern.split_last()?;
    let open = *last == "**";
    let fixed = if open { fixed } else { &pattern[..] };
    if (open && project.len() <= fixed.len()) || (!open && project.len() != fixed.len()) {
        return None;
    }
    let mut literal_segments = 0;
    for (want, have) in fixed.iter().zip(&project) {
        match *want {
            "*" => {}
            "**" => return None,
            want if want.eq_ignore_ascii_case(have) => literal_segments += 1,
            _ => return None,
        }
    }
    Some(Specificity { remote: true, literal_segments, exact: !open && literal_segments == fixed.len() })
}

/// Whether `path`, `/`-separated, is absolute: from the root, or from a
/// Windows drive (`C:/work`).
pub fn is_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with('/') || (bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/')
}

fn folder_matches(folder: &str, clone: &str) -> Option<Specificity> {
    let folder = folder.trim_end_matches('/');
    // A drive's paths don't tell case apart.
    let rest = if folder.starts_with('/') {
        clone.strip_prefix(folder)?
    } else {
        let head = clone.get(..folder.len()).filter(|head| head.eq_ignore_ascii_case(folder))?;
        &clone[head.len()..]
    };
    if !(rest.is_empty() || rest.starts_with('/')) || folder.is_empty() {
        return None;
    }
    let literal_segments = folder.split('/').filter(|segment| !segment.is_empty()).count();
    Some(Specificity { remote: false, literal_segments, exact: rest.is_empty() })
}

/// What a new thread starts with in a workspace; the person can change
/// each, within the rules.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Defaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<Preset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chats_preset: Option<Preset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    /// `None` keeps its threads on this machine.
    #[serde(default)]
    pub server: Option<ServerBinding>,
    #[serde(default)]
    pub rules: Vec<MatchRule>,
    #[serde(default)]
    pub defaults: Defaults,
    /// Given to every agent started in it, such as its own
    /// `CLAUDE_CONFIG_DIR`.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub policy: Policy,
}

impl Workspace {
    pub fn new(id: WorkspaceId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            server: None,
            rules: Vec::new(),
            defaults: Defaults::default(),
            env: BTreeMap::new(),
            policy: Policy::default(),
        }
    }
}

/// Every workspace on a machine, as `workspaces.json` keeps them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Workspaces {
    /// Where projects no rule matches go.
    pub default: WorkspaceId,
    pub workspaces: Vec<Workspace>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidWorkspaces {
    #[error("there's no workspace")]
    Empty,
    #[error("two workspaces have the id {0}")]
    Duplicate(WorkspaceId),
    #[error("the default workspace {0} doesn't exist")]
    NoDefault(WorkspaceId),
    #[error("{0} has an empty match rule")]
    EmptyRule(WorkspaceId),
    #[error("{workspace}'s folder {folder:?} isn't absolute")]
    RelativeFolder { workspace: WorkspaceId, folder: String },
    #[error("{workspace}'s folder {folder:?} has a wildcard; a folder rule takes one folder and covers everything under it")]
    WildcardFolder { workspace: WorkspaceId, folder: String },
}

/// Why a project is in its workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case", tag = "kind", content = "rule")]
pub enum Why {
    /// The person put it there.
    Chosen,
    Rule(MatchRule),
    /// Nothing matched it.
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved<'a> {
    pub workspace: &'a Workspace,
    pub why: Why,
}

impl Workspaces {
    /// One workspace, the default, with the given server: what a machine
    /// had before workspaces.
    pub fn single(server: Option<ServerBinding>) -> Self {
        let id = WorkspaceId::new("default").expect("a valid id");
        let mut workspace = Workspace::new(id.clone(), "Default");
        workspace.server = server;
        Self { default: id, workspaces: vec![workspace] }
    }

    pub fn validate(&self) -> Result<(), InvalidWorkspaces> {
        if self.workspaces.is_empty() {
            return Err(InvalidWorkspaces::Empty);
        }
        for (index, workspace) in self.workspaces.iter().enumerate() {
            if self.workspaces[..index].iter().any(|other| other.id == workspace.id) {
                return Err(InvalidWorkspaces::Duplicate(workspace.id.clone()));
            }
            for rule in &workspace.rules {
                match rule {
                    MatchRule::Remote(pattern) if pattern.trim_matches('/').is_empty() => {
                        return Err(InvalidWorkspaces::EmptyRule(workspace.id.clone()));
                    }
                    MatchRule::Folder(folder) if !is_absolute(folder) => {
                        return Err(InvalidWorkspaces::RelativeFolder {
                            workspace: workspace.id.clone(),
                            folder: folder.clone(),
                        });
                    }
                    MatchRule::Folder(folder) if folder.contains('*') => {
                        return Err(InvalidWorkspaces::WildcardFolder {
                            workspace: workspace.id.clone(),
                            folder: folder.clone(),
                        });
                    }
                    _ => {}
                }
            }
        }
        if self.get(&self.default).is_none() {
            return Err(InvalidWorkspaces::NoDefault(self.default.clone()));
        }
        Ok(())
    }

    /// Writes each folder rule as [`MatchRule::folder`] would.
    pub fn tidy(&mut self) {
        for rule in self.workspaces.iter_mut().flat_map(|workspace| &mut workspace.rules) {
            if let MatchRule::Folder(folder) = rule {
                *rule = MatchRule::folder(folder);
            }
        }
    }

    pub fn get(&self, id: &WorkspaceId) -> Option<&Workspace> {
        self.workspaces.iter().find(|workspace| workspace.id == *id)
    }

    pub fn get_mut(&mut self, id: &WorkspaceId) -> Option<&mut Workspace> {
        self.workspaces.iter_mut().find(|workspace| workspace.id == *id)
    }

    pub fn default_workspace(&self) -> &Workspace {
        self.get(&self.default).unwrap_or(&self.workspaces[0])
    }

    /// The workspace of a project: the one the person chose for it, else
    /// the most specific rule's, else the default. `clone` is where the
    /// project is cloned on this machine, if it is.
    pub fn resolve(&self, project: &str, clone: Option<&str>, chosen: Option<&WorkspaceId>) -> Resolved<'_> {
        if let Some(workspace) = chosen.and_then(|id| self.get(id)) {
            return Resolved { workspace, why: Why::Chosen };
        }
        let best = self
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.rules.iter().map(move |rule| (workspace, rule)))
            .filter_map(|(workspace, rule)| rule.matches(project, clone).map(|fit| (fit, workspace, rule)))
            .max_by_key(|(fit, _, _)| *fit);
        match best {
            Some((_, workspace, rule)) => Resolved { workspace, why: Why::Rule(rule.clone()) },
            None => Resolved { workspace: self.default_workspace(), why: Why::Default },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(id: &str) -> WorkspaceId {
        WorkspaceId::new(id).unwrap()
    }

    fn sample() -> Workspaces {
        let mut personal = Workspace::new(id("personal"), "Personal");
        personal.rules = vec![MatchRule::Remote("github.com/me/*".into()), MatchRule::Folder("/home/me/code".into())];
        let mut work = Workspace::new(id("work"), "Work");
        work.rules = vec![
            MatchRule::Remote("github.com/acme/*".into()),
            MatchRule::Remote("gitlab.acme.dev/**".into()),
            MatchRule::Folder("/home/me/work".into()),
            MatchRule::Folder("C:/Users/me/work".into()),
        ];
        let mut billing = Workspace::new(id("billing"), "Billing");
        billing.rules = vec![MatchRule::Remote("github.com/acme/billing".into())];
        Workspaces { default: id("personal"), workspaces: vec![personal, work, billing] }
    }

    #[test]
    fn the_most_specific_rule_decides() {
        let all = sample();
        let cases: &[(&str, Option<&str>, &str, Why)] = &[
            ("github.com/acme/web", None, "work", Why::Rule(MatchRule::Remote("github.com/acme/*".into()))),
            ("github.com/acme/billing", None, "billing", Why::Rule(MatchRule::Remote("github.com/acme/billing".into()))),
            ("GitHub.com/Acme/web", None, "work", Why::Rule(MatchRule::Remote("github.com/acme/*".into()))),
            ("gitlab.acme.dev/platform/infra", None, "work", Why::Rule(MatchRule::Remote("gitlab.acme.dev/**".into()))),
            ("gitlab.acme.dev", None, "personal", Why::Default),
            ("github.com/acme/web/extra", None, "personal", Why::Default),
            // A remote beats a folder, whichever is longer.
            (
                "github.com/acme/web",
                Some("/home/me/code/acme-web"),
                "work",
                Why::Rule(MatchRule::Remote("github.com/acme/*".into())),
            ),
            ("github.com/other/thing", Some("/home/me/work/thing"), "work", Why::Rule(MatchRule::Folder("/home/me/work".into()))),
            ("github.com/other/thing", Some("/home/me/workshop/thing"), "personal", Why::Default),
            // A project named by its folder (ADR 0053) matches folders only.
            (
                "/home/me/work/scratch",
                Some("/home/me/work/scratch"),
                "work",
                Why::Rule(MatchRule::Folder("/home/me/work".into())),
            ),
            ("silt:chats", None, "personal", Why::Default),
            (
                "github.com/other/thing",
                Some("c:/Users/me/Work/thing"),
                "work",
                Why::Rule(MatchRule::Folder("C:/Users/me/work".into())),
            ),
        ];
        for (project, clone, want, why) in cases {
            let resolved = all.resolve(project, *clone, None);
            assert_eq!(resolved.workspace.id.as_str(), *want, "{project} at {clone:?}");
            assert_eq!(&resolved.why, why, "{project} at {clone:?}");
        }
    }

    #[test]
    fn the_persons_choice_beats_rules() {
        let all = sample();
        let resolved = all.resolve("github.com/acme/web", None, Some(&id("personal")));
        assert_eq!(resolved.workspace.id.as_str(), "personal");
        assert_eq!(resolved.why, Why::Chosen);
        // A choice naming a workspace that's gone falls back to the rules.
        assert_eq!(all.resolve("github.com/acme/web", None, Some(&id("gone"))).workspace.id.as_str(), "work");
    }

    #[test]
    fn ids_are_slugs() {
        assert!(WorkspaceId::new("work-2").is_ok());
        for bad in ["", "-work", "Work", "work stuff", "wörk", &"a".repeat(41)] {
            assert!(WorkspaceId::new(bad).is_err(), "{bad:?}");
        }
        assert_eq!(WorkspaceId::from_name("  Work stuff (Acme) ").unwrap().as_str(), "work-stuff-acme");
        assert!(WorkspaceId::from_name("!!!").is_err());
        assert!(serde_json::from_str::<WorkspaceId>("\"Not OK\"").is_err());
    }

    #[test]
    fn validation_names_what_is_wrong() {
        let mut all = sample();
        assert_eq!(all.validate(), Ok(()));
        all.default = id("gone");
        assert_eq!(all.validate(), Err(InvalidWorkspaces::NoDefault(id("gone"))));
        let mut all = sample();
        all.workspaces.push(Workspace::new(id("work"), "Again"));
        assert_eq!(all.validate(), Err(InvalidWorkspaces::Duplicate(id("work"))));
        let mut all = sample();
        all.workspaces[1].rules.push(MatchRule::Folder("~/work".into()));
        assert!(matches!(all.validate(), Err(InvalidWorkspaces::RelativeFolder { .. })));
        let mut all = sample();
        all.workspaces[1].rules.push(MatchRule::Folder("/home/*/work".into()));
        assert!(matches!(all.validate(), Err(InvalidWorkspaces::WildcardFolder { .. })));
        assert_eq!(Workspaces { default: id("x"), workspaces: vec![] }.validate(), Err(InvalidWorkspaces::Empty));
    }

    #[test]
    fn a_folder_written_like_a_remote_pattern_means_the_folder() {
        for written in ["/home/me/work/*", "/home/me/work/**", "/home/me/work/*/", "/home/me/work//**/*", "/home/me/work/"] {
            assert_eq!(MatchRule::folder(written), MatchRule::Folder("/home/me/work".into()), "{written}");
        }
        assert_eq!(MatchRule::folder("C:/Users/me/work/*"), MatchRule::Folder("C:/Users/me/work".into()));
        let mut all = sample();
        all.workspaces[1].rules = vec![MatchRule::Folder("/home/me/work/*".into())];
        all.tidy();
        assert_eq!(all.validate(), Ok(()));
        let resolved = all.resolve("github.com/other/app", Some("/home/me/work/app"), None);
        assert_eq!(resolved.workspace.id, id("work"));
    }

    #[test]
    fn the_file_reads_back() {
        let mut all = sample();
        all.workspaces[1].server = Some(ServerBinding { url: "https://silt.acme.dev".into(), org: Some("acme".into()) });
        all.workspaces[1].env.insert("CLAUDE_CONFIG_DIR".into(), "/home/me/.config/silt/work/claude".into());
        let json = serde_json::to_string_pretty(&all).unwrap();
        assert!(json.contains(r#""kind": "remote""#), "{json}");
        assert_eq!(serde_json::from_str::<Workspaces>(&json).unwrap(), all);
        let minimal: Workspaces =
            serde_json::from_str(r#"{"default":"default","workspaces":[{"id":"default","name":"Default"}]}"#).unwrap();
        assert_eq!(minimal, Workspaces::single(None));
    }
}
