//! `refs/notes/silt` (ADR 0042), and what notes and `silt blame` read of a
//! repository: the lines a commit changed, and `git blame`'s answer.

use silt_types::GitOid;

use crate::{GitError, Repo};

pub const NOTES_REF: &str = "refs/notes/silt";

/// Where a remote's notes are fetched to before they're merged into ours.
fn fetched_ref(remote: &str) -> String {
    format!("refs/notes/silt-from/{remote}")
}

/// The identity a notes commit takes when the repository has none.
const FALLBACK_IDENTITY: [(&str, &str); 2] = [("user.name", "Silt"), ("user.email", "silt@localhost")];

/// A line of the file as it is now, as `git blame` placed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlamedLine {
    pub commit: GitOid,
    /// Its number in `commit`, from 1, and the file's path there.
    pub line: u32,
    pub path: String,
    pub text: String,
}

impl Repo {
    /// `git config silt.notes` in this clone, for projects no server knows.
    pub fn notes_config(&self) -> Option<String> {
        let value = self.git(&["config", "--get", "silt.notes"], None).ok()?;
        Some(String::from_utf8_lossy(&value).trim().to_owned())
    }

    pub fn note(&self, commit: GitOid) -> Result<Option<String>, GitError> {
        let commit = commit.to_string();
        match self.git(&["notes", "--ref", NOTES_REF, "show", &commit], None) {
            Ok(text) => Ok(Some(String::from_utf8_lossy(&text).into_owned())),
            Err(GitError::Command { stderr, .. }) if stderr.contains("no note found") => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub fn add_note(&self, commit: GitOid, text: &str) -> Result<(), GitError> {
        let commit = commit.to_string();
        self.notes_git(&["notes", "--ref", NOTES_REF, "add", "-f", "-F", "-", &commit], Some(text.as_bytes()))?;
        Ok(())
    }

    /// Brings `remote`'s notes into ours, ours winning for a commit both
    /// noted, then pushes ours there. A remote without notes yet just gets
    /// ours.
    pub fn sync_notes(&self, remote: &str) -> Result<(), GitError> {
        let fetched = fetched_ref(remote);
        let refspec = format!("+{NOTES_REF}:{fetched}");
        match self.git(&["fetch", "--quiet", "--no-tags", remote, &refspec], None) {
            Ok(_) => {
                self.notes_git(&["notes", "--ref", NOTES_REF, "merge", "--quiet", "-s", "ours", &fetched], None)?;
            }
            Err(GitError::Command { stderr, .. }) if stderr.contains("couldn't find remote ref") => {}
            Err(error) => return Err(error),
        }
        self.git(&["push", "--quiet", remote, &format!("{NOTES_REF}:{NOTES_REF}")], None)?;
        Ok(())
    }

    pub fn has_remote(&self, name: &str) -> bool {
        self.remote_url(name).ok().flatten().is_some()
    }

    /// The lines `commit` added or changed against its first parent, by
    /// path, numbered as in the commit.
    pub fn changed_lines(&self, commit: GitOid) -> Result<Vec<(String, Vec<u32>)>, GitError> {
        let commit = commit.to_string();
        let parent = self.git(&["rev-parse", "--verify", "--quiet", &format!("{commit}^1")], None).ok();
        let parent = match parent {
            Some(parent) => String::from_utf8_lossy(&parent).trim().to_owned(),
            None => String::from_utf8_lossy(&self.git(&["hash-object", "-t", "tree", "--stdin"], Some(b""))?).trim().to_owned(),
        };
        let diff = self.git(
            &["-c", "core.quotePath=false", "diff", "--no-color", "--no-ext-diff", "--no-renames", "-U0", &parent, &commit],
            None,
        )?;
        Ok(changed_lines(&String::from_utf8_lossy(&diff)))
    }

    /// The text of `path` in `commit`; `None` when it isn't there or isn't text.
    pub fn text_at(&self, commit: GitOid, path: &str) -> Option<String> {
        let bytes = self.git(&["cat-file", "blob", &format!("{commit}:{path}")], None).ok()?;
        String::from_utf8(bytes).ok()
    }

    /// `git blame` of the file at `path` as it is in the worktree.
    pub fn blame(&self, path: &str) -> Result<Vec<BlamedLine>, GitError> {
        let output = self.git(&["blame", "--porcelain", "--", path], None)?;
        Ok(parse_porcelain(&String::from_utf8_lossy(&output)))
    }

    /// A notes command, which commits: with a stand-in identity when the
    /// repository has none.
    fn notes_git(&self, args: &[&str], stdin: Option<&[u8]>) -> Result<Vec<u8>, GitError> {
        if self.git(&["var", "GIT_COMMITTER_IDENT"], None).is_ok() {
            return self.git(args, stdin);
        }
        let identity: Vec<String> = FALLBACK_IDENTITY.iter().map(|(key, value)| format!("{key}={value}")).collect();
        let with: Vec<&str> = identity.iter().flat_map(|setting| ["-c", setting.as_str()]).chain(args.iter().copied()).collect();
        self.git(&with, stdin)
    }
}

/// The new side's lines of each file in a `-U0` diff. Quoted paths, which
/// hold characters notes leave out, are skipped.
pub fn changed_lines(diff: &str) -> Vec<(String, Vec<u32>)> {
    let mut files: Vec<(String, Vec<u32>)> = Vec::new();
    let mut current: Option<usize> = None;
    for line in diff.lines() {
        if let Some(path) = line.strip_prefix("+++ ") {
            current = path.strip_prefix("b/").map(|path| {
                files.push((path.to_owned(), Vec::new()));
                files.len() - 1
            });
        } else if line.starts_with("diff --git ") {
            current = None;
        } else if let (Some(index), Some(hunk)) = (current, line.strip_prefix("@@ ")) {
            let Some(new) = hunk.split(' ').find_map(|part| part.strip_prefix('+')) else {
                continue;
            };
            let (start, count) = match new.split_once(',') {
                Some((start, count)) => (start.parse::<u32>().ok(), count.parse::<u32>().ok()),
                None => (new.parse().ok(), Some(1)),
            };
            if let (Some(start), Some(count)) = (start, count) {
                files[index].1.extend(start..start.saturating_add(count));
            }
        }
    }
    files.retain(|(_, lines)| !lines.is_empty());
    files
}

fn parse_porcelain(output: &str) -> Vec<BlamedLine> {
    let mut paths: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut lines = Vec::new();
    let mut header: Option<(String, u32)> = None;
    for line in output.lines() {
        if let Some(text) = line.strip_prefix('\t') {
            if let Some((commit, number)) = header.take()
                && let Ok(oid) = commit.parse::<GitOid>()
            {
                let path = paths.get(&commit).cloned().unwrap_or_default();
                lines.push(BlamedLine { commit: oid, line: number, path, text: text.to_owned() });
            }
            continue;
        }
        if let Some(path) = line.strip_prefix("filename ") {
            if let Some((commit, _)) = &header {
                paths.insert(commit.clone(), path.to_owned());
            }
            continue;
        }
        let mut words = line.split(' ');
        if let (Some(commit), Some(original)) = (words.next(), words.next())
            && commit.len() >= 40
            && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
            && let Ok(original) = original.parse()
        {
            header = Some((commit.to_owned(), original));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_context_diff_gives_each_files_new_lines() {
        let diff = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,0 +2,3 @@\n+x\n+y\n+z\n@@ -9 +12 @@\n-old\n+new\n\
                    diff --git a/gone b/gone\n--- a/gone\n+++ /dev/null\n@@ -1 +0,0 @@\n-bye\n\
                    diff --git a/new b/new\n--- /dev/null\n+++ b/new\n@@ -0,0 +1 @@\n+hi\n";
        assert_eq!(changed_lines(diff), [("src/a.rs".to_owned(), vec![2, 3, 4, 12]), ("new".to_owned(), vec![1])]);
    }

    #[test]
    fn porcelain_names_each_lines_commit_line_and_path() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let output = format!(
            "{a} 1 1 2\nauthor X\nfilename old.rs\n\tone\n{a} 2 2\n\ttwo\n{b} 5 3 1\nauthor Y\nfilename new.rs\n\tthree\n"
        );
        let lines = parse_porcelain(&output);
        let shown: Vec<(char, u32, &str, &str)> = lines
            .iter()
            .map(|line| (line.commit.to_string().chars().next().unwrap(), line.line, line.path.as_str(), line.text.as_str()))
            .collect();
        assert_eq!(shown, [('a', 1, "old.rs", "one"), ('a', 2, "old.rs", "two"), ('b', 5, "new.rs", "three")]);
    }
}
