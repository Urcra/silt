//! What the stand-ins for agent CLIs in `src/bin` share. They write each
//! agent's own session format (docs/research/agents.md), so the tests run
//! on every platform without the real agents. A first run records a prompt
//! and an edit of `src/main.rs`; a resumed run checks the restored session
//! names the current directory and mentions every `FAKE_EXPECT` word
//! (dashes for spaces), then adds `FAKE_SAY` and the line `FAKE_LINE`.

use std::env;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

pub mod codex;

/// Ends a stand-in with a code a test can tell apart, and why.
pub struct Failed(pub u8, pub String);

pub type Step = Result<(), Failed>;

pub fn exit(result: Step) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failed(code, message)) => {
            if !message.is_empty() {
                eprintln!("{message}");
            }
            ExitCode::from(code)
        }
    }
}

pub fn io(error: std::io::Error) -> Failed {
    Failed(1, error.to_string())
}

/// Gives a hook its input. A hook may exit without reading it, as `true`
/// does, and the agent goes on, so a closed pipe is no failure.
pub fn feed_hook(child: &mut std::process::Child, input: &[u8]) -> Step {
    match child.stdin.take().expect("piped").write_all(input) {
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        other => other.map_err(io),
    }
}

/// Appends `text`, making the file and its folder as an agent would, as in
/// a chat's empty folder (ADR 0059).
pub fn append(path: &Path, text: &str) -> Step {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io)?;
    }
    fs::OpenOptions::new().create(true).append(true).open(path).and_then(|mut file| file.write_all(text.as_bytes())).map_err(io)
}

pub fn write(path: &Path, bytes: &[u8]) -> Step {
    fs::write(path, bytes).map_err(io)
}

pub fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

pub fn cwd() -> Result<PathBuf, Failed> {
    env::current_dir().map_err(|error| Failed(1, format!("no current directory: {error}")))
}

/// A directory the test names in `variable`.
pub fn env_dir(variable: &str) -> Result<PathBuf, Failed> {
    env::var_os(variable).map(PathBuf::from).ok_or_else(|| Failed(1, format!("{variable} is unset")))
}

pub fn env_or(variable: &str, default: &str) -> String {
    env::var(variable).unwrap_or_else(|_| default.to_owned())
}

/// The file the stand-ins edit, as the agents name it.
pub fn main_rs(cwd: &Path) -> PathBuf {
    cwd.join("src").join("main.rs")
}

/// The JSON records of a JSON Lines file's text.
pub fn records(text: &str) -> Vec<Value> {
    text.lines().filter_map(|line| serde_json::from_str(line).ok()).collect()
}

/// Whether `value` holds `key` with the string `wanted`, at any depth.
pub fn has(value: &Value, key: &str, wanted: &str) -> bool {
    match value {
        Value::Object(map) => {
            map.iter().any(|(name, inner)| (name == key && inner.as_str() == Some(wanted)) || has(inner, key, wanted))
        }
        Value::Array(items) => items.iter().any(|inner| has(inner, key, wanted)),
        _ => false,
    }
}

/// Whether any string in `value`, at any depth, contains `wanted`.
pub fn mentions(value: &Value, wanted: &str) -> bool {
    match value {
        Value::String(text) => text.contains(wanted),
        Value::Object(map) => map.values().any(|inner| mentions(inner, wanted)),
        Value::Array(items) => items.iter().any(|inner| mentions(inner, wanted)),
        _ => false,
    }
}

/// Fails with code 6, showing `text`, unless it mentions every
/// `FAKE_EXPECT` word, dashes standing for spaces.
pub fn expect_words(text: &str, what: &str) -> Step {
    for needle in env_or("FAKE_EXPECT", "").split_whitespace() {
        let needle = needle.replace('-', " ");
        if !text.contains(&needle) {
            return Err(Failed(6, format!("{what} lacks {needle}\n{text}")));
        }
    }
    Ok(())
}

pub fn millis() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis())
}

/// A moment in UTC, broken into its calendar fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Utc {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl Utc {
    pub fn now() -> Self {
        Self::at(i64::try_from(millis() / 1000).unwrap_or(0))
    }

    /// From seconds since the epoch, by Howard Hinnant's days-to-civil.
    pub fn at(seconds: i64) -> Self {
        let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
        let shifted = days + 719_468;
        let era = shifted.div_euclid(146_097);
        let day_of_era = shifted.rem_euclid(146_097);
        let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month_index = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * month_index + 2) / 5 + 1;
        let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
        let field = |value: i64| u32::try_from(value).unwrap_or(0);
        Self {
            year: year_of_era + era * 400 + i64::from(month <= 2),
            month: field(month),
            day: field(day),
            hour: field(rest / 3600),
            minute: field(rest % 3600 / 60),
            second: field(rest % 60),
        }
    }

    /// RFC 3339 with milliseconds, as the agents stamp their records.
    pub fn rfc3339(self) -> String {
        format!("{}-{:02}-{:02}T{:02}:{:02}:{:02}.000Z", self.year, self.month, self.day, self.hour, self.minute, self.second)
    }

    /// The date and time as file names carry them: `2026-10-24T18-05-09`.
    pub fn file_stamp(self) -> String {
        format!("{}-{:02}-{:02}T{:02}-{:02}-{:02}", self.year, self.month, self.day, self.hour, self.minute, self.second)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_the_calendar_s() {
        assert_eq!(Utc::at(0).rfc3339(), "1970-01-01T00:00:00.000Z");
        assert_eq!(Utc::at(1_792_871_109).rfc3339(), "2026-10-24T19:45:09.000Z");
        assert_eq!(Utc::at(951_782_400).file_stamp(), "2000-02-29T00-00-00");
    }

    #[test]
    fn records_are_searched_at_any_depth() {
        let record = serde_json::json!({"payload": {"cwd": "/w/repo", "items": [{"path": "/w/repo/src/main.rs"}]}});
        assert!(has(&record, "cwd", "/w/repo") && !has(&record, "cwd", "/elsewhere"));
        assert!(mentions(&record, "/w/repo/src/main.rs") && !mentions(&record, "nowhere"));
    }
}
