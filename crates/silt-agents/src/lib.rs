//! Adapters for agent CLIs (ADR 0007, ADR 0012). The session files an
//! agent writes are the record; this crate finds them, reads them
//! incrementally, derives an agent-independent view of the conversation,
//! writes them back on another machine, and turns one agent's conversation
//! into another's.

pub mod acp;
pub mod antigravity;
pub mod claude;
pub mod cloud;
pub mod codex;
pub mod conventions;
pub mod copilot;
mod entry;
mod error;
pub mod gemini;
pub mod history;
pub mod opencode;
mod render;
mod rewrite;
pub mod sign_in;
mod tail;
pub mod time;
pub mod tool;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub use entry::{Entry, Task, TaskKind, TaskStatus};
pub use error::AgentError;
pub use render::{Role, render_transcript, translated_text};
pub use rewrite::{CheckedSession, RecordedSession, Rewrites, is_safe_id, is_side_file_name};
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
    /// The subagent the record is from, for agents that write a subagent's
    /// records into the session's own file; named as its `Task` news names
    /// its `side`.
    pub side: Option<String>,
}

/// A file that restoring a session writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Write {
    /// A transcript, which replaces the agent's own and keeps a copy of it
    /// when it has lines the recording lacks.
    Transcript {
        path: PathBuf,
        lines: String,
    },
    File {
        path: PathBuf,
        bytes: Vec<u8>,
    },
}

/// What restoring a session writes, decided before anything is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restore {
    /// The transcript the agent resumes from.
    pub transcript: PathBuf,
    pub writes: Vec<Write>,
}

impl Restore {
    pub fn apply(&self) -> Result<(), AgentError> {
        for write in &self.writes {
            let (Write::Transcript { path, .. } | Write::File { path, .. }) = write;
            std::fs::create_dir_all(path.parent().ok_or_else(|| AgentError::NoDirectory(path.clone()))?)?;
            match write {
                Write::Transcript { path, lines } => replace_transcript(path, lines)?,
                Write::File { path, bytes } => std::fs::write(path, bytes)?,
            }
        }
        Ok(())
    }

    /// Side files of the session, kept under `dir`.
    pub(crate) fn side_files(&mut self, dir: &Path, session: &CheckedSession) {
        for (name, bytes) in &session.attachments {
            self.writes.push(Write::File { path: dir.join(name), bytes: bytes.clone() });
        }
    }
}

impl Read {
    pub fn unparseable() -> Self {
        Self { entries: vec![Entry::unparseable()], ..Self::default() }
    }
}

/// Reads one transcript file's records in order. Some agents write a
/// message again when it grows, so a reader may remember what it has seen.
pub trait Reader: Send {
    fn push(&mut self, line: &str) -> Read;

    /// Whether what it read before changes how it reads a line, so a
    /// reader for a session read before must be shown its earlier lines.
    fn remembers(&self) -> bool {
        true
    }
}

/// A reader for agents whose records stand alone.
pub struct Stateless(pub fn(&str) -> Read);

impl Reader for Stateless {
    fn push(&mut self, line: &str) -> Read {
        (self.0)(line)
    }

    fn remembers(&self) -> bool {
        false
    }
}

/// How Silt learns what an agent does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capture {
    /// An append-only transcript, which also marks turns and tools.
    Transcript,
    /// A transcript, with turns and tools reported through Silt's hooks.
    Hooks,
    /// A store, read whole with [`Agent::snapshot`].
    Snapshot,
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
    New {
        id: Option<String>,
    },
    Resume {
        id: String,
    },
    /// A new session whose first prompt carries the earlier conversation.
    Primed {
        id: Option<String>,
        prompt: String,
    },
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
    /// The `silt` that command runs, for agents that can start a program
    /// without a shell, which Windows needs: its agents pick Git Bash or
    /// PowerShell, and the two read a quoted path differently.
    pub hook_program: PathBuf,
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
        Self { program: std::env::var_os(variable).unwrap_or_else(|| command.into()), args: Vec::new(), envs: Vec::new() }
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

/// A session the agent has on disk, which may have ended long ago.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnDisk {
    pub id: String,
    pub transcript: PathBuf,
    /// Where it ran, when its files say.
    pub cwd: Option<PathBuf>,
    pub started_ms: Option<i64>,
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
    fn capture(&self) -> Capture {
        Capture::Transcript
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
    /// Every session on disk, for importing (ADR 0043). An agent that can
    /// only be asked per directory is asked in each of `dirs`.
    fn on_disk(&self, _dirs: &[PathBuf]) -> Vec<OnDisk> {
        Vec::new()
    }
    /// The session's records as of now, for agents read by snapshot.
    /// `cwd` is the folder the session ran in, as an agent that keeps its
    /// sessions per project finds them only from there.
    fn snapshot(&self, _found: &Found, _cwd: &Path) -> Result<Vec<String>, AgentError> {
        Err(AgentError::NoSnapshot)
    }
    /// Identifies a snapshot record, so a record is stored once.
    fn record_key(&self, _line: &str) -> Option<String> {
        None
    }
    /// A new id for a fork of session `from`.
    fn fork_id(&self, _from: &str) -> String {
        uuid::Uuid::new_v4().to_string()
    }
    /// Whether [`Agent::restore`] can bring `session` back, or the agent
    /// must start primed with the conversation instead, as an agent that
    /// resumes only from a store of its own which Silt can't write.
    fn can_restore(&self, _session: &CheckedSession) -> bool {
        true
    }
    /// Writes a recorded session so the agent continues it in `cwd`.
    /// Returns the main transcript to keep tailing.
    fn restore(&self, root: &Path, cwd: &Path, session: &CheckedSession) -> Result<PathBuf, AgentError>;
    /// A native session holding `entries`, recorded by any agent, or `None`
    /// when this agent has no translation and needs a primed session.
    fn translate(&self, _entries: &[Entry], _cwd: &Path, _id: &str) -> Option<Vec<String>> {
        None
    }
    fn launch(&self, plan: &LaunchPlan) -> Result<Launch, AgentError>;
}

/// The environment variables that say where agents keep their sessions.
pub const HOME_VARIABLES: [&str; 5] = ["HOME", "CLAUDE_CONFIG_DIR", "CODEX_HOME", "GEMINI_CLI_HOME", "COPILOT_HOME"];

pub const AGENTS: [&dyn Agent; 6] =
    [&claude::ClaudeCode, &codex::Codex, &gemini::Gemini, &antigravity::Antigravity, &opencode::OpenCode, &copilot::Copilot];

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
        let kept = self.readers.entry(key).or_insert_with(|| KeptReader { reader: reader(agent), last_read: 0 });
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

/// An agent's home: `$variable` when set, else `dir` in the user's home.
pub(crate) fn agent_home(variable: &str, dir: &str) -> Option<PathBuf> {
    std::env::var_os(variable).map(PathBuf::from).or_else(|| home().map(|home| home.join(dir)))
}

pub(crate) fn home() -> Option<PathBuf> {
    silt_os::home_dir()
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
pub(crate) fn replace_transcript(path: &Path, content: &str) -> Result<(), AgentError> {
    let dir = path.parent().ok_or_else(|| AgentError::NoDirectory(path.to_owned()))?;
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    match std::fs::read(path) {
        Ok(existing) if !content.as_bytes().starts_with(&existing) => {
            let backup = dir.join(format!("{name}.before-silt-{}", now_ms()));
            std::fs::copy(path, &backup)?;
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let staged = dir.join(format!(".{name}.{}.silt-tmp", std::process::id()));
    let mut file = std::fs::File::create(&staged)?;
    std::io::Write::write_all(&mut file, content.as_bytes())?;
    file.sync_all()?;
    Ok(std::fs::rename(&staged, path)?)
}

/// Milliseconds since the epoch of a file's last change.
/// `text` as one word for a POSIX shell.
pub fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// `text` as one argument for PowerShell's call operator, `& '<path>'`.
pub fn powershell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

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
    silt_os::link(target, link)
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

/// A transcript's first `count` lines, read without reading the rest.
pub(crate) fn first_lines(path: &Path, count: usize) -> Vec<String> {
    const LIMIT: u64 = 4 << 20;
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    std::io::BufRead::lines(std::io::BufReader::new(std::io::Read::take(file, LIMIT))).take(count).map_while(Result::ok).collect()
}

/// When a transcript record says it was written: its `timestamp`, as RFC
/// 3339 or milliseconds since the epoch.
pub fn record_time(line: &str) -> Option<i64> {
    #[derive(serde::Deserialize)]
    struct Stamped {
        timestamp: serde_json::Value,
    }
    let stamped: Stamped = serde_json::from_str(line).ok()?;
    match stamped.timestamp {
        serde_json::Value::String(text) => time::parse_timestamp(&text),
        serde_json::Value::Number(number) => number.as_i64(),
        _ => None,
    }
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
