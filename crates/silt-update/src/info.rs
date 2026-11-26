//! Which build this is: its channel, commit and the clone it was built
//! from, as the build script found them (ADR 0100, ADR 0129).

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// `silt --version`'s answer: `0.0.1 (dev a1b2c3d)`.
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("SILT_BUILD_LABEL"), ")");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    /// A tagged release, installed from its binaries.
    Stable,
    /// The `nightly` release, built from `main` each night.
    Nightly,
    /// A build of a clone, updated by pulling and building it again.
    Dev,
}

impl Channel {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "stable" => Some(Self::Stable),
            "nightly" => Some(Self::Nightly),
            "dev" => Some(Self::Dev),
            _ => None,
        }
    }
}

/// A commit's name in hex, checked before it becomes an argument to git.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
#[serde(try_from = "String", into = "String")]
pub struct Commit(String);

impl Commit {
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let hex = text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        ((7..=64).contains(&text.len()) && hex).then(|| Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn short(&self) -> &str {
        &self.0[..7]
    }

    /// Whether both name the same commit, one perhaps abbreviated.
    pub fn same(&self, other: &Commit) -> bool {
        self.0.starts_with(&other.0) || other.0.starts_with(&self.0)
    }
}

impl TryFrom<String> for Commit {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text).ok_or_else(|| format!("not a commit: {text:?}"))
    }
}

impl From<Commit> for String {
    fn from(commit: Commit) -> Self {
        commit.0
    }
}

impl fmt::Display for Commit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where a dev build's clone is: on this machine, or, for a Windows
/// build made by install-windows.sh, in a WSL distribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Local(PathBuf),
    Wsl { distribution: String, path: String },
}

impl Source {
    /// A path, or `wsl://<distribution>/<path>`.
    pub fn parse(text: &str) -> Option<Self> {
        if text.is_empty() || text.chars().any(char::is_control) {
            return None;
        }
        let Some(rest) = text.strip_prefix("wsl://") else {
            return Some(Self::Local(PathBuf::from(text)));
        };
        let (distribution, path) = rest.split_once('/')?;
        let named = !distribution.is_empty()
            && distribution.len() <= 64
            && distribution.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
        let path = format!("/{path}");
        let plain = path.len() > 1 && !path.split('/').any(|part| part == "..");
        (named && plain).then(|| Self::Wsl { distribution: distribution.to_owned(), path })
    }

    /// As people read it: the path, and the distribution it's in.
    pub fn shown(&self) -> String {
        match self {
            Self::Local(path) => path.display().to_string(),
            Self::Wsl { distribution, path } => format!("{path} in WSL ({distribution})"),
        }
    }
}

/// What a build knows of itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildInfo {
    pub version: String,
    pub channel: Channel,
    pub commit: Option<Commit>,
    pub branch: Option<String>,
    /// Tracked files had changes when it was built.
    pub dirty: bool,
    /// The clone a dev build came from; other channels have none.
    pub source: Option<Source>,
    /// A debug build, from `cargo build` or a test, that nothing pointed at
    /// a clone: updating `cargo install`'s copy from it would be a surprise.
    pub debug: bool,
}

impl BuildInfo {
    /// This program's build. A debug build reads the same variables when it
    /// runs too, so a test can make it an older build of another clone.
    pub fn current() -> Self {
        let baked = |name: &str| -> Option<String> {
            let value = match name {
                "SILT_BUILD_COMMIT" => env!("SILT_BUILD_COMMIT"),
                "SILT_BUILD_BRANCH" => env!("SILT_BUILD_BRANCH"),
                "SILT_BUILD_DIRTY" => env!("SILT_BUILD_DIRTY"),
                "SILT_BUILD_SOURCE" => env!("SILT_BUILD_SOURCE"),
                "SILT_BUILD_CHANNEL" => env!("SILT_BUILD_CHANNEL"),
                _ => "",
            };
            (!value.is_empty()).then(|| value.to_owned())
        };
        let debug = cfg!(debug_assertions);
        let given = |name: &str| if debug { std::env::var(name).ok().filter(|value| !value.is_empty()) } else { None };
        // Pointed at another clone or channel, it's another build: nothing
        // baked in holds.
        let pointed = given("SILT_BUILD_SOURCE").is_some() || given("SILT_BUILD_CHANNEL").is_some();
        let mut info = if pointed { Self::from_vars(given) } else { Self::from_vars(baked) };
        info.debug = debug && !pointed;
        info
    }

    /// A build from its `SILT_BUILD_*` variables.
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> Self {
        let commit = var("SILT_BUILD_COMMIT").and_then(|commit| Commit::parse(&commit));
        let source = var("SILT_BUILD_SOURCE").and_then(|source| Source::parse(&source));
        let fallback = if commit.is_some() && source.is_some() { Channel::Dev } else { Channel::Stable };
        let channel = var("SILT_BUILD_CHANNEL").and_then(|channel| Channel::parse(&channel)).unwrap_or(fallback);
        Self {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            channel,
            commit,
            branch: var("SILT_BUILD_BRANCH").filter(|branch| !branch.trim().is_empty()),
            dirty: var("SILT_BUILD_DIRTY").is_some_and(|dirty| matches!(dirty.trim(), "1" | "true" | "yes")),
            // A release is built in CI's checkout, which no one updates.
            source: source.filter(|_| channel == Channel::Dev),
            debug: false,
        }
    }

    pub fn view(&self) -> BuildView {
        BuildView {
            version: self.version.clone(),
            channel: self.channel,
            commit: self.commit.clone(),
            branch: self.branch.clone(),
            dirty: self.dirty,
            source: self.source.as_ref().map(Source::shown),
        }
    }
}

/// A build, as the web UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct BuildView {
    pub version: String,
    pub channel: Channel,
    pub commit: Option<Commit>,
    pub branch: Option<String>,
    pub dirty: bool,
    /// The clone it was built from.
    pub source: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| pairs.iter().find(|(key, _)| *key == name).map(|(_, value)| (*value).to_owned())
    }

    #[test]
    fn a_build_of_a_clone_is_dev_and_one_without_is_stable() {
        let dev = BuildInfo::from_vars(vars(&[("SILT_BUILD_COMMIT", "a1b2c3d4e5f6"), ("SILT_BUILD_SOURCE", "/src/silt")]));
        assert_eq!(dev.channel, Channel::Dev);
        assert_eq!(dev.source, Some(Source::Local("/src/silt".into())));
        assert_eq!(BuildInfo::from_vars(vars(&[])).channel, Channel::Stable);
    }

    #[test]
    fn a_release_names_its_channel_and_keeps_no_clone() {
        for (name, channel) in [("stable", Channel::Stable), ("nightly", Channel::Nightly)] {
            let built = BuildInfo::from_vars(vars(&[
                ("SILT_BUILD_COMMIT", "a1b2c3d4e5f6"),
                ("SILT_BUILD_SOURCE", "/home/runner/work/silt/silt"),
                ("SILT_BUILD_CHANNEL", name),
            ]));
            assert_eq!(built.channel, channel);
            assert_eq!(built.source, None, "{name}: CI's checkout isn't a clone to update");
            assert!(built.commit.is_some());
        }
    }

    #[test]
    fn only_hex_is_a_commit() {
        assert_eq!(Commit::parse("A1B2C3D"), None);
        assert_eq!(Commit::parse("a1b2c3"), None);
        assert_eq!(Commit::parse("--upload-pack=x"), None);
        let commit = Commit::parse("a1b2c3d4e5f60718").expect("hex");
        assert_eq!(commit.short(), "a1b2c3d");
        assert!(commit.same(&Commit::parse("a1b2c3d").expect("hex")));
    }

    #[test]
    fn a_wsl_source_names_a_distribution_and_an_absolute_path() {
        assert_eq!(
            Source::parse("wsl://Ubuntu-24.04/home/me/silt"),
            Some(Source::Wsl { distribution: "Ubuntu-24.04".into(), path: "/home/me/silt".into() })
        );
        for wrong in ["wsl://", "wsl://Ubuntu", "wsl://Ub untu/home", "wsl://Ubuntu/../etc", "wsl:///home", "/a\nb"] {
            assert_eq!(Source::parse(wrong), None, "{wrong}");
        }
    }

    #[test]
    fn this_build_names_itself() {
        assert!(VERSION.starts_with(env!("CARGO_PKG_VERSION")));
        assert!(BuildInfo::current().debug || std::env::var_os("SILT_BUILD_SOURCE").is_some());
    }
}
