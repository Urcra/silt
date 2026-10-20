//! Removes secrets from deltas before they are pushed (ADR 0009): values of
//! environment variables whose names look secret, and well-known token
//! formats. Conversation lines get `[REDACTED]`; text edits keep their length
//! (one `*` per character) so CRDT positions stay valid. Redaction is
//! deterministic, so pushing the same delta again gives the same bytes.

use std::env;

use silt_crdt::TextOp;
use silt_log::{ConvOp, Delta, Op};

const MIN_SECRET_LEN: usize = 8;
const MIN_TOKEN_BODY: usize = 16;
const SECRET_NAME_PARTS: [&str; 7] = ["KEY", "TOKEN", "SECRET", "PASSWORD", "PASSWD", "CREDENTIAL", "AUTH"];
const TOKEN_PREFIXES: [&str; 9] = ["github_pat_", "ghp_", "gho_", "ghs_", "sk-ant-", "sk-", "xoxb-", "xoxp-", "AKIA"];

#[derive(Debug, Clone, Default)]
pub struct Redactor {
    secrets: Vec<String>,
}

impl Redactor {
    pub fn new(mut secrets: Vec<String>) -> Self {
        secrets.retain(|secret| secret.chars().count() >= MIN_SECRET_LEN);
        secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        secrets.dedup();
        Self { secrets }
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

    pub fn redact_delta(&self, delta: &Delta) -> Delta {
        let mut delta = delta.clone();
        match &mut delta.op {
            Op::Text {
                op: TextOp::Insert { content, .. },
                ..
            } => *content = self.mask(content),
            Op::Conv(ConvOp::Native { line, .. }) => *line = self.replace(line),
            Op::Conv(ConvOp::Title(title)) => *title = self.replace(title),
            _ => {}
        }
        delta
    }

    /// `text` with every secret replaced by `[REDACTED]`.
    pub fn replace(&self, text: &str) -> String {
        self.apply(text, |_| "[REDACTED]".to_owned())
    }

    /// `text` with every secret's characters replaced by `*`, keeping length.
    pub fn mask(&self, text: &str) -> String {
        self.apply(text, |secret| "*".repeat(secret.chars().count()))
    }

    fn apply(&self, text: &str, cover: impl Fn(&str) -> String) -> String {
        let mut out = text.to_owned();
        for secret in &self.secrets {
            if out.contains(secret.as_str()) {
                out = out.replace(secret.as_str(), &cover(secret));
            }
        }
        for (start, end) in tokens(&out).into_iter().rev() {
            let covered = cover(&out[start..end]);
            out.replace_range(start..end, &covered);
        }
        out
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

    #[test]
    fn replaces_known_secrets_and_token_formats() {
        let redactor = Redactor::new(vec!["hunter2hunter2".into(), "short".into()]);
        let line = r#"{"text":"pw hunter2hunter2 and ghp_abcdefghijklmnopqrstuvwxyz123456 and short and sk-ab"}"#;
        assert_eq!(
            redactor.replace(line),
            r#"{"text":"pw [REDACTED] and [REDACTED] and short and sk-ab"}"#
        );
    }

    #[test]
    fn masking_keeps_the_length() {
        let redactor = Redactor::new(vec!["s3cr3t-value".into()]);
        let text = "KEY=s3cr3t-value\nAWS=AKIAABCDEFGHIJKLMNOP\n";
        let masked = redactor.mask(text);
        assert_eq!(masked.chars().count(), text.chars().count());
        assert_eq!(masked, "KEY=************\nAWS=********************\n");
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        let redactor = Redactor::new(Vec::new());
        let text = "risk-free task-runner sk-ip ask-me";
        assert_eq!(redactor.replace(text), text);
    }
}
