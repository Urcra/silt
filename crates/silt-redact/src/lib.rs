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

use std::env;
use std::path::Path;

use sha2::{Digest, Sha256};
use silt_crdt::TextOp;
use silt_log::{CommentOp, ConvOp, Delta, MrOp, Op};

const MIN_SECRET_LEN: usize = 8;
const MIN_TOKEN_BODY: usize = 16;
/// Hex digits of a secret's hash in its placeholder and mask: enough to
/// tell a machine's secrets apart, too few to check guesses against.
const ID_LEN: usize = 12;
const SECRET_NAME_PARTS: [&str; 7] = ["KEY", "TOKEN", "SECRET", "PASSWORD", "PASSWD", "CREDENTIAL", "AUTH"];
const TOKEN_PREFIXES: [&str; 9] = ["github_pat_", "ghp_", "gho_", "ghs_", "sk-ant-", "sk-", "xoxb-", "xoxp-", "AKIA"];

/// The secret's short hash, which names it in placeholders and masks.
pub fn id(secret: &str) -> String {
    Sha256::digest(secret.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()[..ID_LEN]
        .to_owned()
}

/// What replaces the secret in conversation text.
pub fn placeholder(secret: &str) -> String {
    format!("[REDACTED:{}]", id(secret))
}

/// What replaces the secret in a file: `*`, its id, then `*` up to the
/// secret's length in characters.
pub fn mask(secret: &str) -> String {
    let length = secret.chars().count();
    let shown = id(secret);
    let shown = &shown[..ID_LEN.min(length.saturating_sub(2))];
    format!("*{shown}{}", "*".repeat(length - 1 - shown.len()))
}

/// Values in a secrets file, one per line; blank lines and `#` comments are
/// skipped.
pub fn read_secrets_file(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
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
}

impl Redactor {
    pub fn new(secrets: Vec<String>) -> Self {
        let mut redactor = Self { secrets };
        redactor.normalize();
        redactor
    }

    fn normalize(&mut self) {
        self.secrets.retain(|secret| secret.chars().count() >= MIN_SECRET_LEN);
        self.secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        self.secrets.dedup();
    }

    /// Secrets from this process's environment.
    pub fn from_env() -> Self {
        Self::new(
            env::vars()
                .filter(|(name, value)| {
                    let name = name.to_uppercase();
                    SECRET_NAME_PARTS.iter().any(|part| name.contains(part)) && !value.starts_with('/')
                })
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
            Op::Text {
                op: TextOp::Insert { content, .. },
                ..
            } => *content = self.cover(content, mask, &mut found),
            Op::Conv(ConvOp::Native { line, .. }) => text(line),
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

    /// `text` with every secret replaced by its placeholder, and the secrets.
    pub fn replace_found(&self, text: &str) -> (String, Vec<String>) {
        let mut found = Vec::new();
        (self.cover(text, placeholder, &mut found), found)
    }

    fn cover(&self, text: &str, cover: impl Fn(&str) -> String, found: &mut Vec<String>) -> String {
        let mut out = text.to_owned();
        for secret in &self.secrets {
            if out.contains(secret.as_str()) {
                out = out.replace(secret.as_str(), &cover(secret));
                found.push(secret.clone());
            }
        }
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
    /// Longest first, so a secret containing another is revealed whole.
    secrets: Vec<String>,
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
        if secret.chars().count() < MIN_SECRET_LEN || self.secrets.contains(&secret) {
            return false;
        }
        let at = self.secrets.partition_point(|known| known.len() >= secret.len());
        self.secrets.insert(at, secret);
        true
    }

    /// `text` with the placeholders and masks of known secrets put back.
    pub fn reveal(&self, text: &str) -> String {
        let mut out = text.to_owned();
        if !out.contains("[REDACTED:") && !out.contains('*') {
            return out;
        }
        for secret in &self.secrets {
            for hidden in [placeholder(secret), mask(secret)] {
                if out.contains(&hidden) {
                    out = out.replace(&hidden, secret);
                }
            }
        }
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
            Op::Text {
                op: TextOp::Insert { content, .. },
                ..
            } => reveal(content),
            Op::Conv(ConvOp::Native { line, .. }) => reveal(line),
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

/// Byte ranges of well-known token formats in `text`.
fn tokens(text: &str) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut search = 0;
    while search < text.len() {
        let next = TOKEN_PREFIXES
            .iter()
            .filter_map(|prefix| text[search..].find(prefix).map(|at| (search + at, prefix.len())))
            .min();
        let Some((start, prefix_len)) = next else {
            break;
        };
        let at_boundary = text[..start]
            .chars()
            .next_back()
            .is_none_or(|before| !before.is_ascii_alphanumeric());
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
            format!(
                r#"{{"text":"pw {} and {} and short and sk-ab"}}"#,
                placeholder("hunter2hunter2"),
                placeholder(TOKEN)
            )
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
    fn leaves_ordinary_text_alone() {
        let redactor = Redactor::new(Vec::new());
        let text = "risk-free task-runner sk-ip ask-me";
        assert_eq!(redactor.replace(text), text);
        assert_eq!(Overlay::new(["s3cr3t-value".to_owned()]).reveal("a * b ** [REDACTED]"), "a * b ** [REDACTED]");
    }
}
