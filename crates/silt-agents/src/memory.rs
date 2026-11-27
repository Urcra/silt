//! What agents write down to read in later sessions (ADR 0135), read from
//! each agent's own store into one shape for the memory viewer.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::conventions::{parse_frontmatter, split_frontmatter};

/// The file indexing a folder of memories, a line a memory.
pub const INDEX: &str = "MEMORY.md";

/// Whose a store is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// One project's, read in that project only.
    Project,
    /// The person's, read in every project.
    Everywhere,
}

/// What a store is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A folder of `MEMORY.md` and a file a memory beside it, which the
    /// agent writes itself, as Claude Code and Gemini CLI keep them.
    Memory,
    /// A rules file the person writes, read whole every session; Gemini CLI
    /// also adds to its own.
    Rules,
    /// A summary the agent rebuilds from its sessions, read whole, which
    /// only the agent changes, as Codex's.
    Summary,
}

/// One of an agent's stores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    pub scope: Scope,
    pub kind: Kind,
    /// The folder of a [`Kind::Memory`] store, else the file.
    pub path: PathBuf,
}

impl Store {
    pub fn new(scope: Scope, kind: Kind, path: PathBuf) -> Self {
        Self { scope, kind, path }
    }

    /// Silt may change it: an agent's own summary is rewritten from its
    /// sessions, so a change there wouldn't last.
    pub fn editable(&self) -> bool {
        self.kind != Kind::Summary
    }
}

/// One memory, as its file holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Its file's name within the store.
    pub file: String,
    pub name: String,
    pub description: String,
    /// Claude Code's `user`, `feedback`, `project` or `reference`.
    pub kind: Option<String>,
    /// The file's text after its frontmatter.
    pub body: String,
    /// The whole file, as an edit starts from.
    pub text: String,
    /// When it last changed, in ms since the epoch: the frontmatter's
    /// `modified`, else the file's.
    pub modified_ms: Option<i64>,
    /// The agent's session that wrote it, when it says.
    pub session: Option<String>,
}

/// The memories in an indexed store, in its index's order, then the files
/// it doesn't list. A missing folder has none.
pub fn read_indexed(dir: &Path) -> io::Result<Vec<Entry>> {
    let names = match fs::read_dir(dir) {
        Ok(read) => read
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
            .filter(|name| is_entry_file(name))
            .collect::<Vec<_>>(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let index = fs::read_to_string(dir.join(INDEX)).unwrap_or_default();
    let listed = index_lines(&index);
    let mut ordered: Vec<String> = listed.iter().map(|line| line.file.clone()).filter(|file| names.contains(file)).collect();
    let mut rest: Vec<String> = names.iter().filter(|name| !ordered.contains(name)).cloned().collect();
    rest.sort();
    ordered.extend(rest);
    let mut entries = Vec::new();
    for file in ordered {
        let path = dir.join(&file);
        let text = fs::read_to_string(&path)?;
        let modified = fs::metadata(&path).ok().and_then(|meta| meta.modified().ok());
        let modified_ms = modified.and_then(|time| time.duration_since(UNIX_EPOCH).ok()).map(|since| since.as_millis() as i64);
        let line = listed.iter().find(|line| line.file == file);
        entries.push(parse_entry(&file, &text, line, modified_ms));
    }
    Ok(entries)
}

/// A memory file's fields, from its frontmatter where it has one, else
/// from its index line and its first lines.
pub fn parse_entry(file: &str, text: &str, line: Option<&IndexLine>, file_modified_ms: Option<i64>) -> Entry {
    let (front, body) = split_frontmatter(text);
    let fields = parse_frontmatter(front);
    let field = |keys: &[&str]| keys.iter().find_map(|key| fields.get(*key).filter(|value| !value.is_empty()).cloned());
    let body = body.trim_start_matches(['\n', '\r']).to_owned();
    let heading = body.lines().find_map(|line| line.strip_prefix("# ")).map(|title| title.trim().to_owned());
    let stem = file.strip_suffix(".md").unwrap_or(file).to_owned();
    let name = field(&["name"]).or_else(|| line.map(|line| line.title.clone())).or(heading).unwrap_or(stem);
    let description = field(&["description"]).or_else(|| line.and_then(|line| line.hook.clone())).unwrap_or_else(|| {
        body.lines().map(str::trim).find(|line| !line.is_empty() && !line.starts_with('#')).map(str::to_owned).unwrap_or_default()
    });
    let modified_ms = field(&["metadata.modified", "modified"]).and_then(|value| parse_time(&value)).or(file_modified_ms);
    Entry {
        file: file.to_owned(),
        name,
        description,
        kind: field(&["metadata.type", "type"]),
        body,
        text: text.to_owned(),
        modified_ms,
        session: field(&["metadata.originSessionId", "originSessionId"]),
    }
}

/// One line of `MEMORY.md`: `- [Title](file.md) — hook`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexLine {
    pub title: String,
    pub file: String,
    pub hook: Option<String>,
}

pub fn index_lines(index: &str) -> Vec<IndexLine> {
    index.lines().filter_map(index_line).collect()
}

fn index_line(line: &str) -> Option<IndexLine> {
    let rest = line.trim_start().strip_prefix(['-', '*'])?.trim_start().strip_prefix('[')?;
    let (title, rest) = rest.split_once("](")?;
    let (file, rest) = rest.split_once(')')?;
    let hook = rest.trim().trim_start_matches(['—', '–', '-', ':']).trim();
    Some(IndexLine {
        title: title.trim().to_owned(),
        file: file.trim().to_owned(),
        hook: (!hook.is_empty()).then(|| hook.to_owned()),
    })
}

/// Whether `name` can be a memory's file: a plain `.md` name in the store's
/// own folder, never the index.
pub fn is_entry_file(name: &str) -> bool {
    name.ends_with(".md")
        && name != INDEX
        && !name.starts_with('.')
        && !name.contains(['/', '\\'])
        && name.chars().all(|ch| !ch.is_control())
        && name.len() <= 255
}

/// Replaces the memory `file` in `dir` with `text`, whole.
pub fn write_entry(dir: &Path, file: &str, text: &str) -> io::Result<()> {
    let path = entry_path(dir, file)?;
    if !path.is_file() {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("no memory {file}")));
    }
    write_atomically(&path, text.as_bytes())
}

/// Replaces a whole-file store, such as a rules file, with `text`, making
/// it if it isn't there yet.
pub fn write_whole(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    write_atomically(path, text.as_bytes())
}

/// Removes the memory `file` from `dir` and its line from the index.
pub fn forget_entry(dir: &Path, file: &str) -> io::Result<()> {
    let path = entry_path(dir, file)?;
    fs::remove_file(&path)?;
    let index_path = dir.join(INDEX);
    let Ok(index) = fs::read_to_string(&index_path) else { return Ok(()) };
    let kept = without_line_for(&index, file);
    if kept != index {
        write_atomically(&index_path, kept.as_bytes())?;
    }
    Ok(())
}

/// `index` without the lines that point at `file`.
pub fn without_line_for(index: &str, file: &str) -> String {
    let mut kept = String::with_capacity(index.len());
    for line in index.split_inclusive('\n') {
        if index_line(line.trim_end()).is_some_and(|line| line.file == file) {
            continue;
        }
        kept.push_str(line);
    }
    kept
}

fn entry_path(dir: &Path, file: &str) -> io::Result<PathBuf> {
    if !is_entry_file(file) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{file:?} isn't a memory's file")));
    }
    Ok(dir.join(file))
}

fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no folder"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(dir)?;
    io::Write::write_all(&mut temporary, bytes)?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// An RFC 3339 time, such as `2026-11-26T10:30:00Z`, in ms since the epoch.
fn parse_time(value: &str) -> Option<i64> {
    crate::time::parse_timestamp(value.trim().trim_matches(['"', '\'']))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE: &str = "---\nname: feedback-testing\ndescription: Tests hit a real store\nmetadata:\n  type: feedback\n  modified: 2026-11-20T10:00:00Z\n  originSessionId: 0b1c\n---\n\nIntegration tests use a real store.\n**Why:** a mock hid a broken migration.\n";

    #[test]
    fn a_claude_memory_reads_its_frontmatter() {
        let entry = parse_entry("feedback-testing.md", CLAUDE, None, Some(1));
        assert_eq!(entry.name, "feedback-testing");
        assert_eq!(entry.description, "Tests hit a real store");
        assert_eq!(entry.kind.as_deref(), Some("feedback"));
        assert_eq!(entry.session.as_deref(), Some("0b1c"));
        assert_eq!(entry.modified_ms, crate::time::parse_timestamp("2026-11-20T10:00:00Z"));
        assert!(entry.body.starts_with("Integration tests"));
        assert_eq!(entry.text, CLAUDE);
    }

    #[test]
    fn a_memory_without_frontmatter_takes_its_index_line() {
        let line = index_line("- [Uses pnpm](pnpm.md) — never npm").unwrap();
        assert_eq!(line, IndexLine { title: "Uses pnpm".into(), file: "pnpm.md".into(), hook: Some("never npm".into()) });
        let entry = parse_entry("pnpm.md", "Install with pnpm.\n", Some(&line), Some(7));
        assert_eq!((entry.name.as_str(), entry.description.as_str()), ("Uses pnpm", "never npm"));
        assert_eq!((entry.kind, entry.modified_ms), (None, Some(7)));
        let bare = parse_entry("notes.md", "# Notes\n\nFirst line.\n", None, None);
        assert_eq!((bare.name.as_str(), bare.description.as_str()), ("Notes", "First line."));
    }

    #[test]
    fn a_store_reads_in_its_index_order_then_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(INDEX), "- [B](b.md) — second\n- [A](a.md) — first\n- [Gone](gone.md)\n").unwrap();
        for name in ["a.md", "b.md", "c.md"] {
            fs::write(dir.path().join(name), format!("# {name}\n")).unwrap();
        }
        fs::write(dir.path().join(".hidden.md"), "x").unwrap();
        let files: Vec<String> = read_indexed(dir.path()).unwrap().into_iter().map(|entry| entry.file).collect();
        assert_eq!(files, ["b.md", "a.md", "c.md"]);
        assert!(read_indexed(&dir.path().join("missing")).unwrap().is_empty());
    }

    #[test]
    fn forgetting_removes_the_file_and_its_line() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(INDEX), "# Memory\n- [A](a.md) — one\n- [B](b.md) — two\n").unwrap();
        fs::write(dir.path().join("a.md"), "a").unwrap();
        fs::write(dir.path().join("b.md"), "b").unwrap();
        forget_entry(dir.path(), "a.md").unwrap();
        assert!(!dir.path().join("a.md").exists());
        assert_eq!(fs::read_to_string(dir.path().join(INDEX)).unwrap(), "# Memory\n- [B](b.md) — two\n");
        write_entry(dir.path(), "b.md", "changed").unwrap();
        assert_eq!(fs::read_to_string(dir.path().join("b.md")).unwrap(), "changed");
    }

    #[test]
    fn only_a_stores_own_files_can_be_changed() {
        let dir = tempfile::tempdir().unwrap();
        for file in ["../x.md", "a/b.md", "MEMORY.md", ".a.md", "a.txt", "a\n.md"] {
            assert!(write_entry(dir.path(), file, "x").is_err(), "{file}");
            assert!(forget_entry(dir.path(), file).is_err(), "{file}");
        }
        assert_eq!(write_entry(dir.path(), "absent.md", "x").unwrap_err().kind(), io::ErrorKind::NotFound);
    }
}
