//! Keeps secrets off the server (ADR 0009, ADR 0015): values of environment
//! variables whose names look secret, values listed in a secrets file, and
//! well-known token formats.
//!
//! - **Redacting.** Conversation text gets `[REDACTED:<id>]`; text edits get
//!   a mask of the same length, so CRDT positions stay valid. Both name the
//!   secret by a short hash without giving it away, and are deterministic, so
//!   pushing the same delta again gives the same bytes.
//! - **Revealing.** An [`Overlay`] holds the secrets a machine knows and puts
//!   them back wherever their placeholders or masks show up.

use std::collections::HashSet;
use std::env;
use std::path::Path;
use std::sync::{LazyLock, OnceLock};

use aho_corasick::{AhoCorasick, Input, MatchKind};
use sha2::{Digest, Sha256};
use silt_crdt::TextOp;
use silt_log::{CommentOp, ConvOp, Delta, MrOp, Op};

const MIN_SECRET_LEN: usize = 8;
const MIN_TOKEN_BODY: usize = 16;
/// Hex digits of a secret's hash in its placeholder and mask: enough to
/// tell a machine's secrets apart, too few to check guesses against.
const ID_LEN: usize = 12;
/// Parts of an environment variable's name, split at `_`, that mark its
/// value as a secret: a part equal to one of these, or ending in one of the
/// first five (`APIKEY`, `GHTOKEN`).
const SECRET_NAME_PARTS: [&str; 7] = ["KEY", "TOKEN", "SECRET", "PASSWORD", "PASSWD", "CREDENTIAL", "AUTH"];
/// Name parts that say the value is about a secret rather than one.
const NOT_SECRET_PARTS: [&str; 12] = ["URL", "URI", "PATH", "FILE", "DIR", "SOCK", "HOST", "PORT", "NAME", "EMAIL", "USER", "ID"];
/// Token prefixes rare enough in ordinary text that an insert ending in one
/// is taken for a token still being typed.
const STRONG_PREFIXES: [&str; 8] = ["github_pat_", "ghp_", "gho_", "ghs_", "sk-ant-", "xoxb-", "xoxp-", "AKIA"];
const TOKEN_PREFIXES: [&str; 9] = ["github_pat_", "ghp_", "gho_", "ghs_", "sk-ant-", "sk-", "xoxb-", "xoxp-", "AKIA"];

/// The secret's short hash, which names it in placeholders and masks.
pub fn id(secret: &str) -> String {
    Sha256::digest(secret.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect::<String>()[..ID_LEN].to_owned()
}

/// What replaces the secret in conversation text.
pub fn placeholder(secret: &str) -> String {
    format!("[REDACTED:{}]", id(secret))
}

/// What replaces the secret in a file: `*`, its id, then `*` up to the
/// secret's length in characters.
pub fn mask(secret: &str) -> String {
    let length = secret.chars().count();
    if length < 3 {
        return "*".repeat(length);
    }
    let shown = id(secret);
    let shown = &shown[..ID_LEN.min(length.saturating_sub(2))];
    format!("*{shown}{}", "*".repeat(length - 1 - shown.len()))
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

    pub fn redact_delta(&self, delta: &Delta) -> Redacted {
        let mut delta = delta.clone();
        let mut found = Vec::new();
        let mut text = |text: &mut String| *text = self.cover(text, placeholder, &mut found);
        match &mut delta.op {
            Op::Text { op: TextOp::Insert { content, .. }, .. } => {
                *content = self.cover_partial(&self.cover(content, mask, &mut found))
            }
            Op::Conv(ConvOp::Native { line, .. }) => *line = self.cover_json(line, &mut found),
            Op::Conv(ConvOp::Title(title)) => text(title),
            Op::Conv(ConvOp::Merged { title, summary, .. }) | Op::Mr(MrOp::Opened { title, summary, .. }) => {
                text(title);
                text(summary);
            }
            Op::Mr(MrOp::Declined { reason, .. }) => text(reason),
            Op::Comment(CommentOp::Added { text: comment, .. }) => text(comment),
            _ => {}
        }
        Redacted { delta, found }
    }

    /// `text` with every secret replaced by its placeholder.
    pub fn replace(&self, text: &str) -> String {
        self.cover(text, placeholder, &mut Vec::new())
    }

    /// `text` with every secret masked, keeping its length.
    pub fn mask_text(&self, text: &str) -> String {
        self.cover(text, mask, &mut Vec::new())
    }

    /// `text` with every secret replaced by its placeholder, and the secrets.
    pub fn replace_found(&self, text: &str) -> (String, Vec<String>) {
        let mut found = Vec::new();
        (self.cover(text, placeholder, &mut found), found)
    }

    #[cfg(test)]
    fn cover_json_found(&self, line: &str) -> (String, Vec<String>) {
        let mut found = Vec::new();
        (self.cover_json(line, &mut found), found)
    }

    /// A transcript line is JSON: secrets appear there escaped.
    fn cover_json(&self, line: &str, found: &mut Vec<String>) -> String {
        let matchers = self.matchers();
        let covered = self.cover_with(line, &matchers.json, |pattern| matchers.json_secrets[pattern], placeholder, found);
        self.cover_tokens(covered, placeholder, found)
    }

    /// An insert that ends in what may be the start of a secret, a token
    /// prefix or the beginning of a known secret, is masked: the rest may
    /// arrive in a later edit, and a secret typed in pieces must not leave
    /// in pieces (ADR 0016).
    fn cover_partial(&self, content: &str) -> String {
        let tail_len = content
            .chars()
            .rev()
            .take_while(|char| char.is_ascii_alphanumeric() || matches!(char, '_' | '-' | '+' | '/' | '='))
            .count();
        if tail_len == 0 {
            return content.to_owned();
        }
        let split = content.char_indices().rev().nth(tail_len - 1).map_or(0, |(at, _)| at);
        let tail = &content[split..];
        let started = STRONG_PREFIXES.iter().any(|prefix| tail.starts_with(prefix))
            || (tail.len() >= 4 && self.secrets.iter().any(|secret| secret.starts_with(tail) && secret != tail));
        if started { format!("{}{}", &content[..split], mask(tail)) } else { content.to_owned() }
    }

    fn cover(&self, text: &str, cover: impl Fn(&str) -> String, found: &mut Vec<String>) -> String {
        let covered = self.cover_with(text, &self.matchers().plain, |pattern| pattern, &cover, found);
        self.cover_tokens(covered, cover, found)
    }

    /// `text` with each match of `matcher` covered as the secret
    /// `secret_of` names for its pattern.
    fn cover_with(
        &self,
        text: &str,
        matcher: &AhoCorasick,
        secret_of: impl Fn(usize) -> usize,
        cover: impl Fn(&str) -> String,
        found: &mut Vec<String>,
    ) -> String {
        let mut out = String::with_capacity(text.len());
        matcher.replace_all_with(text, &mut out, |matched, _, out| {
            let secret = &self.secrets[secret_of(matched.pattern().as_usize())];
            out.push_str(&cover(secret));
            found.push(secret.clone());
            true
        });
        out
    }

    fn cover_tokens(&self, mut out: String, cover: impl Fn(&str) -> String, found: &mut Vec<String>) -> String {
        for (start, end) in tokens(&out).into_iter().rev() {
            let secret = out[start..end].to_owned();
            out.replace_range(start..end, &cover(&secret));
            found.push(secret);
        }
        found.sort();
        found.dedup();
        out
    }
}

/// The secrets one machine knows, to put back what was redacted. Never
/// leaves the machine.
#[derive(Debug, Clone, Default)]
pub struct Overlay {
    secrets: Vec<Known>,
    known: HashSet<String>,
    /// Built on first use from `secrets`.
    matcher: OnceLock<CoverMatcher>,
}

/// A secret with the forms that hide it, worked out once: each takes a hash.
#[derive(Debug, Clone)]
struct Known {
    secret: String,
    placeholder: String,
    mask: String,
}

/// Every placeholder and mask of the known secrets, longest first.
#[derive(Debug, Clone)]
struct CoverMatcher {
    automaton: AhoCorasick,
    /// Each pattern's secret, and whether the pattern is its mask.
    covers: Vec<(usize, bool)>,
}

impl CoverMatcher {
    fn new(secrets: &[Known]) -> Self {
        let mut covers: Vec<(&str, usize, bool)> = secrets
            .iter()
            .enumerate()
            .flat_map(|(index, known)| [(known.placeholder.as_str(), index, false), (known.mask.as_str(), index, true)])
            .collect();
        covers.sort_by_key(|(pattern, _, _)| std::cmp::Reverse(pattern.len()));
        Self {
            automaton: automaton(covers.iter().map(|(pattern, _, _)| pattern)),
            covers: covers.iter().map(|(_, index, is_mask)| (*index, *is_mask)).collect(),
        }
    }
}

impl Overlay {
    pub fn new(secrets: impl IntoIterator<Item = String>) -> Self {
        let mut overlay = Self::default();
        for secret in secrets {
            overlay.learn(secret);
        }
        overlay
    }

    /// Remembers `secret`; whether it is new.
    pub fn learn(&mut self, secret: String) -> bool {
        if secret.chars().count() < MIN_SECRET_LEN || self.known.contains(&secret) {
            return false;
        }
        self.known.insert(secret.clone());
        self.secrets.push(Known { placeholder: placeholder(&secret), mask: mask(&secret), secret });
        self.matcher = OnceLock::new();
        true
    }

    /// `text` with the placeholders and masks of known secrets put back.
    pub fn reveal(&self, text: &str) -> String {
        self.reveal_as(text, Hidden::Both, str::to_owned)
    }

    /// Like [`Overlay::reveal`] for JSON text, such as transcript lines:
    /// the secrets go back escaped.
    pub fn reveal_json(&self, text: &str) -> String {
        self.reveal_as(text, Hidden::Both, json_escaped)
    }

    fn reveal_as(&self, text: &str, hidden: Hidden, form: impl Fn(&str) -> String) -> String {
        if self.secrets.is_empty() || (!text.contains("[REDACTED:") && !text.contains('*')) {
            return text.to_owned();
        }
        let matcher = self.matcher.get_or_init(|| CoverMatcher::new(&self.secrets));
        let mut out = String::with_capacity(text.len());
        matcher.automaton.replace_all_with(text, &mut out, |matched, covered, out| {
            let (index, is_mask) = matcher.covers[matched.pattern().as_usize()];
            match hidden {
                Hidden::Masks if !is_mask => out.push_str(covered),
                Hidden::Both | Hidden::Masks => out.push_str(&form(&self.secrets[index].secret)),
            }
            true
        });
        out
    }

    /// The delta as it was before it was redacted, as far as this machine
    /// knows the secrets.
    pub fn reveal_delta(&self, delta: Delta) -> Delta {
        if self.secrets.is_empty() {
            return delta;
        }
        let mut delta = delta;
        let reveal = |text: &mut String| *text = self.reveal(text);
        match &mut delta.op {
            // Only masks keep an insert's length, which its span counts.
            Op::Text { op: TextOp::Insert { content, .. }, .. } => {
                *content = self.reveal_as(content, Hidden::Masks, str::to_owned)
            }
            Op::Conv(ConvOp::Native { line, .. }) => *line = self.reveal_as(line, Hidden::Both, json_escaped),
            Op::Conv(ConvOp::Title(title)) => reveal(title),
            Op::Conv(ConvOp::Merged { title, summary, .. }) | Op::Mr(MrOp::Opened { title, summary, .. }) => {
                reveal(title);
                reveal(summary);
            }
            Op::Mr(MrOp::Declined { reason, .. }) => reveal(reason),
            Op::Comment(CommentOp::Added { text, .. }) => reveal(text),
            _ => {}
        }
        delta
    }
}

/// Which redacted forms to put secrets back into.
#[derive(Clone, Copy)]
enum Hidden {
    Both,
    Masks,
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
    use super::*;

    const TOKEN: &str = "ghp_abcdefghijklmnopqrstuvwxyz123456";

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
    fn masks_keep_the_length_and_name_the_secret() {
        let redactor = Redactor::new(vec!["s3cr3t-value".into()]);
        let text = "KEY=s3cr3t-value\nAWS=AKIAABCDEFGHIJKLMNOP\n";
        let masked = redactor.cover(text, mask, &mut Vec::new());
        assert_eq!(masked.chars().count(), text.chars().count());
        assert!(masked.starts_with(&format!("KEY=*{}", &id("s3cr3t-value")[..10])));
        assert_eq!(mask("12345678"), format!("*{}*", &id("12345678")[..6]));
        assert_eq!(mask(TOKEN).len(), TOKEN.len());
    }

    #[test]
    fn the_overlay_puts_back_what_it_knows() {
        let redactor = Redactor::new(vec!["s3cr3t-value".into()]);
        let text = format!("KEY=s3cr3t-value token {TOKEN} other ghp_zzzzzzzzzzzzzzzzzzzzzzzzzz");
        let replaced = redactor.replace(&text);
        let masked = redactor.cover(&text, mask, &mut Vec::new());
        let overlay = Overlay::new(["s3cr3t-value".to_owned(), TOKEN.to_owned()]);
        let partly = format!("KEY=s3cr3t-value token {TOKEN} other {}", placeholder("ghp_zzzzzzzzzzzzzzzzzzzzzzzzzz"));
        assert_eq!(overlay.reveal(&replaced), partly);
        assert_eq!(overlay.reveal(&masked).len(), text.len());
        assert!(overlay.reveal(&masked).contains(TOKEN));
        assert_eq!(Overlay::default().reveal(&replaced), replaced);
    }

    #[test]
    fn many_secrets_cost_one_pass() {
        let secrets: Vec<String> = (0..5_000).map(|n| format!("secret-value-{n:05}")).collect();
        let redactor = Redactor::new(secrets.clone());
        let overlay = Overlay::new(secrets);
        let text = "plain words and secret-value-04321 and \"secret-value-00007\" ".repeat(2_000);
        let started = std::time::Instant::now();
        let (replaced, found) = redactor.replace_found(&text);
        assert_eq!(found, ["secret-value-00007", "secret-value-04321"]);
        assert_eq!(overlay.reveal(&replaced), text);
        let masked = redactor.mask_text(&text);
        assert_eq!(overlay.reveal(&masked), text);
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "{:?}", started.elapsed());
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        let redactor = Redactor::new(Vec::new());
        let text = "risk-free task-runner sk-ip ask-me";
        assert_eq!(redactor.replace(text), text);
        assert_eq!(Overlay::new(["s3cr3t-value".to_owned()]).reveal("a * b ** [REDACTED]"), "a * b ** [REDACTED]");
    }

    #[test]
    fn inserts_get_masks_back_and_keep_their_length() {
        use silt_crdt::InodeId;
        use silt_types::{ActorId, DeltaId, DeltaSpan, Frontier, Lamport, ThreadId};
        let insert = |content: String| Delta {
            span: DeltaSpan::new(ActorId(1), Lamport(1), content.chars().count() as u64),
            parents: Frontier::root(),
            thread: ThreadId(1),
            author: silt_log::Author::User,
            time_ms: 0,
            op: Op::Text { inode: InodeId(DeltaId::new(ActorId(1), Lamport(0))), op: TextOp::Insert { pos: 0, content } },
        };
        let overlay = Overlay::new([TOKEN.to_owned()]);
        let masked = overlay.reveal_delta(insert(format!("x {}", mask(TOKEN))));
        assert!(matches!(&masked.op, Op::Text { op: TextOp::Insert { content, .. }, .. } if *content == format!("x {TOKEN}")));
        let pasted = format!("x {}", placeholder(TOKEN));
        let kept = overlay.reveal_delta(insert(pasted.clone()));
        assert!(
            matches!(&kept.op, Op::Text { op: TextOp::Insert { content, .. }, .. } if *content == pasted),
            "a placeholder in a file stays"
        );
    }

    #[test]
    fn transcript_lines_are_matched_and_revealed_as_json() {
        let redactor = Redactor::new(vec![r#"pa"ss\word12"#.to_owned()]);
        let line = r#"{"text":"pw pa\"ss\\word12\nghp_abcdefghijklmnopqrstuvwxyz123456 done"}"#;
        let (redacted, found) = redactor.cover_json_found(line);
        assert!(!redacted.contains("word12") && !redacted.contains("ghp_abcdef"), "{redacted}");
        assert_eq!(found.len(), 2);
        let overlay = Overlay::new(found);
        let mut delta_line = redacted.clone();
        delta_line = overlay.reveal_as(&delta_line, Hidden::Both, json_escaped);
        assert_eq!(delta_line, line);
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

    #[test]
    fn a_secret_typed_in_pieces_is_masked_from_the_first_piece() {
        let redactor = Redactor::new(vec!["correcthorsebattery".to_owned()]);
        assert_eq!(redactor.cover_partial("key = ghp_ab"), format!("key = {}", mask("ghp_ab")));
        assert_eq!(redactor.cover_partial("pw: correct"), format!("pw: {}", mask("correct")));
        assert_eq!(redactor.cover_partial("let total = sum"), "let total = sum");
        assert_eq!(mask("a"), "*");
    }
}
