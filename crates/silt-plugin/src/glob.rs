//! Globs over a call's declared fields, such as `acme/*` for a repo or
//! `ada/**` for a branch: `*` within one `/`-separated part, `**` across
//! parts. Never regexes over free text, which shell expansion defeats.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(transparent)]
pub struct Glob(pub String);

impl Glob {
    pub fn new(pattern: impl Into<String>) -> Self {
        Self(pattern.into())
    }

    pub fn matches(&self, value: &str) -> bool {
        matches(self.0.as_bytes(), value.as_bytes())
    }
}

fn matches(pattern: &[u8], value: &[u8]) -> bool {
    match pattern {
        [] => value.is_empty(),
        [b'*', b'*', rest @ ..] => (0..=value.len()).any(|skip| matches(rest, &value[skip..])),
        [b'*', rest @ ..] => {
            let end = value.iter().position(|&byte| byte == b'/').unwrap_or(value.len());
            (0..=end).any(|skip| matches(rest, &value[skip..]))
        }
        [first, rest @ ..] => value.first() == Some(first) && matches(rest, &value[1..]),
    }
}

/// How specific a pattern is: more literal characters, then fewer
/// wildcards, win when two match.
pub fn specificity(pattern: &str) -> (usize, std::cmp::Reverse<usize>) {
    let wildcards = pattern.matches('*').count();
    (pattern.len() - wildcards, std::cmp::Reverse(wildcards))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_star_stays_within_a_part_and_two_cross_them() {
        let one = Glob::new("acme/*");
        assert!(one.matches("acme/api"));
        assert!(!one.matches("acme/api/sub"));
        assert!(!one.matches("acme-labs/api"));
        assert!(Glob::new("ada/**").matches("ada/retry/uploads"));
        assert!(Glob::new("**").matches("anything/at/all"));
        assert!(Glob::new("v*-rc").matches("v2-rc"));
        assert!(!Glob::new("main").matches("main2"));
        assert!(Glob::new("*").matches(""));
    }

    #[test]
    fn the_more_literal_pattern_is_more_specific() {
        assert!(specificity("acme/api") > specificity("acme/*"));
        assert!(specificity("acme/*") > specificity("*/*"));
        assert!(specificity("acme/a*") > specificity("acme/*"));
    }
}
