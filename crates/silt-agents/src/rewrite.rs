/// A session as recorded elsewhere, possibly continued across machines
/// and forks, to be written back under one id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordedSession {
    /// The id to restore it as.
    pub id: String,
    /// Ids, working directories and session directories it was recorded
    /// under; each is rewritten to the new one.
    pub former_ids: Vec<String>,
    pub former_cwds: Vec<String>,
    /// Checkout roots, rewritten to the new checkout root after the cwds.
    pub former_roots: Vec<String>,
    pub former_dirs: Vec<String>,
    pub main: Vec<String>,
    pub subagents: Vec<(String, Vec<String>)>,
    /// Side files by path relative to the session directory.
    pub attachments: Vec<(String, Vec<u8>)>,
}

/// Replaces the paths and ids a session was recorded under with new ones.
#[derive(Debug, Clone, Default)]
pub struct Rewrites {
    pairs: Vec<(String, String)>,
}

impl Rewrites {
    /// Former session directories, cwds, roots and ids, in that order of
    /// precedence, mapped to the new ones.
    pub fn new(session: &RecordedSession, new_dir: Option<&str>, new_cwd: &str, new_root: &str) -> Self {
        let mut pairs = Vec::new();
        let mut add = |former: &[String], new: &str| {
            pairs.extend(former.iter().filter(|old| !old.is_empty()).map(|old| (old.clone(), new.to_owned())));
        };
        if let Some(new_dir) = new_dir {
            add(&session.former_dirs, new_dir);
        }
        add(&session.former_cwds, new_cwd);
        add(&session.former_roots, new_root);
        add(&session.former_ids, &session.id);
        // Longest first, so a path is never rewritten through a shorter path
        // it starts with.
        pairs.sort_by_key(|(old, _)| std::cmp::Reverse(old.len()));
        Self { pairs }
    }

    pub fn with(mut self, old: &str, new: &str) -> Self {
        self.pairs.push((old.to_owned(), new.to_owned()));
        self.pairs.sort_by_key(|(old, _)| std::cmp::Reverse(old.len()));
        self
    }

    pub fn apply(&self, text: &str) -> String {
        let pairs: Vec<(&str, &str)> = self.pairs.iter().map(|(old, new)| (old.as_str(), new.as_str())).collect();
        replace_once(text, &pairs)
    }

    pub fn lines(&self, lines: &[String]) -> String {
        lines.iter().map(|line| self.apply(line) + "\n").collect()
    }
}

/// Replaces every occurrence of each `old` with its `new` in one pass, so
/// replacements are never themselves rewritten. Earlier pairs win.
fn replace_once(text: &str, rewrites: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(ch) = rest.chars().next() {
        match rewrites.iter().find(|(old, _)| rest.starts_with(old)) {
            Some((old, new)) => {
                out.push_str(new);
                rest = &rest[old.len()..];
            }
            None => {
                out.push(ch);
                rest = &rest[ch.len_utf8()..];
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_never_apply_to_their_own_output() {
        let rewrites = [("/a/repo/old", "/a/repo/new"), ("/a/repo", "/a/repo/new")];
        assert_eq!(
            replace_once("cd /a/repo/old && cd /a/repo", &rewrites),
            "cd /a/repo/new && cd /a/repo/new"
        );
    }
}
