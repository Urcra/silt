//! Adapters for agent CLIs (ADR 0007, ADR 0012). The session files an
//! agent writes are the record; this crate finds them, reads them
//! incrementally, derives an agent-independent view of the conversation,
//! writes them back on another machine, and turns one agent's conversation
//! into another's.

pub mod acp;
pub mod claude;
pub mod codex;
pub mod conventions;
pub mod copilot;
mod entry;
pub mod gemini;
pub mod opencode;
mod render;
mod rewrite;
mod tail;
pub mod time;

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

pub use entry::Entry;
pub use render::{Role, render_transcript, translated_text};
pub use rewrite::{RecordedSession, Rewrites, is_safe_id};
pub use tail::{complete_lines, complete_lines_ending};

use conventions::NativeConventions;

/// A turn boundary read from a transcript, for agents whose hooks Silt
/// doesn't install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    PromptSubmitted,
    ToolStarted { tool_use_id: String, tool: String },
    ToolFinished { tool_use_id: String },
    Stopped,
}

/// What one transcript record adds to the conversation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Read {
    pub entries: Vec<Entry>,
    pub marks: Vec<Mark>,
}

/// Reads one transcript file's records in order. Some agents write a
/// message again when it grows, so a reader may remember what it has seen.
pub trait Reader: Send {
    fn push(&mut self, line: &str) -> Read;
}

/// A reader for agents whose records stand alone.
pub struct Stateless(pub fn(&str) -> Read);

impl Reader for Stateless {
    fn push(&mut self, line: &str) -> Read {
        (self.0)(line)
    }
}

/// How a new session gets its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionIds {
    /// Silt picks it and passes it to the agent.
    Chosen,
    /// The agent picks it; Silt finds the session once it is written.
    Discovered,
}

/// How a session is started or continued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    New { id: Option<String> },
    Resume { id: String },
    /// A new session whose first prompt carries the earlier conversation.
    Primed { id: Option<String>, prompt: String },
}

/// Silt's MCP server for the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServer {
    pub command: PathBuf,
    pub args: Vec<String>,
}

/// Everything Silt adds to an agent run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub start: Start,
    /// Where the agent runs.
    pub cwd: PathBuf,
    pub mcp: McpServer,
    /// Shell command that forwards hook events: `<command> <Event>`.
    pub hook_command: String,
    /// Rules and skills the agent doesn't read by itself.
    pub instructions: Option<String>,
    /// A directory Silt may use for per-run configuration files.
    pub scratch: PathBuf,
    /// Skill directories to hand the agent natively, where it can load them.
    pub skills: Vec<PathBuf>,
    pub args: Vec<String>,
}

/// The process to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub envs: Vec<(OsString, OsString)>,
}

impl Launch {
    /// `SILT_<AGENT>_BIN` overrides the program, as tests and wrappers do.
    pub fn new(command: &str) -> Self {
        let variable = format!("SILT_{}_BIN", command.to_ascii_uppercase());
        Self {
            program: std::env::var_os(variable).unwrap_or_else(|| command.into()),
            args: Vec::new(),
            envs: Vec::new(),
        }
    }

    pub fn arg(&mut self, arg: impl Into<OsString>) -> &mut Self {
        self.args.push(arg.into());
        self
    }

    pub fn env(&mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> &mut Self {
        self.envs.push((key.into(), value.into()));
        self
    }
}

/// A session found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub id: String,
    pub transcript: PathBuf,
}

/// One agent CLI.
pub trait Agent: Send + Sync {
    /// The name recorded with its sessions.
    fn id(&self) -> &'static str;
    /// The command people run.
    fn command(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn native(&self) -> NativeConventions;
    fn reader(&self) -> Box<dyn Reader>;
    /// Whether turn boundaries come from Silt's hooks rather than the
    /// transcript.
    fn hooks(&self) -> bool {
        false
    }
    fn session_ids(&self) -> SessionIds;
    /// Where a session's main transcript is or will be.
    fn transcript(&self, cwd: &Path, id: &str) -> Option<PathBuf>;
    /// Further transcripts next to the main one, such as subagents', keyed
    /// by name.
    fn side_transcripts(&self, _transcript: &Path) -> Vec<(String, PathBuf)> {
        Vec::new()
    }
    /// The directory of side files the transcript refers to by path.
    fn session_dir(&self, _transcript: &Path) -> Option<PathBuf> {
        None
    }
    /// Files to keep with the session, by name relative to the session dir.
    fn attachments(&self, _transcript: &Path) -> Vec<(String, PathBuf)> {
        Vec::new()
    }
    /// The newest session started in `cwd` no earlier than `since_ms` whose
    /// id isn't in `known`.
    fn discover(&self, cwd: &Path, since_ms: i64, known: &[String]) -> Option<Found>;
    /// Whether sessions live in a store rather than append-only files, and
    /// are read with [`Agent::snapshot`].
    fn reads_by_snapshot(&self) -> bool {
        false
    }
    /// The session's records as of now, for agents read by snapshot.
    fn snapshot(&self, _found: &Found) -> io::Result<Vec<String>> {
        Err(io::Error::other("this agent's sessions are files"))
    }
    /// Identifies a snapshot record, so a record is stored once.
    fn record_key(&self, _line: &str) -> Option<String> {
        None
    }
    /// A new id for a fork of session `from`.
    fn fork_id(&self, _from: &str) -> String {
        uuid::Uuid::new_v4().to_string()
    }
    /// Writes a recorded session so the agent continues it in `cwd`.
    /// Returns the main transcript to keep tailing.
    fn restore(&self, root: &Path, cwd: &Path, session: &RecordedSession) -> io::Result<PathBuf>;
    /// A native session holding `entries`, recorded by any agent, or `None`
    /// when this agent has no translation and needs a primed session.
    fn translate(&self, _entries: &[Entry], _cwd: &Path, _id: &str) -> Option<Vec<String>> {
        None
    }
    fn launch(&self, plan: &LaunchPlan) -> io::Result<Launch>;
}

/// The environment variables that say where agents keep their sessions.
pub const HOME_VARIABLES: [&str; 5] = ["HOME", "CLAUDE_CONFIG_DIR", "CODEX_HOME", "GEMINI_CLI_HOME", "COPILOT_HOME"];

pub const AGENTS: [&dyn Agent; 5] = [
    &claude::ClaudeCode,
    &codex::Codex,
    &gemini::Gemini,
    &opencode::OpenCode,
    &copilot::Copilot,
];

/// The agent recorded as `name`, or run as the command `name`, including
/// the sessions Silt records itself for agents the web UI starts.
pub fn agent(name: &str) -> Option<&'static dyn Agent> {
    AGENTS
        .into_iter()
        .find(|agent| agent.id() == name || agent.command() == name)
        .or_else(|| (name == acp::AGENT).then_some(&acp::Acp as &dyn Agent))
}

/// The reader for an agent's transcripts; unknown agents' records are kept
/// but read as nothing.
pub fn reader(agent_id: &str) -> Box<dyn Reader> {
    match agent(agent_id) {
        Some(agent) => agent.reader(),
        None => Box::new(Stateless(|_| Read::default())),
    }
}

/// Readers for many sessions' transcripts, for reading a log in order.
/// Only the most recently read ones are kept: a long-running reader of every
/// session, like the server's index, would otherwise keep one per session
/// ever seen. A session read again after its reader went starts afresh.
#[derive(Default)]
pub struct Transcripts {
    readers: std::collections::HashMap<TranscriptKey, KeptReader>,
    reads: u64,
}

/// Agent, session and file.
type TranscriptKey = (String, String, String);

struct KeptReader {
    reader: Box<dyn Reader>,
    last_read: u64,
}

/// Readers kept before the least recently used quarter is dropped.
const READERS_KEPT: usize = 1024;

impl std::fmt::Debug for Transcripts {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Transcripts").field("readers", &self.readers.len()).finish()
    }
}

impl Transcripts {
    pub fn push(&mut self, agent: &str, session: &str, file: &str, line: &str) -> Read {
        self.reads += 1;
        let key = (agent.to_owned(), session.to_owned(), file.to_owned());
        if self.readers.len() >= READERS_KEPT && !self.readers.contains_key(&key) {
            self.forget_least_recent();
        }
        let kept = self.readers.entry(key).or_insert_with(|| KeptReader {
            reader: reader(agent),
            last_read: 0,
        });
        kept.last_read = self.reads;
        kept.reader.push(line)
    }

    fn forget_least_recent(&mut self) {
        let mut reads: Vec<u64> = self.readers.values().map(|kept| kept.last_read).collect();
        let (_, cutoff, _) = reads.select_nth_unstable(READERS_KEPT / 4);
        let cutoff = *cutoff;
        self.readers.retain(|_, kept| kept.last_read > cutoff);
    }

    pub fn len(&self) -> usize {
        self.readers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.readers.is_empty()
    }

    pub fn entries(&mut self, agent: &str, session: &str, file: &str, line: &str) -> Vec<Entry> {
        self.push(agent, session, file, line).entries
    }
}

pub(crate) fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
}

/// Writes a restored transcript in place of whatever the agent has there.
/// The new file is renamed into place whole, so a crash leaves the old one.
/// An old one that isn't the start of the new one has lines the recording
/// lacks, written while Silt wasn't looking; it is kept beside the new one.
pub(crate) fn replace_transcript(path: &Path, content: &str) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| std::io::Error::other("a transcript has a directory"))?;
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    match std::fs::read(path) {
        Ok(existing) if !content.as_bytes().starts_with(&existing) => {
            let backup = dir.join(format!("{name}.before-silt-{}", now_ms()));
            std::fs::copy(path, &backup)?;
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let staged = dir.join(format!(".{name}.{}.silt-tmp", std::process::id()));
    let mut file = std::fs::File::create(&staged)?;
    std::io::Write::write_all(&mut file, content.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&staged, path)
}

/// Milliseconds since the epoch of a file's last change.
/// Links each skill directory into `dir` under its directory name, made
/// safe as a file name. Of skills sharing a name the first is kept, as Silt
/// lists the repository's before the personal ones.
pub(crate) fn link_skills(dir: &Path, skills: &[PathBuf]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for skill in skills {
        let name: String = skill
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_default()
            .chars()
            .map(|char| if char.is_ascii_alphanumeric() || matches!(char, '-' | '_' | '.') { char } else { '-' })
            .collect();
        let name = name.trim_start_matches('.');
        if name.is_empty() {
            continue;
        }
        let link = dir.join(name);
        // A link already there, even a dangling one, is a skill of that name.
        if std::fs::symlink_metadata(&link).is_err() {
            symlink(skill, &link)?;
        }
    }
    Ok(())
}

/// Makes `link` point at `target`, a directory or a file.
pub(crate) fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    return std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    return if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    };
    #[cfg(not(any(unix, windows)))]
    return Err(std::io::Error::new(std::io::ErrorKind::Unsupported, format!("can't link {} here", link.display())));
}

/// A transcript's first line, which agents start with a record of where
/// the session runs, read without reading the rest.
pub(crate) fn first_line(path: &Path) -> Option<String> {
    const LIMIT: u64 = 4 << 20;
    let file = std::fs::File::open(path).ok()?;
    let mut line = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(std::io::Read::take(file, LIMIT)), &mut line).ok()?;
    Some(line)
}

pub(crate) fn modified_ms(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let elapsed = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(elapsed.as_millis()).ok()
}

/// A stable UUID-shaped id for the `index`th translated line of a session.
pub(crate) fn derived_uuid(session: &str, index: usize) -> String {
    let mut hash: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    for byte in session.bytes().chain(index.to_le_bytes()) {
        hash ^= u128::from(byte);
        hash = hash.wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013B);
    }
    let hex = format!("{hash:032x}");
    format!("{}-{}-4{}-8{}-{}", &hex[..8], &hex[8..12], &hex[13..16], &hex[17..20], &hex[20..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backups(dir: &Path) -> usize {
        std::fs::read_dir(dir)
            .expect("lists")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().contains(".before-silt-"))
            .count()
    }

    #[cfg(unix)]
    #[test]
    fn sessions_translate_into_a_directory_whose_name_is_not_utf8() {
        use std::os::unix::ffi::OsStrExt as _;
        let cwd = Path::new(std::ffi::OsStr::from_bytes(b"/w/caf\xe9"));
        let messages = [(Role::User, "hi".to_owned())];
        assert!(claude::transcript_lines(&messages, cwd, "s")[0].contains("/w/caf\u{fffd}"));
        assert!(codex::rollout_lines(&messages, cwd, "s")[0].contains("/w/caf\u{fffd}"));
        assert!(copilot::event_lines(&messages, cwd, "s")[0].contains("/w/caf\u{fffd}"));
    }

    #[test]
    fn skills_sharing_a_name_link_once() {
        let root = tempfile::tempdir().expect("temp dir");
        let skill = |path: &str| {
            let dir = root.path().join(path);
            std::fs::create_dir_all(&dir).expect("mkdir");
            dir
        };
        let skills = [skill("repo/review"), skill("home/review"), skill("home/odd name"), root.path().join("gone/stale")];
        let linked = root.path().join("plugin/skills");
        link_skills(&linked, &skills).expect("links");
        link_skills(&linked, &skills).expect("links again over dangling links");
        assert_eq!(std::fs::read_link(linked.join("review")).expect("link"), skills[0]);
        assert!(linked.join("odd-name").is_dir());
        assert!(std::fs::symlink_metadata(linked.join("stale")).is_ok());
    }

    #[test]
    fn only_recently_read_transcripts_keep_a_reader() {
        let mut transcripts = Transcripts::default();
        for session in 0..READERS_KEPT * 3 {
            transcripts.push("claude-code", "kept", "main", "{}");
            transcripts.push("claude-code", &session.to_string(), "main", "{}");
        }
        assert!(transcripts.len() <= READERS_KEPT, "{}", transcripts.len());
        assert!(transcripts.readers.contains_key(&("claude-code".to_owned(), "kept".to_owned(), "main".to_owned())));
    }

    #[test]
    fn a_transcript_with_lines_the_recording_lacks_is_kept() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("session.jsonl");
        replace_transcript(&path, "one\n").expect("writes");
        replace_transcript(&path, "one\ntwo\n").expect("extends");
        assert_eq!(backups(dir.path()), 0, "the recording only adds to what is there");
        std::fs::write(&path, "one\ntwo\nwritten outside silt\n").expect("the agent goes on");
        replace_transcript(&path, "one\ntwo\nthree\n").expect("restores");
        assert_eq!(std::fs::read_to_string(&path).expect("reads"), "one\ntwo\nthree\n");
        assert_eq!(backups(dir.path()), 1, "the lines Silt never saw are kept");
    }
}
