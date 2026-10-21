//! Fixtures as big as real use (ADR 0034), generated the same way every
//! time and cached, since writing them takes longer than some budgets.

use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

/// Bump when a generator changes, so no stale fixture is measured.
const FIXTURE_VERSION: u32 = 1;

pub const TRANSCRIPT_BYTES: u64 = 100_000_000;
pub const REPO_FILES: usize = 50_000;
pub const SESSION_ID: &str = "5e1f0000-0000-4000-8000-000000000001";

pub struct Fixtures {
    dir: PathBuf,
}

impl Fixtures {
    pub fn in_dir(root: &Path) -> Self {
        Self { dir: root.join(format!("v{FIXTURE_VERSION}")) }
    }

    pub fn transcript(&self) -> io::Result<PathBuf> {
        let path = self.dir.join("transcript.jsonl");
        cached(&path, |tmp| write_transcript(tmp, TRANSCRIPT_BYTES))?;
        Ok(path)
    }

    /// A git repository of `REPO_FILES` small files in one commit.
    pub fn big_repo(&self) -> io::Result<PathBuf> {
        let path = self.dir.join("repo-50k");
        cached(&path, |tmp| write_repo(tmp, REPO_FILES))?;
        Ok(path)
    }
}

fn cached(path: &Path, make: impl FnOnce(&Path) -> io::Result<()>) -> io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    let parent = path.parent().expect("a fixture lives in a directory");
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(".{}.partial", path.file_name().expect("named").to_string_lossy()));
    if tmp.is_dir() {
        fs::remove_dir_all(&tmp)?;
    } else if tmp.exists() {
        fs::remove_file(&tmp)?;
    }
    eprintln!("silt-bench: writing fixture {}", path.display());
    make(&tmp)?;
    fs::rename(&tmp, path)
}

/// A clone of `repo` at `to`, for a run to change as it likes.
pub fn clone_repo(repo: &Path, to: &Path) -> io::Result<()> {
    git(Path::new("/"), &["clone", "-q", "--local", &repo.to_string_lossy(), &to.to_string_lossy()])
}

/// A repository of `files` small files, a hundred to a directory.
pub fn write_repo(repo: &Path, files: usize) -> io::Result<()> {
    for index in 0..files {
        let dir = repo.join(format!("src/d{}", index / 100));
        fs::create_dir_all(&dir)?;
        fs::write(dir.join(format!("f{index}.rs")), format!("pub fn f{index}() {{}}\n"))?;
    }
    git(repo, &["init", "-q", "-b", "main"])?;
    git(repo, &["add", "."])?;
    git(repo, &["-c", "user.name=bench", "-c", "user.email=bench@example.com", "commit", "-q", "-m", "files"])
}

fn git(dir: &Path, args: &[&str]) -> io::Result<()> {
    let status = Command::new("git").args(args).current_dir(dir).status()?;
    if status.success() { Ok(()) } else { Err(io::Error::other(format!("git {} failed", args.join(" ")))) }
}

/// Writes Claude Code transcript lines until `bytes` are written.
pub fn write_transcript(path: &Path, bytes: u64) -> io::Result<()> {
    let mut out = BufWriter::new(fs::File::create(path)?);
    let mut lines = Transcript::new("/bench/repo");
    let mut written = 0;
    while written < bytes {
        for line in lines.prompt() {
            out.write_all(line.as_bytes())?;
            out.write_all(b"\n")?;
            written += line.len() as u64 + 1;
        }
    }
    out.flush()
}

/// The lines of `prompts` turns of a small session, for fixtures where the
/// transcript isn't what's measured.
pub fn small_transcript(path: &Path, cwd: &Path, prompts: usize) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut lines = Transcript::new(&cwd.to_string_lossy());
    let text: String = (0..prompts).flat_map(|_| lines.prompt()).map(|line| line + "\n").collect();
    fs::write(path, text)
}

/// Generates lines shaped like a long real Claude Code session: per prompt,
/// a few tool calls, each with thinking, its result and hook attachments,
/// plus the small bookkeeping lines Claude writes. Tool results carry most
/// of the bytes, as they do in real sessions.
struct Transcript {
    cwd: String,
    words: Words,
    index: usize,
    parent: Option<String>,
}

impl Transcript {
    fn new(cwd: &str) -> Self {
        Self { cwd: cwd.to_owned(), words: Words(0x2545_F491_4F6C_DD1D), index: 0, parent: None }
    }

    fn prompt(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        let prompt = self.words.sentence(60);
        lines.push(self.entry("user", json!({ "message": { "role": "user", "content": prompt } })));
        for kind in ["last-prompt", "mode", "permission-mode", "ai-title", "agent-name", "bridge-session"] {
            let value = self.words.sentence(8);
            lines.push(self.entry(kind, json!({ "value": value })));
        }
        let tools = 3 + self.index % 2;
        for tool in 0..tools {
            if tool % 3 == 0 {
                let thinking = json!([{ "type": "thinking", "thinking": "", "signature": self.words.token(1600) }]);
                lines.push(self.assistant(thinking));
            }
            let id = format!("toolu_{:024x}", self.index);
            let path = format!("{}/src/d{}/f{}.rs", self.cwd, self.index % 500, self.index);
            let call = json!([{ "type": "tool_use", "id": id, "name": "Read", "input": { "file_path": path, "limit": 200 } }]);
            lines.push(self.assistant(call));
            let output = self.words.code(1 + self.index % 7);
            lines.push(self.entry(
                "user",
                json!({
                    "message": { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": id, "content": output }] },
                    "toolUseResult": { "type": "text", "file": { "filePath": path, "content": output, "numLines": 40 } },
                }),
            ));
            for event in ["PreToolUse", "PostToolUse"] {
                let attachment = json!({
                    "type": "hook_success",
                    "hookName": format!("{event}:Read"),
                    "toolUseID": id,
                    "hookEvent": event,
                    "content": "",
                    "stdout": self.words.sentence(40),
                    "stderr": "",
                    "exitCode": 0,
                    "command": "silt hook claude",
                    "durationMs": 12,
                });
                lines.push(self.entry("attachment", json!({ "attachment": attachment })));
            }
        }
        let reply = json!([{ "type": "text", "text": self.words.sentence(25) }]);
        lines.push(self.assistant(reply));
        lines
    }

    fn assistant(&mut self, content: Value) -> String {
        let message = json!({
            "model": "claude-opus-4",
            "id": format!("msg_{:024x}", self.index),
            "type": "message",
            "role": "assistant",
            "content": content,
            "stop_reason": "tool_use",
            "stop_sequence": null,
            "usage": {
                "input_tokens": 3,
                "cache_creation_input_tokens": 512,
                "cache_read_input_tokens": 98_304,
                "output_tokens": 220,
                "service_tier": "standard",
                "cache_creation": { "ephemeral_1h_input_tokens": 512, "ephemeral_5m_input_tokens": 0 },
            },
        });
        self.entry("assistant", json!({ "message": message, "requestId": format!("req_{:024x}", self.index) }))
    }

    fn entry(&mut self, kind: &str, fields: Value) -> String {
        self.index += 1;
        let uuid = format!("00000000-0000-4000-8000-{:012x}", self.index);
        let mut line = json!({
            "parentUuid": self.parent,
            "isSidechain": false,
            "type": kind,
            "uuid": uuid,
            "timestamp": format!("2026-10-20T10:{:02}:{:02}.000Z", (self.index / 60) % 60, self.index % 60),
            "userType": "external",
            "entrypoint": "cli",
            "cwd": self.cwd,
            "sessionId": SESSION_ID,
            "version": "2.1.0",
            "gitBranch": "main",
        });
        if let (Value::Object(line), Value::Object(fields)) = (&mut line, fields) {
            line.extend(fields);
        }
        self.parent = Some(uuid);
        line.to_string()
    }
}

/// Text from a fixed-seed generator, so each run writes the same bytes and
/// the search index sees varied words, as it does with real sessions.
struct Words(u64);

const WORDS: &[&str] = &[
    "the",
    "module",
    "function",
    "returns",
    "error",
    "store",
    "delta",
    "thread",
    "family",
    "fork",
    "merge",
    "commit",
    "checkout",
    "transcript",
    "session",
    "agent",
    "hook",
    "test",
    "builds",
    "passes",
    "fails",
    "reads",
    "writes",
    "index",
    "search",
    "query",
    "row",
    "chunk",
    "snapshot",
    "replica",
    "sync",
    "server",
    "daemon",
    "socket",
    "request",
    "answer",
    "budget",
    "measure",
    "parse",
    "line",
    "offset",
    "batch",
    "commit",
    "lock",
    "turn",
    "tool",
    "result",
    "file",
    "path",
    "rename",
    "diff",
    "patch",
    "review",
    "approve",
    "pull",
    "push",
    "branch",
    "graph",
    "history",
    "inbox",
    "notice",
    "project",
    "clone",
    "remote",
    "blob",
    "hash",
    "encode",
    "decode",
    "frontier",
    "version",
    "lamport",
    "actor",
    "span",
    "tree",
    "inode",
    "text",
    "anchor",
    "cursor",
    "range",
    "slice",
    "vector",
    "map",
    "set",
];

impl Words {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn word(&mut self) -> &'static str {
        #[expect(clippy::cast_possible_truncation, reason = "taken modulo a short list")]
        let index = (self.next() % WORDS.len() as u64) as usize;
        WORDS[index]
    }

    fn sentence(&mut self, words: usize) -> String {
        (0..words).map(|_| self.word()).collect::<Vec<_>>().join(" ")
    }

    fn token(&mut self, length: usize) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        (0..length).map(|_| char::from(ALPHABET[(self.next() % 64) as usize])).collect()
    }

    /// About `lines` × 10 lines of Rust-looking source, as a file read
    /// returns.
    fn code(&mut self, lines: usize) -> String {
        let mut code = String::new();
        for number in 0..lines * 10 {
            let (name, other) = (self.word(), self.word());
            code.push_str(&format!(
                "{number:>6}\tpub fn {name}_{other}(&self) -> Result<{}, Error> {{ self.{other}() }}\n",
                self.word()
            ));
        }
        code
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_transcript_every_time() {
        let mut first = Transcript::new("/repo");
        let mut second = Transcript::new("/repo");
        assert_eq!(first.prompt(), second.prompt());
    }

    #[test]
    fn every_line_is_json_chained_to_the_one_before() {
        let lines = Transcript::new("/repo").prompt();
        let mut parent = Value::Null;
        for line in &lines {
            let value: Value = serde_json::from_str(line).expect("json");
            assert_eq!(value["parentUuid"], parent);
            assert_eq!(value["sessionId"], SESSION_ID);
            parent = value["uuid"].clone();
        }
    }

    #[test]
    fn lines_are_about_as_big_as_a_real_sessions() {
        let mut transcript = Transcript::new("/home/someone/project");
        let lines: Vec<String> = (0..50).flat_map(|_| transcript.prompt()).collect();
        let average = lines.iter().map(String::len).sum::<usize>() / lines.len();
        assert!((1_200..2_600).contains(&average), "{average} bytes a line");
    }
}
