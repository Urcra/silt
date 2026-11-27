//! What agents write down to read in later sessions (ADR 0137), read from
//! each agent's own store into one shape for the memory viewer.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::conventions::{parse_frontmatter, split_frontmatter};

/// The file indexing a folder of memories, a line a memory.
pub const INDEX: &str = "MEMORY.md";

/// Settings › Memory's switches, in `memory.json` (ADR 0137).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    /// Agents share memory; on unless turned off.
    #[serde(default = "on")]
    pub share: bool,
    /// The projects, by their clones, whose memory is also sent to Copilot
    /// Memory on GitHub.
    #[serde(default)]
    pub copilot: std::collections::BTreeSet<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { share: true, copilot: Default::default() }
    }
}

fn on() -> bool {
    true
}

pub fn settings(data_dir: &Path) -> Settings {
    fs::read_to_string(data_dir.join(SETTINGS)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

pub fn save_settings(data_dir: &Path, settings: &Settings) -> io::Result<()> {
    fs::create_dir_all(data_dir)?;
    let json = serde_json::to_vec_pretty(settings).map_err(io::Error::other)?;
    write_atomically(&data_dir.join(SETTINGS), &json)
}

/// Whether agents share memory.
pub fn sharing(data_dir: &Path) -> bool {
    settings(data_dir).share
}

pub fn set_sharing(data_dir: &Path, share: bool) -> io::Result<()> {
    save_settings(data_dir, &Settings { share, ..settings(data_dir) })
}

const SETTINGS: &str = "memory.json";

/// Silt's own store of what holds in every project, in its data folder.
pub fn everywhere_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("memory").join("everywhere")
}

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

/// What [`merge_into`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Merged {
    /// Memories copied over, by their name in the target.
    pub copied: Vec<String>,
}

/// The file in a merged-from folder saying it was, so it's merged once.
pub const MERGED_MARK: &str = ".silt-merged";

/// Copies the memories of the folder `from` into `into`, with their index
/// lines, once: the folder is marked as merged, and left otherwise as it
/// was. A memory the target holds the same is skipped; one it holds
/// differently under the same name is kept beside it, named after `tag`.
pub fn merge_into(from: &Path, into: &Path, tag: &str) -> io::Result<Merged> {
    let mut merged = Merged::default();
    if from.join(MERGED_MARK).exists() || !from.is_dir() {
        return Ok(merged);
    }
    let entries = read_indexed(from)?;
    if !entries.is_empty() {
        fs::create_dir_all(into)?;
    }
    let mut index = fs::read_to_string(into.join(INDEX)).unwrap_or_default();
    for entry in &entries {
        if let Added::Copied(name) = add_to(into, entry, &mut index, tag)? {
            merged.copied.push(name);
        }
    }
    if !merged.copied.is_empty() {
        write_atomically(&into.join(INDEX), index.as_bytes())?;
    }
    fs::write(from.join(MERGED_MARK), format!("merged into {}\n", into.display()))?;
    Ok(merged)
}

/// What [`add_to`] did with a memory.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Added {
    /// Copied under this name.
    Copied(String),
    /// The target held it the same already.
    Held(String),
    /// No free name for it.
    Skipped,
}

/// Copies `entry` into the folder `into`, adding its line to `index`: under
/// its own name, or `<name>-<tag>.md` when the target holds that name
/// differently.
fn add_to(into: &Path, entry: &Entry, index: &mut String, tag: &str) -> io::Result<Added> {
    let name = match fs::read_to_string(into.join(&entry.file)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => entry.file.clone(),
        Err(error) => return Err(error),
        Ok(held) if held == entry.text => return Ok(Added::Held(entry.file.clone())),
        Ok(_) => {
            let stem = entry.file.strip_suffix(".md").unwrap_or(&entry.file);
            let tag: String = tag.chars().filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-').collect();
            format!("{stem}-{tag}.md")
        }
    };
    if !is_entry_file(&name) || into.join(&name).exists() {
        return Ok(Added::Skipped);
    }
    fs::create_dir_all(into)?;
    write_atomically(&into.join(&name), entry.text.as_bytes())?;
    if !index.is_empty() && !index.ends_with('\n') {
        index.push('\n');
    }
    let hook = if entry.description.is_empty() { String::new() } else { format!(" — {}", entry.description) };
    index.push_str(&format!("- [{}]({name}){hook}\n", entry.name));
    Ok(Added::Copied(name))
}

/// Moves the memory `file` from the folder `from` to `into`, with its index
/// line, and gives its name there.
pub fn move_entry(from: &Path, file: &str, into: &Path) -> io::Result<String> {
    let entry = read_indexed(from)?
        .into_iter()
        .find(|entry| entry.file == file)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no memory {file}")))?;
    let mut index = fs::read_to_string(into.join(INDEX)).unwrap_or_default();
    let name = match add_to(into, &entry, &mut index, "moved")? {
        Added::Copied(name) => {
            write_atomically(&into.join(INDEX), index.as_bytes())?;
            name
        }
        Added::Held(name) => name,
        Added::Skipped => return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{file} is there already"))),
    };
    forget_entry(from, file)?;
    Ok(name)
}

/// Moves the memories of `kind` from the folder `from` to `into`, such as
/// what an agent filed under a project about the person.
pub fn move_kind(from: &Path, kind: &str, into: &Path) -> io::Result<Vec<String>> {
    let mut moved = Vec::new();
    for entry in read_indexed(from)?.into_iter().filter(|entry| entry.kind.as_deref() == Some(kind)) {
        moved.push(move_entry(from, &entry.file, into)?);
    }
    Ok(moved)
}

/// What an agent is told of the memories in `dir`, which hold in every
/// project: where they are and their index. `None` when there are none.
pub fn everywhere_prompt(dir: &Path) -> Option<String> {
    let entries = read_indexed(dir).ok()?;
    if entries.is_empty() {
        return None;
    }
    let mut prompt = format!(
        "# Memory everywhere\n\nWhat you and the person's other agents learned about them, which holds in every project. \
Silt keeps it in `{}`: read a memory's file there when it's relevant. \
To remember something new about the person, save it to your own memory as you would, of type `user`; Silt moves it here.\n\n",
        dir.display()
    );
    for entry in entries {
        let hook = if entry.description.is_empty() { String::new() } else { format!(" — {}", entry.description) };
        prompt.push_str(&format!("- [{}]({}){hook}\n", entry.name, entry.file));
    }
    Some(prompt)
}

/// The kinds a memory can be, as Claude Code files them.
pub const KINDS: [&str; 4] = ["user", "feedback", "project", "reference"];

/// Writes a memory in Claude Code's format to the folder `dir`, with its
/// index line: as `file` when given, replacing it, else a new file named
/// from its kind and name. Gives the file's name.
pub fn write_new(dir: &Path, file: Option<&str>, name: &str, description: &str, kind: &str, body: &str) -> io::Result<String> {
    let invalid = |message: String| io::Error::new(io::ErrorKind::InvalidInput, message);
    if !KINDS.contains(&kind) {
        return Err(invalid(format!("a memory is of kind {}, not {kind}", KINDS.join(", "))));
    }
    let one_line = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    let (name, description) = (one_line(name), one_line(description));
    if name.is_empty() {
        return Err(invalid("a memory needs a name".into()));
    }
    let file = match file {
        Some(file) => file.to_owned(),
        None => {
            let slug: String = name
                .to_lowercase()
                .chars()
                .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
                .collect::<String>()
                .split('-')
                .filter(|part| !part.is_empty())
                .take(8)
                .collect::<Vec<_>>()
                .join("-");
            let stem = format!("{kind}-{slug}");
            (0..)
                .map(|n| if n == 0 { format!("{stem}.md") } else { format!("{stem}-{n}.md") })
                .find(|file| !dir.join(file).exists())
                .expect("a name is free")
        }
    };
    if !is_entry_file(&file) {
        return Err(invalid(format!("{file:?} isn't a memory's file")));
    }
    let quoted = |text: &str| serde_json::Value::String(text.to_owned()).to_string();
    let text = format!(
        "---\nname: {}\ndescription: {}\nmetadata:\n  type: {kind}\n  modified: {}\n---\n\n{}\n",
        quoted(&name),
        quoted(&description),
        crate::time::timestamp_now(),
        body.trim()
    );
    fs::create_dir_all(dir)?;
    write_atomically(&dir.join(&file), text.as_bytes())?;
    let index = fs::read_to_string(dir.join(INDEX)).unwrap_or_default();
    let mut index = without_line_for(&index, &file);
    if !index.is_empty() && !index.ends_with('\n') {
        index.push('\n');
    }
    let hook = if description.is_empty() { String::new() } else { format!(" — {description}") };
    index.push_str(&format!("- [{name}]({file}){hook}\n"));
    write_atomically(&dir.join(INDEX), index.as_bytes())?;
    Ok(file)
}

/// What an agent whose own memory isn't the shared one is told (ADR
/// 0137): the indexes of the project's memory and the memory everywhere,
/// and the tools to read and add to them. `None` when both are empty.
pub fn shared_prompt(project: Option<&Path>, everywhere: &Path) -> Option<String> {
    let list = |dir: &Path| {
        read_indexed(dir)
            .unwrap_or_default()
            .into_iter()
            .map(|entry| {
                let hook = if entry.description.is_empty() { String::new() } else { format!(" — {}", entry.description) };
                format!("- [{}]({}){hook}\n", entry.name, entry.file)
            })
            .collect::<String>()
    };
    let (in_project, in_everywhere) = (project.map(list).unwrap_or_default(), list(everywhere));
    let mut prompt = String::from(
        "# Memory\n\nYou share memory with the person's other agents through Silt: what earlier sessions learnt about this \
project, and about the person in every project. Read a memory with silt_memory_read when it's relevant, find one with \
silt_memory_search, and save what a later session should know with silt_memory_write. Memories can be out of date: \
check what a memory names still exists before relying on it.\n",
    );
    if in_project.is_empty() && in_everywhere.is_empty() {
        prompt.push_str("\nNothing is remembered yet.\n");
        return Some(prompt);
    }
    if !in_project.is_empty() {
        prompt.push_str(&format!("\n## This project\n\n{in_project}"));
    }
    if !in_everywhere.is_empty() {
        prompt.push_str(&format!("\n## Everywhere\n\n{in_everywhere}"));
    }
    Some(prompt)
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
    fn a_threads_memory_is_merged_once_keeping_both_of_a_name() {
        let root = tempfile::tempdir().unwrap();
        let (from, into) = (root.path().join("thread"), root.path().join("clone"));
        fs::create_dir_all(&from).unwrap();
        fs::create_dir_all(&into).unwrap();
        fs::write(from.join(INDEX), "- [Same](same.md) — kept\n- [New](new.md) — learnt\n- [Clash](clash.md) — changed\n")
            .unwrap();
        for (file, text) in [("same.md", "same"), ("new.md", "new"), ("clash.md", "theirs")] {
            fs::write(from.join(file), text).unwrap();
        }
        fs::write(into.join(INDEX), "- [Same](same.md) — kept\n- [Clash](clash.md) — mine").unwrap();
        fs::write(into.join("same.md"), "same").unwrap();
        fs::write(into.join("clash.md"), "mine").unwrap();

        let merged = merge_into(&from, &into, "t1").unwrap();
        assert_eq!(merged.copied, ["new.md", "clash-t1.md"]);
        assert_eq!(fs::read_to_string(into.join("clash.md")).unwrap(), "mine");
        assert_eq!(fs::read_to_string(into.join("clash-t1.md")).unwrap(), "theirs");
        assert_eq!(
            fs::read_to_string(into.join(INDEX)).unwrap(),
            "- [Same](same.md) — kept\n- [Clash](clash.md) — mine\n- [New](new.md) — learnt\n- [Clash](clash-t1.md) — changed\n"
        );
        assert!(from.join("new.md").exists(), "the thread's folder is left as it was");
        assert_eq!(merge_into(&from, &into, "t1").unwrap(), Merged::default(), "once");
        assert_eq!(merge_into(&root.path().join("missing"), &into, "t2").unwrap(), Merged::default());
    }

    #[test]
    fn memories_about_the_person_move_with_their_lines() {
        let root = tempfile::tempdir().unwrap();
        let (project, everywhere) = (root.path().join("project"), root.path().join("everywhere"));
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join(INDEX), "- [Rust](user-rust.md) — senior\n- [Gate](project-gate.md) — CI\n").unwrap();
        fs::write(
            project.join("user-rust.md"),
            "---\nname: Rust\ndescription: senior\nmetadata:\n  type: user\n---\nCodes in Rust.\n",
        )
        .unwrap();
        fs::write(project.join("project-gate.md"), "---\nname: Gate\nmetadata:\n  type: project\n---\nCI.\n").unwrap();

        assert_eq!(move_kind(&project, "user", &everywhere).unwrap(), ["user-rust.md"]);
        assert!(!project.join("user-rust.md").exists());
        assert_eq!(fs::read_to_string(project.join(INDEX)).unwrap(), "- [Gate](project-gate.md) — CI\n");
        assert_eq!(fs::read_to_string(everywhere.join(INDEX)).unwrap(), "- [Rust](user-rust.md) — senior\n");
        assert!(move_kind(&project, "user", &everywhere).unwrap().is_empty());

        let prompt = everywhere_prompt(&everywhere).unwrap();
        assert!(prompt.contains(&everywhere.display().to_string()));
        assert!(prompt.contains("- [Rust](user-rust.md) — senior"));
        assert_eq!(everywhere_prompt(&root.path().join("none")), None);

        assert_eq!(move_entry(&everywhere, "user-rust.md", &project).unwrap(), "user-rust.md");
        assert!(project.join("user-rust.md").exists() && !everywhere.join("user-rust.md").exists());
    }

    #[test]
    fn a_new_memory_is_written_as_claude_code_writes_one() {
        let dir = tempfile::tempdir().unwrap();
        let file =
            write_new(dir.path(), None, "Uses pnpm, never npm", "in every project", "feedback", "Install with pnpm.").unwrap();
        assert_eq!(file, "feedback-uses-pnpm-never-npm.md");
        let entry = &read_indexed(dir.path()).unwrap()[0];
        assert_eq!((entry.name.as_str(), entry.description.as_str()), ("Uses pnpm, never npm", "in every project"));
        assert_eq!((entry.kind.as_deref(), entry.body.trim()), (Some("feedback"), "Install with pnpm."));
        assert_eq!(
            write_new(dir.path(), None, "Uses pnpm, never npm", "", "feedback", "Again.").unwrap(),
            "feedback-uses-pnpm-never-npm-1.md"
        );
        write_new(dir.path(), Some(&file), "Uses pnpm", "changed", "feedback", "Still pnpm.").unwrap();
        let index = fs::read_to_string(dir.path().join(INDEX)).unwrap();
        assert_eq!(index.matches(&file).count(), 1, "{index}");
        assert!(index.contains("- [Uses pnpm](feedback-uses-pnpm-never-npm.md) — changed"));
        assert!(write_new(dir.path(), None, "x", "", "secret", "x").is_err());
        assert!(write_new(dir.path(), Some("../x.md"), "x", "", "user", "x").is_err());
    }

    #[test]
    fn sharing_is_on_until_turned_off() {
        let dir = tempfile::tempdir().unwrap();
        assert!(sharing(dir.path()));
        set_sharing(dir.path(), false).unwrap();
        assert!(!sharing(dir.path()));
        let (project, everywhere) = (dir.path().join("p"), dir.path().join("e"));
        assert!(shared_prompt(Some(&project), &everywhere).unwrap().contains("Nothing is remembered yet"));
        write_new(&project, None, "Gate", "CI runs it", "project", "CI.").unwrap();
        let prompt = shared_prompt(Some(&project), &everywhere).unwrap();
        assert!(prompt.contains("## This project") && prompt.contains("- [Gate](project-gate.md) — CI runs it"));
        assert!(!prompt.contains("## Everywhere"));
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
