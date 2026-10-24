//! Project hooks for managed checkouts: `.agents/prepare` runs when a
//! checkout mounts and `.agents/linked` lists git-ignored paths shared
//! between all checkouts on the machine. A file under `.silt/` replaces
//! its `.agents/` counterpart.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use crate::{GitError, Repo};

const OVERRIDE_DIR: &str = ".silt";
const SHARED_DIR: &str = ".agents";

/// The project file `name` in `checkout`, preferring `.silt/<name>`.
pub fn project_file(checkout: &Path, name: &str) -> Option<PathBuf> {
    [OVERRIDE_DIR, SHARED_DIR].into_iter().map(|dir| checkout.join(dir).join(name)).find(|path| path.is_file())
}

/// Paths listed in a `linked` file: one repo-relative path per line, `#`
/// starting a comment. Paths that would leave the checkout are dropped.
pub fn parse_linked(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.split_once('#').map_or(line, |(path, _)| path).trim())
        .map(|path| path.trim_start_matches("./").trim_end_matches('/'))
        .filter(|path| !path.is_empty())
        .filter(|path| Path::new(path).components().all(|component| matches!(component, Component::Normal(_))))
        .map(str::to_owned)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkOutcome {
    Linked {
        path: String,
        seeded: bool,
    },
    AlreadyLinked {
        path: String,
    },
    /// The path is tracked or not ignored, so linking it would change the
    /// thread's files.
    NotIgnored {
        path: String,
    },
    /// Something other than the link is already there.
    Occupied {
        path: String,
    },
}

/// Links each path in the checkout's `linked` file to `<shared>/<path>`.
/// A shared path that doesn't exist yet is seeded from the source clone
/// when it has one, or else starts as an empty directory.
pub fn link_shared_paths(checkout: &Repo, source: &Path, shared: &Path) -> Result<Vec<LinkOutcome>, GitError> {
    let Some(file) = project_file(checkout.root(), "linked") else {
        return Ok(Vec::new());
    };
    let paths = parse_linked(&fs::read_to_string(file)?);
    let is_linked = |path: &str| fs::read_link(checkout.root().join(path)).is_ok_and(|current| current == shared.join(path));
    // Git refuses to look past a symlink, so linked paths aren't asked about.
    let candidates: Vec<String> =
        paths.iter().filter(|path| !is_linked(path)).flat_map(|path| [path.clone(), format!("{path}/")]).collect();
    let ignored = checkout.ignored(&candidates)?;
    let mut outcomes = Vec::new();
    let mut excluded = Vec::new();
    for path in paths {
        if is_linked(&path) {
            excluded.push(path.clone());
            outcomes.push(LinkOutcome::AlreadyLinked { path });
            continue;
        }
        if !ignored.contains(&path) && !ignored.contains(&format!("{path}/")) {
            outcomes.push(LinkOutcome::NotIgnored { path });
            continue;
        }
        let link = checkout.root().join(&path);
        let target = shared.join(&path);
        if link.symlink_metadata().is_ok() {
            outcomes.push(LinkOutcome::Occupied { path });
            continue;
        }
        let seeded = if target.symlink_metadata().is_ok() { false } else { seed(&source.join(&path), &target)? };
        fs::create_dir_all(link.parent().expect("joined onto the checkout"))?;
        silt_os::link(&target, &link)?;
        excluded.push(path.clone());
        outcomes.push(LinkOutcome::Linked { path, seeded });
    }
    exclude(checkout, &excluded)?;
    Ok(outcomes)
}

/// Creates `target` from the source clone's copy, or as an empty directory.
/// Returns whether anything was copied.
fn seed(from: &Path, target: &Path) -> io::Result<bool> {
    fs::create_dir_all(target.parent().expect("joined onto the shared dir"))?;
    if from.symlink_metadata().is_err() {
        fs::create_dir_all(target)?;
        return Ok(false);
    }
    let status = Command::new("cp").args(["-a", "--reflink=auto"]).arg(from).arg(target).stdin(Stdio::null()).status()?;
    if status.success() {
        Ok(true)
    } else {
        Err(io::Error::other(format!("could not copy {} to {}", from.display(), target.display())))
    }
}

/// Ignores the links themselves: git sees a symlink as a file, so an ignore
/// rule such as `node_modules/` that only matches directories misses it.
/// A gitignore rule matching exactly `path` from the repository's root:
/// wildcards, backslashes and trailing spaces in the name are escaped.
fn ignore_rule(path: &str) -> String {
    let mut rule = String::from("/");
    for ch in path.chars() {
        if matches!(ch, '*' | '?' | '[' | '\\') {
            rule.push('\\');
        }
        rule.push(ch);
    }
    let kept = rule.trim_end_matches(' ').len();
    let spaces = rule.len() - kept;
    rule.truncate(kept);
    rule.push_str(&"\\ ".repeat(spaces));
    rule
}

fn exclude(checkout: &Repo, paths: &[String]) -> Result<(), GitError> {
    if paths.is_empty() {
        return Ok(());
    }
    let exclude = String::from_utf8_lossy(&checkout.git(&["rev-parse", "--git-path", "info/exclude"], None)?).trim().to_owned();
    let exclude = checkout.root().join(exclude);
    let existing = fs::read_to_string(&exclude).unwrap_or_default();
    let missing: Vec<String> =
        paths.iter().map(|path| ignore_rule(path)).filter(|rule| !existing.lines().any(|line| line == rule)).collect();
    if missing.is_empty() {
        return Ok(());
    }
    let mut text = existing;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    for rule in missing {
        text.push_str(&rule);
        text.push('\n');
    }
    fs::create_dir_all(exclude.parent().expect("info/exclude has a parent"))?;
    fs::write(exclude, text)?;
    Ok(())
}

#[derive(Debug)]
pub enum PrepareOutcome {
    Ran { script: PathBuf, status: ExitStatus },
    NotExecutable { script: PathBuf },
}

/// Runs the checkout's `prepare` script from the checkout root, with its
/// output going where this process's goes and no input.
pub fn run_prepare(checkout: &Path, envs: &[(&str, &str)]) -> io::Result<Option<PrepareOutcome>> {
    let Some(script) = project_file(checkout, "prepare") else {
        return Ok(None);
    };
    let mut command = if cfg!(windows) {
        // Windows can't run a script by itself; Git for Windows' shell can,
        // as it runs git's hooks.
        let Some(shell) = std::env::var_os("PATH").and_then(|path| silt_os::find_program("sh", &path)) else {
            return Ok(Some(PrepareOutcome::NotExecutable { script }));
        };
        let mut command = Command::new(shell);
        command.arg(&script);
        command
    } else {
        if !silt_os::is_executable(&fs::metadata(&script)?) {
            return Ok(Some(PrepareOutcome::NotExecutable { script }));
        }
        Command::new(&script)
    };
    let status = command.current_dir(checkout).envs(envs.iter().copied()).stdin(Stdio::null()).status()?;
    Ok(Some(PrepareOutcome::Ran { script, status }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linked_files_list_relative_paths() {
        let text = "# shared between checkouts\nnode_modules/\n./target # build output\n\n/etc/passwd\n../up\nweb/node_modules\n";
        assert_eq!(parse_linked(text), vec!["node_modules", "target", "web/node_modules"]);
    }

    #[test]
    fn ignore_rules_match_names_literally() {
        assert_eq!(ignore_rule("node_modules"), "/node_modules");
        assert_eq!(ignore_rule("a*b?[c]\\d"), "/a\\*b\\?\\[c]\\\\d");
        assert_eq!(ignore_rule("trailing  "), "/trailing\\ \\ ");
    }
}
