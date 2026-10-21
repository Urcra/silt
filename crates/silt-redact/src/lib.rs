//! Keeps secrets out of conversations that leave the machine (ADR 0009,
//! ADR 0015, ADR 0019): values of environment variables whose names look
//! secret, values listed in a secrets file, and well-known token formats.
//!
//! Transcript lines, titles, merge request texts, comments and live drafts
//! get `[REDACTED:<id>]`, which names the secret by a short hash without
//! giving it away, and is deterministic, so pushing the same delta again
//! gives the same bytes. Files follow git: what git would commit is synced
//! as it is, and nothing puts secrets back.

use std::env;
use std::path::Path;
use std::sync::{LazyLock, OnceLock};

use aho_corasick::{AhoCorasick, Input, MatchKind};
use sha2::{Digest, Sha256};
use silt_log::{CommentOp, ConvOp, Delta, MrOp, Op};

const MIN_SECRET_LEN: usize = 8;
const MIN_TOKEN_BODY: usize = 16;
/// Hex digits of a secret's hash in its placeholder: enough to tell a
/// machine's secrets apart, too few to check guesses against.
const ID_LEN: usize = 12;
/// Parts of an environment variable's name, split at `_`, that mark its
/// value as a secret: a part equal to one of these, or ending in one of the
/// first five (`APIKEY`, `GHTOKEN`).
const SECRET_NAME_PARTS: [&str; 7] = ["KEY", "TOKEN", "SECRET", "PASSWORD", "PASSWD", "CREDENTIAL", "AUTH"];
/// Name parts that say the value is about a secret rather than one.
const NOT_SECRET_PARTS: [&str; 12] = ["URL", "URI", "PATH", "FILE", "DIR", "SOCK", "HOST", "PORT", "NAME", "EMAIL", "USER", "ID"];
const TOKEN_PREFIXES: [&str; 9] = ["github_pat_", "ghp_", "gho_", "ghs_", "sk-ant-", "sk-", "xoxb-", "xoxp-", "AKIA"];

/// The secret's short hash, which names it in placeholders.
pub fn id(secret: &str) -> String {
    Sha256::digest(secret.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect::<String>()[..ID_LEN].to_owned()
}

/// What replaces the secret.
pub fn placeholder(secret: &str) -> String {
    format!("[REDACTED:{}]", id(secret))
}

/// Values in a secrets file, one per line; blank lines and `#` comments are
/// skipped. A missing file has none.
pub fn read_secrets_file(path: &Path) -> std::io::Result<Vec<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            Ok(text.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')).map(str::to_owned).collect())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

/// Whether an environment variable called `name` holding `value` is a
/// secret.
fn is_secret_variable(name: &str, value: &str) -> bool {
    let name = name.to_ascii_uppercase();
    let parts: Vec<&str> = name.split('_').collect();
    let marked = parts.iter().any(|part| {
        SECRET_NAME_PARTS.iter().enumerate().any(|(index, marker)| *part == *marker || (index < 5 && part.ends_with(marker)))
    });
    let about = parts.last().is_some_and(|last| NOT_SECRET_PARTS.contains(last));
    let looks_like_path = value.starts_with(['/', '~', '.']);
    marked && !about && !looks_like_path
}

/// `secret` as it appears inside a JSON string.
fn json_escaped(secret: &str) -> String {
    let mut out = String::with_capacity(secret.len());
    for char in secret.chars() {
        match char {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            char if u32::from(char) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(char))),
            char => out.push(char),
        }
    }
    out
}

/// A delta made safe to push, and the secrets taken out of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Redacted {
    pub delta: Delta,
    pub found: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Redactor {
    secrets: Vec<String>,
    /// Built on first use from `secrets`.
    matchers: OnceLock<SecretMatchers>,
}

/// Finds every known secret in one pass, whatever their number.
#[derive(Debug, Clone)]
struct SecretMatchers {
    /// The secrets as they are.
    plain: AhoCorasick,
    /// The secrets also as they appear inside JSON strings, each pattern
    /// naming its secret in `json_secrets`.
    json: AhoCorasick,
    json_secrets: Vec<usize>,
}

/// An automaton preferring, where patterns start at the same place, the one
/// listed first: callers list longer ones first, so a secret containing
/// another is covered whole.
fn automaton<P: AsRef<[u8]>>(patterns: impl IntoIterator<Item = P>) -> AhoCorasick {
    AhoCorasick::builder()
        .match_kind(MatchKind::LeftmostFirst)
        .build(patterns)
        .expect("secrets are short enough for an automaton")
}

impl SecretMatchers {
    fn new(secrets: &[String]) -> Self {
        let mut json: Vec<(String, usize)> = secrets.iter().enumerate().map(|(index, secret)| (secret.clone(), index)).collect();
        json.extend(secrets.iter().enumerate().filter_map(|(index, secret)| {
            let escaped = json_escaped(secret);
            (escaped != *secret).then_some((escaped, index))
        }));
        json.sort_by_key(|(pattern, _)| std::cmp::Reverse(pattern.len()));
        Self {
            plain: automaton(secrets),
            json: automaton(json.iter().map(|(pattern, _)| pattern)),
            json_secrets: json.iter().map(|(_, index)| *index).collect(),
        }
    }
}

impl Redactor {
    pub fn new(secrets: Vec<String>) -> Self {
        let mut redactor = Self { secrets, matchers: OnceLock::new() };
        redactor.normalize();
        redactor
    }

    fn normalize(&mut self) {
        self.secrets.retain(|secret| secret.chars().count() >= MIN_SECRET_LEN);
        self.secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        self.secrets.dedup();
        self.matchers = OnceLock::new();
    }

    fn matchers(&self) -> &SecretMatchers {
        self.matchers.get_or_init(|| SecretMatchers::new(&self.secrets))
    }

    /// Secrets from this process's environment. Variables that aren't
    /// valid UTF-8 are skipped.
    pub fn from_env() -> Self {
        Self::new(
            env::vars_os()
                .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
                .filter(|(name, value)| is_secret_variable(name, value))
                .map(|(_, value)| value)
                .collect(),
        )
    }

    /// Also redacts `secrets`.
    #[must_use]
    pub fn with(mut self, secrets: impl IntoIterator<Item = String>) -> Self {
        self.secrets.extend(secrets);
        self.normalize();
        self
    }

    /// The secrets known by value, not by their format.
    pub fn secrets(&self) -> &[String] {
        &self.secrets
    }

    /// The delta with its conversation text redacted. File content follows
    /// git and goes as it is.
    pub fn redact_delta(&self, delta: &Delta) -> Redacted {
        let mut delta = delta.clone();
        let mut found = Vec::new();
        let mut text = |text: &mut String| *text = self.cover(text, &mut found);
        match &mut delta.op {
            Op::Conv(ConvOp::Native { line, .. }) => *line = self.cover_json(line, &mut found),
            Op::Conv(ConvOp::Title(title)) => text(title),
            Op::Conv(ConvOp::Merged { title, summary, .. }) | Op::Mr(MrOp::Opened { title, summary, .. }) => {
                text(title);
                text(summary);
            }
            Op::Mr(MrOp::Declined { reason, .. } | MrOp::ChangesRequested { reason, .. }) => text(reason),
            Op::Comment(CommentOp::Added { text: comment, .. } | CommentOp::Replied { text: comment, .. }) => text(comment),
            _ => {}
        }
        Redacted { delta, found }
    }

    /// `text` with every secret replaced by its placeholder.
    pub fn replace(&self, text: &str) -> String {
        self.cover(text, &mut Vec::new())
    }

    /// `text` with every secret replaced by its placeholder, and the secrets.
    pub fn replace_found(&self, text: &str) -> (String, Vec<String>) {
        let mut found = Vec::new();
        (self.cover(text, &mut found), found)
    }

    /// A transcript line is JSON: secrets appear there escaped.
    fn cover_json(&self, line: &str, found: &mut Vec<String>) -> String {
        let matchers = self.matchers();
        let covered = self.cover_with(line, &matchers.json, |pattern| matchers.json_secrets[pattern], found);
        cover_tokens(covered, found)
    }

    fn cover(&self, text: &str, found: &mut Vec<String>) -> String {
        let covered = self.cover_with(text, &self.matchers().plain, |pattern| pattern, found);
        cover_tokens(covered, found)
    }

    /// `text` with each match of `matcher` replaced by the placeholder of
    /// the secret `secret_of` names for its pattern.
    fn cover_with(
        &self,
        text: &str,
        matcher: &AhoCorasick,
        secret_of: impl Fn(usize) -> usize,
        found: &mut Vec<String>,
    ) -> String {
        let mut out = String::with_capacity(text.len());
        matcher.replace_all_with(text, &mut out, |matched, _, out| {
            let secret = &self.secrets[secret_of(matched.pattern().as_usize())];
            out.push_str(&placeholder(secret));
            found.push(secret.clone());
            true
        });
        out
    }
}

fn cover_tokens(mut out: String, found: &mut Vec<String>) -> String {
    for (start, end) in tokens(&out).into_iter().rev() {
        let secret = out[start..end].to_owned();
        out.replace_range(start..end, &placeholder(&secret));
        found.push(secret);
    }
    found.sort();
    found.dedup();
    out
}

/// Byte ranges of well-known token formats in `text`.
fn tokens(text: &str) -> Vec<(usize, usize)> {
    static PREFIXES: LazyLock<AhoCorasick> = LazyLock::new(|| automaton(TOKEN_PREFIXES));
    let mut found = Vec::new();
    let mut search = 0;
    while search < text.len() {
        let Some(next) = PREFIXES.find(Input::new(text).range(search..)) else {
            break;
        };
        let (start, prefix_len) = (next.start(), next.len());
        // `\n` and friends in JSON text end in a letter, yet are boundaries.
        let mut before = text[..start].chars().rev();
        let at_boundary = match (before.next(), before.next()) {
            (None, _) => true,
            (Some(char), _) if !char.is_ascii_alphanumeric() => true,
            (Some(_), Some('\\')) => true,
            _ => false,
        };
        let body = text[start + prefix_len..]
            .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
            .unwrap_or(text.len() - start - prefix_len);
        if at_boundary && body >= MIN_TOKEN_BODY {
            found.push((start, start + prefix_len + body));
            search = start + prefix_len + body;
        } else {
            search = start + prefix_len;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use silt_crdt::{InodeId, TextOp};
    use silt_types::{ActorId, DeltaId, DeltaSpan, Frontier, Lamport, ThreadId};

    use super::*;

    const TOKEN: &str = "ghp_abcdefghijklmnopqrstuvwxyz123456";

    fn delta(op: Op) -> Delta {
        Delta {
            span: DeltaSpan::new(ActorId(1), Lamport(1), op.units() as u64),
            parents: Frontier::root(),
            thread: ThreadId(1),
            author: silt_log::Author::User,
            time_ms: 0,
            op,
        }
    }

    #[test]
    fn replaces_known_secrets_and_token_formats_with_named_placeholders() {
        let redactor = Redactor::new(vec!["hunter2hunter2".into(), "short".into()]);
        let line = format!(r#"{{"text":"pw hunter2hunter2 and {TOKEN} and short and sk-ab"}}"#);
        let (redacted, found) = redactor.replace_found(&line);
        assert_eq!(
            redacted,
            format!(r#"{{"text":"pw {} and {} and short and sk-ab"}}"#, placeholder("hunter2hunter2"), placeholder(TOKEN))
        );
        assert_eq!(found, vec![TOKEN.to_owned(), "hunter2hunter2".to_owned()]);
        assert!(placeholder(TOKEN).starts_with("[REDACTED:") && placeholder(TOKEN).len() == 23);
    }

    #[test]
    fn file_edits_follow_git_and_conversation_is_redacted() {
        let redactor = Redactor::new(vec!["s3cr3t-value".into()]);
        let insert = delta(Op::Text {
            inode: InodeId(DeltaId::new(ActorId(1), Lamport(0))),
            op: TextOp::Insert { pos: 0, content: format!("KEY=s3cr3t-value {TOKEN}") },
        });
        let redacted = redactor.redact_delta(&insert);
        assert_eq!(redacted.delta, insert, "what git would commit goes as it is");
        assert!(redacted.found.is_empty());
        let title = redactor.redact_delta(&delta(Op::Conv(ConvOp::Title("use s3cr3t-value".into()))));
        assert_eq!(title.delta.op, Op::Conv(ConvOp::Title(format!("use {}", placeholder("s3cr3t-value")))));
        assert_eq!(title.found, ["s3cr3t-value"]);
    }

    #[test]
    fn many_secrets_cost_one_pass() {
        let secrets: Vec<String> = (0..5_000).map(|n| format!("secret-value-{n:05}")).collect();
        let redactor = Redactor::new(secrets);
        let text = "plain words and secret-value-04321 and \"secret-value-00007\" ".repeat(2_000);
        let started = std::time::Instant::now();
        let (replaced, found) = redactor.replace_found(&text);
        assert_eq!(found, ["secret-value-00007", "secret-value-04321"]);
        assert!(!replaced.contains("secret-value-0"));
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "{:?}", started.elapsed());
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        let redactor = Redactor::new(Vec::new());
        let text = "risk-free task-runner sk-ip ask-me";
        assert_eq!(redactor.replace(text), text);
    }

    #[test]
    fn transcript_lines_are_matched_as_json() {
        let redactor = Redactor::new(vec![r#"pa"ss\word12"#.to_owned()]);
        let line = r#"{"text":"pw pa\"ss\\word12\nghp_abcdefghijklmnopqrstuvwxyz123456 done"}"#;
        let mut found = Vec::new();
        let redacted = redactor.cover_json(line, &mut found);
        assert!(!redacted.contains("word12") && !redacted.contains("ghp_abcdef"), "{redacted}");
        assert_eq!(found.len(), 2);
        assert!(serde_json::from_str::<serde_json::Value>(&redacted).is_ok(), "still JSON");
    }

    #[test]
    fn only_secret_variables_count() {
        for (name, value) in [("OPENAI_API_KEY", "sk-x"), ("APIKEY", "abc"), ("DB_PASSWORD", "x"), ("GH_TOKEN", "x")] {
            assert!(is_secret_variable(name, value), "{name}");
        }
        for (name, value) in [
            ("GIT_AUTHOR_EMAIL", "me@example.com"),
            ("NEXTAUTH_URL", "http://localhost:3000"),
            ("SSH_AUTH_SOCK", "/run/user/1000/ssh"),
            ("TOKEN_FILE", "secrets/token"),
            ("AWS_ACCESS_KEY_ID", "AKIA..."),
        ] {
            assert!(!is_secret_variable(name, value), "{name}");
        }
    }
}
