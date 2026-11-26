//! Whether a newer build is out: for a dev build, whether `main` has moved
//! past it in its clone; for a nightly one, whether the `nightly` release has;
//! for a stable one, whether a newer release is out (ADR 0100, ADR 0129).

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::info::{BuildInfo, Commit, Source};
use crate::shell::{self, Ran};

/// The branch dev and nightly builds follow.
pub const MAIN: &str = "main";

/// How many of the new commits a check lists.
const LISTED: usize = 50;

const GIT_WAIT: Duration = Duration::from_secs(20);
const FETCH_WAIT: Duration = Duration::from_secs(90);
const FETCH_RETRIES: usize = 3;
const FETCH_RETRY_AFTER: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Availability {
    UpToDate,
    /// `main` is `ahead` commits past the build; the newest come first.
    Behind {
        target: Commit,
        ahead: u32,
        commits: Vec<NewCommit>,
    },
    /// Nothing is offered, and why.
    Off {
        reason: OffReason,
    },
    /// A newer release than this stable build.
    Release {
        tag: String,
        url: String,
    },
    /// The `nightly` release is `ahead` commits past this nightly build; the
    /// newest come first.
    Nightly {
        ahead: u32,
        commits: Vec<NewCommit>,
        url: String,
    },
    /// The check itself failed, say offline.
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NewCommit {
    pub hash: Commit,
    pub subject: String,
    /// Committed at, in seconds since the epoch.
    pub time: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OffReason {
    /// A `cargo build` or a test's build, not an installed one.
    DebugBuild,
    /// The build doesn't say which clone or commit it came from.
    NoClone,
    CloneGone {
        path: String,
    },
    BuiltOffMain {
        branch: Option<String>,
    },
    BuiltWithChanges,
    OffMain {
        branch: Option<String>,
    },
    LocalChanges,
    NoUpstream,
    /// `main` has commits its upstream lacks, so it can't fast-forward.
    LocalCommits {
        count: u32,
    },
    /// The clone doesn't have the build's commit, or it isn't in `main`.
    UnknownBuild,
}

impl OffReason {
    /// Why there are no updates, as `silt update --check` says it.
    pub fn explain(&self) -> String {
        match self {
            Self::DebugBuild => "this is a debug build; install one with `just install` to get updates".to_owned(),
            Self::NoClone => "this build doesn't say which clone it came from".to_owned(),
            Self::CloneGone { path } => format!("the clone it was built from, {path}, is gone"),
            Self::BuiltOffMain { branch: Some(branch) } => format!("it was built from `{branch}`, not `{MAIN}`"),
            Self::BuiltOffMain { branch: None } => format!("it was built from a detached HEAD, not `{MAIN}`"),
            Self::BuiltWithChanges => "it was built with local changes".to_owned(),
            Self::OffMain { branch: Some(branch) } => format!("its clone is on `{branch}`, not `{MAIN}`"),
            Self::OffMain { branch: None } => format!("its clone has a detached HEAD, not `{MAIN}`"),
            Self::LocalChanges => "its clone has uncommitted changes".to_owned(),
            Self::NoUpstream => format!("`{MAIN}` in its clone has no upstream to follow"),
            Self::LocalCommits { count } => format!("`{MAIN}` in its clone has {count} commits its upstream lacks"),
            Self::UnknownBuild => format!("its clone doesn't have the commit it was built from on `{MAIN}`"),
        }
    }
}

impl Availability {
    pub fn explain(&self) -> String {
        match self {
            Self::UpToDate => "up to date".to_owned(),
            Self::Behind { target, ahead, .. } => format!("{MAIN} is {ahead} commits ahead, at {}", target.short()),
            Self::Off { reason } => format!("updates are off: {}", reason.explain()),
            Self::Release { tag, url } => format!("{tag} is out: {url}"),
            Self::Nightly { ahead, url, .. } => format!("the nightly release is {ahead} commits ahead: {url}"),
            Self::Failed { message } => format!("could not check: {message}"),
        }
    }
}

/// What git says of a clone, once fetched.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Facts {
    pub branch: Option<String>,
    pub dirty: bool,
    pub upstream: Option<Commit>,
    /// `main`'s commits its upstream lacks.
    pub local_commits: u32,
    /// The build's commit is in the upstream's history.
    pub built_in_upstream: bool,
    /// The upstream's commits the build lacks.
    pub ahead: u32,
    pub commits: Vec<NewCommit>,
}

/// What can be said of a dev build alone, before asking git: its clone and
/// commit, or why it has no updates.
pub fn updatable(build: &BuildInfo) -> Result<(&Source, &Commit), OffReason> {
    if build.debug {
        return Err(OffReason::DebugBuild);
    }
    let (Some(source), Some(commit)) = (&build.source, &build.commit) else {
        return Err(OffReason::NoClone);
    };
    if build.branch.as_deref() != Some(MAIN) {
        return Err(OffReason::BuiltOffMain { branch: build.branch.clone() });
    }
    if build.dirty {
        return Err(OffReason::BuiltWithChanges);
    }
    Ok((source, commit))
}

/// What a dev build's clone offers, from what git said of it.
pub fn decide(facts: &Facts) -> Availability {
    let off = |reason| Availability::Off { reason };
    if facts.branch.as_deref() != Some(MAIN) {
        return off(OffReason::OffMain { branch: facts.branch.clone() });
    }
    if facts.dirty {
        return off(OffReason::LocalChanges);
    }
    let Some(upstream) = &facts.upstream else {
        return off(OffReason::NoUpstream);
    };
    if facts.local_commits > 0 {
        return off(OffReason::LocalCommits { count: facts.local_commits });
    }
    if !facts.built_in_upstream {
        return off(OffReason::UnknownBuild);
    }
    if facts.ahead == 0 {
        return Availability::UpToDate;
    }
    Availability::Behind { target: upstream.clone(), ahead: facts.ahead, commits: facts.commits.clone() }
}

/// Whether `release`'s tag names a newer version than `version`, as
/// `v0.3.0` does `0.2.1`.
pub fn newer_release(version: &str, tag: &str) -> bool {
    let parts = |text: &str| -> Option<Vec<u64>> {
        let core = text.trim().trim_start_matches('v').split(['-', '+']).next()?;
        core.split('.').map(|part| part.parse().ok()).collect()
    };
    match (parts(version), parts(tag)) {
        (Some(have), Some(out)) => out > have,
        _ => false,
    }
}

/// The latest release's page, as GitHub's API answers
/// `repos/<repo>/releases/latest`.
pub fn release(build: &BuildInfo, answer: &serde_json::Value) -> Availability {
    let tag = answer.get("tag_name").and_then(serde_json::Value::as_str).unwrap_or_default();
    let url = answer.get("html_url").and_then(serde_json::Value::as_str).unwrap_or_default();
    if tag.is_empty() || !url.starts_with("https://") {
        return Availability::Failed { message: "the release service answered without a release".to_owned() };
    }
    if newer_release(&build.version, tag) {
        Availability::Release { tag: tag.to_owned(), url: url.to_owned() }
    } else {
        Availability::UpToDate
    }
}

/// The repository's API, where releases are learnt of. `SILT_UPDATE_API`
/// stands in for it in tests.
pub const API: &str = "https://api.github.com/repos/Urcra/silt";

/// The rolling release nightly builds come from (ADR 0129).
pub const NIGHTLY_TAG: &str = "nightly";
pub const NIGHTLY_URL: &str = "https://github.com/Urcra/silt/releases/tag/nightly";

/// Where stable builds learn of releases: `releases/latest`, which skips
/// prereleases such as the nightly.
pub fn releases_url(api: &str) -> String {
    format!("{api}/releases/latest")
}

/// Where a nightly build learns how far the `nightly` release is past it.
pub fn nightly_url(api: &str, built: &Commit) -> String {
    format!("{api}/compare/{built}...{NIGHTLY_TAG}")
}

/// The `nightly` release against this nightly build, as GitHub's API answers
/// `repos/<repo>/compare/<build>...nightly`.
pub fn nightly(answer: &serde_json::Value) -> Availability {
    let Some(ahead) = answer.get("ahead_by").and_then(serde_json::Value::as_u64) else {
        return Availability::Failed { message: "the release service answered without a comparison".to_owned() };
    };
    if ahead == 0 {
        return Availability::UpToDate;
    }
    // GitHub lists them oldest first.
    let mut commits: Vec<NewCommit> = answer
        .get("commits")
        .and_then(serde_json::Value::as_array)
        .map(|listed| listed.iter().filter_map(compared_commit).collect())
        .unwrap_or_default();
    commits.reverse();
    commits.truncate(LISTED);
    Availability::Nightly { ahead: u32::try_from(ahead).unwrap_or(u32::MAX), commits, url: NIGHTLY_URL.to_owned() }
}

fn compared_commit(listed: &serde_json::Value) -> Option<NewCommit> {
    let hash = Commit::parse(listed.get("sha")?.as_str()?)?;
    let commit = listed.get("commit")?;
    let message = commit.get("message").and_then(serde_json::Value::as_str).unwrap_or_default();
    let time = commit.pointer("/committer/date").and_then(serde_json::Value::as_str).and_then(utc_seconds).unwrap_or_default();
    Some(NewCommit { hash, subject: message.lines().next().unwrap_or_default().to_owned(), time })
}

/// Seconds since the epoch of a time as GitHub writes it, `2026-11-26T03:17:00Z`.
fn utc_seconds(text: &str) -> Option<i64> {
    let (date, time) = text.strip_suffix('Z')?.split_once('T')?;
    let numbers = |text: &str, separator: char| -> Option<Vec<i64>> {
        text.split(separator).map(|part| part.parse().ok()).collect::<Option<Vec<_>>>().filter(|parts| parts.len() == 3)
    };
    let (date, time) = (numbers(date, '-')?, numbers(time, ':')?);
    let (year, month, day) = (date[0], date[1], date[2]);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Days from the civil date, after Howard Hinnant's `days_from_civil`.
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let of_era_days = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    let days = era * 146_097 + of_era_days - 719_468;
    Some(days * 86_400 + time[0] * 3_600 + time[1] * 60 + time[2])
}

#[derive(Debug, thiserror::Error)]
pub enum CheckError {
    #[error("could not run git: {0}")]
    Run(#[source] std::io::Error),
    #[error("`git {args}` failed: {reason}")]
    Git { args: String, reason: String },
}

fn ran(source: &Source, args: &[&str], wait: Duration) -> Result<Ran, CheckError> {
    shell::run_within(shell::git(source, args), wait).map_err(CheckError::Run)
}

fn text(source: &Source, args: &[&str]) -> Result<String, CheckError> {
    let ran = ran(source, args, GIT_WAIT)?;
    if !ran.status.success() {
        return Err(failed(args, &ran));
    }
    Ok(ran.stdout.trim().to_owned())
}

/// Whether a fetch failed only because another fetch in the clone, such as
/// the daemon's own check beside `silt update`, held the remote's ref.
fn another_fetch_ran(stderr: &str) -> bool {
    stderr.contains("cannot lock ref")
}

fn failed(args: &[&str], ran: &Ran) -> CheckError {
    let reason = ran.stderr.lines().map(str::trim).rfind(|line| !line.is_empty()).unwrap_or("it failed").to_owned();
    CheckError::Git { args: args.join(" "), reason }
}

/// `true` for exit 0, `false` for 1, as `merge-base --is-ancestor` and
/// `diff --quiet` answer.
fn yes_or_no(source: &Source, args: &[&str]) -> Result<bool, CheckError> {
    let ran = ran(source, args, GIT_WAIT)?;
    match ran.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(failed(args, &ran)),
    }
}

/// Whether `path` is a clone at all.
pub fn is_clone(source: &Source) -> bool {
    if let Source::Local(path) = source
        && !path.is_dir()
    {
        return false;
    }
    text(source, &["rev-parse", "--is-inside-work-tree"]).is_ok_and(|answer| answer == "true")
}

/// Fetches `main`'s upstream, unless `fetch` is false, and says what the
/// clone holds against `built`.
pub fn facts(source: &Source, built: &Commit, fetch: bool) -> Result<Facts, CheckError> {
    let branch = text(source, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok().filter(|branch| !branch.is_empty());
    let dirty = !text(source, &["status", "--porcelain", "--untracked-files=no"])?.is_empty();
    let mut facts = Facts { branch, dirty, ..Facts::default() };
    let remote_key = format!("branch.{MAIN}.remote");
    let Ok(remote) = text(source, &["config", "--get", &remote_key]) else {
        return Ok(facts);
    };
    if remote.is_empty() || remote.starts_with('-') {
        return Ok(facts);
    }
    if fetch {
        let args = ["fetch", "--quiet", "--no-tags", "--", remote.as_str(), MAIN];
        let mut fetched = ran(source, &args, FETCH_WAIT)?;
        for _ in 0..FETCH_RETRIES {
            if fetched.status.success() || !another_fetch_ran(&fetched.stderr) {
                break;
            }
            std::thread::sleep(FETCH_RETRY_AFTER);
            fetched = ran(source, &args, FETCH_WAIT)?;
        }
        if !fetched.status.success() {
            return Err(failed(&args, &fetched));
        }
    }
    let upstream_ref = format!("{MAIN}@{{upstream}}");
    let Some(upstream) =
        text(source, &["rev-parse", "--verify", "--quiet", &upstream_ref]).ok().and_then(|hash| Commit::parse(&hash))
    else {
        return Ok(facts);
    };
    let local = format!("{upstream_ref}..{MAIN}");
    facts.local_commits = text(source, &["rev-list", "--count", &local])?.parse().unwrap_or(0);
    let built_commit = format!("{built}^{{commit}}");
    let known = yes_or_no(source, &["cat-file", "-e", &built_commit]).unwrap_or(false);
    facts.built_in_upstream = known && yes_or_no(source, &["merge-base", "--is-ancestor", built.as_str(), upstream.as_str()])?;
    if facts.built_in_upstream {
        let range = format!("{built}..{upstream}");
        facts.ahead = text(source, &["rev-list", "--count", &range])?.parse().unwrap_or(0);
        let listed = format!("--max-count={LISTED}");
        let log = text(source, &["log", &listed, "--format=%H%x1f%ct%x1f%s", &range])?;
        facts.commits = log.lines().filter_map(new_commit).collect();
    }
    facts.upstream = Some(upstream);
    Ok(facts)
}

fn new_commit(line: &str) -> Option<NewCommit> {
    let mut fields = line.splitn(3, '\u{1f}');
    let hash = Commit::parse(fields.next()?)?;
    let time = fields.next()?.parse().ok()?;
    Some(NewCommit { hash, time, subject: fields.next().unwrap_or_default().to_owned() })
}

/// Whether a dev build has an update, fetching first when `fetch`.
pub fn dev(build: &BuildInfo, fetch: bool) -> Availability {
    let (source, built) = match updatable(build) {
        Ok(found) => found,
        Err(reason) => return Availability::Off { reason },
    };
    if !is_clone(source) {
        return Availability::Off { reason: OffReason::CloneGone { path: source.shown() } };
    }
    match facts(source, built, fetch) {
        Ok(facts) => decide(&facts),
        Err(error) => Availability::Failed { message: error.to_string() },
    }
}

/// Whether the desktop app built from `from` differs from one built from
/// `to`, so a window running it needs a relaunch to run `to`. A commit the
/// clone doesn't know is taken to differ.
pub fn desktop_differs(source: &Source, from: &Commit, to: &Commit) -> bool {
    if from.same(to) {
        return false;
    }
    let range = [from.as_str(), to.as_str()];
    let args = ["diff", "--quiet", range[0], range[1], "--", "crates/silt-desktop"];
    !yes_or_no(source, &args).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(text: &str) -> Commit {
        Commit::parse(text).expect("hex")
    }

    #[test]
    fn only_a_fetch_beside_another_is_tried_again() {
        let beside = "error: cannot lock ref 'refs/remotes/origin/main': is at af60b6ca but expected b736ddb7\n";
        assert!(another_fetch_ran(beside));
        assert!(!another_fetch_ran("fatal: unable to access 'https://example.com/': Could not resolve host\n"));
        assert!(!another_fetch_ran("fatal: couldn't find remote ref main\n"));
    }

    fn on_main() -> Facts {
        Facts { branch: Some(MAIN.into()), upstream: Some(commit("f00ba12f00ba12")), built_in_upstream: true, ..Facts::default() }
    }

    #[test]
    fn main_past_the_build_is_an_update() {
        let commits = vec![NewCommit { hash: commit("f00ba12f00ba12"), subject: "feat: x".into(), time: 1 }];
        let facts = Facts { ahead: 3, commits: commits.clone(), ..on_main() };
        assert_eq!(decide(&facts), Availability::Behind { target: commit("f00ba12f00ba12"), ahead: 3, commits });
        assert_eq!(decide(&on_main()), Availability::UpToDate);
    }

    #[test]
    fn a_clone_off_main_or_with_changes_is_left_alone() {
        let off = |facts: Facts| match decide(&facts) {
            Availability::Off { reason } => reason,
            other => panic!("expected off, got {other:?}"),
        };
        assert_eq!(
            off(Facts { branch: Some("feat/x".into()), ..on_main() }),
            OffReason::OffMain { branch: Some("feat/x".into()) }
        );
        assert_eq!(off(Facts { branch: None, ..on_main() }), OffReason::OffMain { branch: None });
        assert_eq!(off(Facts { dirty: true, ..on_main() }), OffReason::LocalChanges);
        assert_eq!(off(Facts { upstream: None, ..on_main() }), OffReason::NoUpstream);
        assert_eq!(off(Facts { local_commits: 2, ..on_main() }), OffReason::LocalCommits { count: 2 });
        assert_eq!(off(Facts { built_in_upstream: false, ..on_main() }), OffReason::UnknownBuild);
    }

    #[test]
    fn a_build_off_main_or_with_changes_has_no_updates() {
        use crate::info::Channel;
        let build = BuildInfo {
            version: "0.0.1".into(),
            channel: Channel::Dev,
            commit: Some(commit("a1b2c3d4")),
            branch: Some(MAIN.into()),
            dirty: false,
            source: Some(Source::Local("/src/silt".into())),
            debug: false,
        };
        assert!(updatable(&build).is_ok());
        assert_eq!(updatable(&BuildInfo { debug: true, ..build.clone() }), Err(OffReason::DebugBuild));
        assert_eq!(updatable(&BuildInfo { dirty: true, ..build.clone() }), Err(OffReason::BuiltWithChanges));
        assert_eq!(
            updatable(&BuildInfo { branch: Some("x".into()), ..build.clone() }),
            Err(OffReason::BuiltOffMain { branch: Some("x".into()) })
        );
        assert_eq!(updatable(&BuildInfo { source: None, ..build }), Err(OffReason::NoClone));
    }

    #[test]
    fn a_release_is_newer_by_its_numbers() {
        assert!(newer_release("0.2.1", "v0.3.0"));
        assert!(newer_release("0.2.1", "v0.10.0"));
        assert!(!newer_release("0.3.0", "v0.3.0"));
        assert!(!newer_release("0.3.0", "v0.2.9"));
        assert!(!newer_release("0.3.0", "nightly"));
    }

    #[test]
    fn github_times_are_seconds_since_the_epoch() {
        assert_eq!(utc_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(utc_seconds("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(utc_seconds("2026-11-26T03:17:05Z"), Some(1_795_663_025));
        assert_eq!(utc_seconds("2026-11-26T03:17:05+01:00"), None);
        assert_eq!(utc_seconds("2026-13-26T03:17:05Z"), None);
    }

    #[test]
    fn the_nightly_past_the_build_lists_its_commits_newest_first() {
        let listed = |sha: &str, message: &str| serde_json::json!({"sha": sha, "commit": {"message": message, "committer": {"date": "1970-01-01T00:01:00Z"}}});
        let answer = serde_json::json!({
            "status": "ahead",
            "ahead_by": 2,
            "commits": [listed("a1b2c3d4e5f6", "feat: older\n\nbody"), listed("b2c3d4e5f6a1", "fix: newer")],
        });
        let Availability::Nightly { ahead, commits, url } = nightly(&answer) else { panic!("an update") };
        assert_eq!((ahead, url.as_str()), (2, NIGHTLY_URL));
        let subjects: Vec<_> = commits.iter().map(|commit| (commit.hash.short(), commit.subject.as_str(), commit.time)).collect();
        assert_eq!(subjects, [("b2c3d4e", "fix: newer", 60), ("a1b2c3d", "feat: older", 60)]);

        assert_eq!(nightly(&serde_json::json!({"status": "identical", "ahead_by": 0, "commits": []})), Availability::UpToDate);
        assert_eq!(nightly(&serde_json::json!({"status": "behind", "ahead_by": 0, "behind_by": 3})), Availability::UpToDate);
        assert!(matches!(nightly(&serde_json::json!({"message": "Not Found"})), Availability::Failed { .. }));
    }
}
