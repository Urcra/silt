//! How much of its plan an agent has used: Claude Code's 5-hour and weekly
//! limits, Codex's, Copilot's monthly premium requests, and Antigravity's
//! rolling and weekly buckets. Each is read the way the agent's own CLI reads
//! it, without refreshing a sign-in or reading Antigravity's OAuth tokens
//! directly (ADR 0095).
//!
//! Sans-I/O: [`Provider::ask`] says what to fetch, [`Provider::read`] reads
//! the answer; the daemon fetches.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Where the answers come from, and the page a person sees all of theirs on.
const CLAUDE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const CODEX_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const COPILOT_URL: &str = "https://api.github.com/copilot_internal/user";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    Claude,
    Codex,
    Copilot,
    Antigravity,
}

/// One request to make, with the sign-in it carries.
#[derive(Clone, PartialEq, Eq)]
pub struct Ask {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

impl std::fmt::Debug for Ask {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Ask").field("url", &self.url).finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlanUsage {
    /// The plan's name, such as Max or Plus, where the agent says.
    pub plan: Option<String>,
    pub limits: Vec<UsageLimit>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageLimit {
    pub name: String,
    /// 0 to 100.
    pub used_percent: f64,
    pub resets_at_ms: Option<i64>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UsageError {
    #[error("{0} isn't signed in to a plan here")]
    SignedOut(&'static str),
    #[error("{0} uses an API key here, which has no plan limits")]
    ApiKey(&'static str),
    #[error("{0}'s sign-in has expired; run it once to renew it")]
    Expired(&'static str),
    #[error("{0}'s usage answer couldn't be read")]
    Unreadable(&'static str),
}

impl Provider {
    /// The provider of a known agent (`silt_workspace::agent_key`).
    pub fn for_agent(key: &str) -> Option<Self> {
        match key {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "copilot" => Some(Self::Copilot),
            "agy" => Some(Self::Antigravity),
            _ => None,
        }
    }

    pub fn agent(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::Copilot => "Copilot",
            Self::Antigravity => "Antigravity",
        }
    }

    /// Where the person sees all of their usage.
    pub fn page(self) -> &'static str {
        match self {
            Self::Claude => "https://claude.ai/settings/usage",
            Self::Codex => "https://chatgpt.com/codex/settings/usage",
            Self::Copilot => "https://github.com/settings/copilot/features",
            Self::Antigravity => "https://antigravity.google/docs/plans",
        }
    }

    /// The variable naming the agent's home, and its default under `~`.
    pub fn home_variable(self) -> (&'static str, &'static str) {
        match self {
            Self::Claude => ("CLAUDE_CONFIG_DIR", ".claude"),
            Self::Codex => ("CODEX_HOME", ".codex"),
            Self::Copilot => ("COPILOT_HOME", ".copilot"),
            Self::Antigravity => ("GEMINI_CLI_HOME", ".gemini"),
        }
    }

    /// The agent's home for a run in `cwd`, whose workspace may set its own
    /// (ADR 0065), else this machine's.
    pub fn home(self, cwd: Option<&Path>) -> Option<PathBuf> {
        let (variable, dir) = self.home_variable();
        match cwd {
            Some(cwd) => crate::agent_home_in(variable, dir, cwd),
            None => crate::agent_home(variable, dir),
        }
    }

    /// What to fetch, with the sign-in kept in `home`, the agent's home for a
    /// run in that folder. Copilot's sign-in is GitHub's: `github` is a token
    /// found elsewhere ([`github_token`]).
    pub fn ask(self, home: &Path, github: Option<&str>, now_ms: i64) -> Result<Ask, UsageError> {
        match self {
            Self::Claude => claude_ask(home, now_ms),
            Self::Codex => codex_ask(home),
            Self::Copilot => {
                let token = github.ok_or(UsageError::SignedOut(self.agent()))?;
                Ok(Ask {
                    url: url_for("SILT_COPILOT_USAGE_URL", COPILOT_URL),
                    headers: vec![
                        ("Authorization".to_owned(), format!("token {token}")),
                        ("Accept".to_owned(), "application/json".to_owned()),
                    ],
                })
            }
            Self::Antigravity => Err(if antigravity_api_key(home) {
                UsageError::ApiKey(self.agent())
            } else {
                UsageError::Unreadable(self.agent())
            }),
        }
    }

    /// Reads the answer to [`Provider::ask`] (or `agy`'s `statusLine` JSON for
    /// [`Provider::Antigravity`]). `home` gives the plan's name where the
    /// answer doesn't.
    pub fn read(self, home: &Path, body: &str, now_ms: i64) -> Result<PlanUsage, UsageError> {
        let value: Value =
            serde_json::from_str(body.trim_start_matches('\u{feff}')).map_err(|_| UsageError::Unreadable(self.agent()))?;
        let usage = match self {
            Self::Claude => claude_usage(&value, claude_plan(home)),
            Self::Codex => codex_usage(&value, now_ms),
            Self::Copilot => copilot_usage(&value),
            Self::Antigravity => antigravity_usage(&value, now_ms),
        };
        usage.ok_or(UsageError::Unreadable(self.agent()))
    }

    /// What's known without asking: Codex writes its limits into each
    /// session as it runs, which stands in when its sign-in can't be used.
    pub fn recorded(self, home: &Path, now_ms: i64) -> Option<PlanUsage> {
        match self {
            Self::Codex => codex_recorded(home, now_ms),
            Self::Claude | Self::Copilot | Self::Antigravity => None,
        }
    }
}

/// Tests point each provider at a stand-in.
fn url_for(variable: &str, default: &str) -> String {
    std::env::var(variable).unwrap_or_else(|_| default.to_owned())
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn claude_oauth(home: &Path) -> Option<Value> {
    read_json(&home.join(".credentials.json"))?.get("claudeAiOauth").cloned()
}

fn claude_ask(home: &Path, now_ms: i64) -> Result<Ask, UsageError> {
    let agent = Provider::Claude.agent();
    let Some(oauth) = claude_oauth(home) else {
        // Signed in with a key, or not at all; Claude Code keeps a key in its global config.
        return Err(if std::env::var_os("ANTHROPIC_API_KEY").is_some() {
            UsageError::ApiKey(agent)
        } else {
            UsageError::SignedOut(agent)
        });
    };
    let token =
        oauth.get("accessToken").and_then(Value::as_str).filter(|token| !token.is_empty()).ok_or(UsageError::SignedOut(agent))?;
    if oauth.get("expiresAt").and_then(Value::as_i64).is_some_and(|expires| expires <= now_ms) {
        return Err(UsageError::Expired(agent));
    }
    Ok(Ask {
        url: url_for("SILT_CLAUDE_USAGE_URL", CLAUDE_URL),
        headers: vec![
            ("Authorization".to_owned(), format!("Bearer {token}")),
            ("anthropic-beta".to_owned(), "oauth-2025-04-20".to_owned()),
        ],
    })
}

fn claude_plan(home: &Path) -> Option<String> {
    claude_oauth(home)?.get("subscriptionType")?.as_str().map(title)
}

/// Claude's limits, as its `/usage` names them; others it may add are left out until named here.
const CLAUDE_LIMITS: [(&str, &str); 4] = [
    ("five_hour", "5-hour limit"),
    ("seven_day", "Weekly · all models"),
    ("seven_day_opus", "Weekly · Opus"),
    ("seven_day_sonnet", "Weekly · Sonnet"),
];

fn claude_usage(value: &Value, plan: Option<String>) -> Option<PlanUsage> {
    if let Some(listed) = value.get("limits").and_then(Value::as_array).filter(|listed| !listed.is_empty()) {
        return Some(PlanUsage { plan, limits: listed.iter().filter_map(claude_listed).collect() });
    }
    let limits: Vec<UsageLimit> = CLAUDE_LIMITS
        .iter()
        .filter_map(|(key, name)| {
            let limit = value.get(key)?;
            Some(UsageLimit {
                name: (*name).to_owned(),
                used_percent: limit.get("utilization")?.as_f64()?.clamp(0.0, 100.0),
                resets_at_ms: limit.get("resets_at").and_then(Value::as_str).and_then(parse_time),
            })
        })
        .collect();
    (!limits.is_empty() || value.get("five_hour").is_some()).then_some(PlanUsage { plan, limits })
}

/// One of the `limits` Claude lists, which name a weekly limit's model where it's
/// scoped to one; a kind not named here is left out.
fn claude_listed(limit: &Value) -> Option<UsageLimit> {
    let name = match limit.get("kind")?.as_str()? {
        "session" => "5-hour limit".to_owned(),
        "weekly_all" => "Weekly · all models".to_owned(),
        "weekly_scoped" => {
            let model = limit.get("scope")?.get("model")?.get("display_name")?.as_str()?;
            format!("Weekly · {model}")
        }
        _ => return None,
    };
    Some(UsageLimit {
        name,
        used_percent: limit.get("percent")?.as_f64()?.clamp(0.0, 100.0),
        resets_at_ms: limit.get("resets_at").and_then(Value::as_str).and_then(parse_time),
    })
}

fn codex_ask(home: &Path) -> Result<Ask, UsageError> {
    let agent = Provider::Codex.agent();
    let auth = read_json(&home.join("auth.json")).ok_or(UsageError::SignedOut(agent))?;
    let tokens = auth.get("tokens").filter(|tokens| tokens.is_object());
    let Some(tokens) = tokens else {
        return Err(if auth.get("OPENAI_API_KEY").and_then(Value::as_str).is_some() {
            UsageError::ApiKey(agent)
        } else {
            UsageError::SignedOut(agent)
        });
    };
    let token = tokens
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .ok_or(UsageError::SignedOut(agent))?;
    let mut headers =
        vec![("Authorization".to_owned(), format!("Bearer {token}")), ("Accept".to_owned(), "application/json".to_owned())];
    if let Some(account) = tokens.get("account_id").and_then(Value::as_str) {
        headers.push(("ChatGPT-Account-Id".to_owned(), account.to_owned()));
    }
    Ok(Ask { url: url_for("SILT_CODEX_USAGE_URL", CODEX_URL), headers })
}

/// A limit's name from how long its window is.
fn window_name(seconds: i64) -> String {
    match seconds {
        18_000 => "5-hour limit".to_owned(),
        604_800 => "Weekly limit".to_owned(),
        seconds if seconds > 0 && seconds % 86_400 == 0 => format!("{}-day limit", seconds / 86_400),
        seconds if seconds > 0 && seconds % 3600 == 0 => format!("{}-hour limit", seconds / 3600),
        _ => "Limit".to_owned(),
    }
}

/// A limit whose window has ended since is used up no more.
fn current(used_percent: f64, resets_at_ms: Option<i64>, now_ms: i64) -> (f64, Option<i64>) {
    match resets_at_ms {
        Some(reset) if reset <= now_ms => (0.0, None),
        reset => (used_percent.clamp(0.0, 100.0), reset),
    }
}

fn codex_usage(value: &Value, now_ms: i64) -> Option<PlanUsage> {
    let limits = value.get("rate_limit")?;
    let read = |window: &Value| {
        let seconds = window.get("limit_window_seconds").and_then(Value::as_i64).unwrap_or(0);
        let reset = window
            .get("reset_at")
            .and_then(Value::as_i64)
            .map(|at| at * 1000)
            .or_else(|| window.get("reset_after_seconds").and_then(Value::as_i64).map(|after| now_ms + after * 1000));
        let (used_percent, resets_at_ms) = current(window.get("used_percent")?.as_f64()?, reset, now_ms);
        Some(UsageLimit { name: window_name(seconds), used_percent, resets_at_ms })
    };
    Some(PlanUsage {
        plan: value.get("plan_type").and_then(Value::as_str).map(title),
        limits: ["primary_window", "secondary_window"]
            .iter()
            .filter_map(|key| limits.get(key).filter(|window| window.is_object()).and_then(read))
            .collect(),
    })
}

/// How far back to look for a session that recorded Codex's limits: its longest window.
const RECORDED_WITHIN_MS: i64 = 8 * 24 * 60 * 60 * 1000;

fn codex_recorded(home: &Path, now_ms: i64) -> Option<PlanUsage> {
    crate::codex::rollouts_since(home, now_ms - RECORDED_WITHIN_MS)
        .iter()
        .take(20)
        .find_map(|rollout| fs::read_to_string(rollout).ok().and_then(|text| codex_recorded_in(&text, now_ms)))
}

/// The last limits a rollout recorded, from its `token_count` events.
fn codex_recorded_in(rollout: &str, now_ms: i64) -> Option<PlanUsage> {
    rollout.lines().rev().find_map(|line| {
        if !line.contains("\"rate_limits\"") {
            return None;
        }
        let record: Value = serde_json::from_str(line).ok()?;
        let limits = record.get("payload")?.get("rate_limits").filter(|limits| limits.is_object())?;
        let at = record.get("timestamp").and_then(Value::as_str).and_then(parse_time).unwrap_or(now_ms);
        let read = |window: &Value| {
            let minutes = window.get("window_minutes").and_then(Value::as_i64).unwrap_or(0);
            let reset = window
                .get("resets_at")
                .and_then(Value::as_i64)
                .map(|at| at * 1000)
                .or_else(|| window.get("resets_in_seconds").and_then(Value::as_i64).map(|after| at + after * 1000));
            let (used_percent, resets_at_ms) = current(window.get("used_percent")?.as_f64()?, reset, now_ms);
            Some(UsageLimit { name: window_name(minutes * 60), used_percent, resets_at_ms })
        };
        let found: Vec<UsageLimit> = ["primary", "secondary"]
            .iter()
            .filter_map(|key| limits.get(key).filter(|window| window.is_object()).and_then(read))
            .collect();
        (!found.is_empty()).then(|| PlanUsage { plan: limits.get("plan_type").and_then(Value::as_str).map(title), limits: found })
    })
}

/// A GitHub token for Copilot's usage, without running anything: the
/// variables Copilot reads, then the sign-in GitHub's Copilot editors keep.
/// `env` reads a variable; `config_dirs` are where those editors keep theirs.
pub fn github_token(env: &dyn Fn(&str) -> Option<String>, config_dirs: &[PathBuf]) -> Option<String> {
    ["COPILOT_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"]
        .iter()
        .find_map(|name| env(name).filter(|token| !token.trim().is_empty()))
        .or_else(|| {
            config_dirs.iter().find_map(|dir| {
                ["apps.json", "hosts.json"].iter().find_map(|file| {
                    let signed_in = read_json(&dir.join("github-copilot").join(file))?;
                    signed_in.as_object()?.iter().find_map(|(host, entry)| {
                        host.starts_with("github.com")
                            .then(|| entry.get("oauth_token")?.as_str().filter(|token| !token.is_empty()).map(str::to_owned))
                            .flatten()
                    })
                })
            })
        })
}

const COPILOT_PLANS: [(&str, &str); 5] =
    [("free", "Free"), ("individual", "Pro"), ("individual_pro", "Pro+"), ("business", "Business"), ("enterprise", "Enterprise")];

const COPILOT_QUOTAS: [(&str, &str); 3] =
    [("premium_interactions", "Premium requests"), ("chat", "Chat messages"), ("completions", "Code completions")];

fn copilot_usage(value: &Value) -> Option<PlanUsage> {
    let plan = value
        .get("copilot_plan")
        .and_then(Value::as_str)
        .map(|plan| COPILOT_PLANS.iter().find(|(id, _)| *id == plan).map_or_else(|| title(plan), |(_, name)| (*name).to_owned()));
    let reset_of = |keys: &[&str]| keys.iter().find_map(|key| value.get(key).and_then(Value::as_str).and_then(parse_time));
    if let Some(snapshots) = value.get("quota_snapshots").filter(|snapshots| snapshots.is_object()) {
        let resets_at_ms = reset_of(&["quota_reset_date_utc", "quota_reset_date"]);
        let limits = COPILOT_QUOTAS
            .iter()
            .filter_map(|(key, name)| {
                let quota = snapshots.get(key)?;
                if quota.get("unlimited").and_then(Value::as_bool).unwrap_or(false) {
                    return None;
                }
                let remaining = quota.get("percent_remaining")?.as_f64()?;
                Some(UsageLimit { name: (*name).to_owned(), used_percent: (100.0 - remaining).clamp(0.0, 100.0), resets_at_ms })
            })
            .collect();
        return Some(PlanUsage { plan, limits });
    }
    // The free plan counts what's left of each monthly quota.
    let left = value.get("limited_user_quotas")?;
    let monthly = value.get("monthly_quotas")?;
    let resets_at_ms = reset_of(&["limited_user_reset_date"]);
    let limits = COPILOT_QUOTAS
        .iter()
        .filter_map(|(key, name)| {
            let total = monthly.get(key)?.as_f64().filter(|total| *total > 0.0)?;
            let remaining = left.get(key)?.as_f64()?;
            Some(UsageLimit {
                name: (*name).to_owned(),
                used_percent: ((total - remaining) * 100.0 / total).clamp(0.0, 100.0),
                resets_at_ms,
            })
        })
        .collect();
    Some(PlanUsage { plan, limits })
}

/// How far along `agy`'s `statusLine` payload is during startup: quota arrives
/// first, then `plan_tier` shortly after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AntigravitySnapshot {
    Pending,
    QuotaOnly,
    Complete,
}

/// Checks whether a `statusLine` JSON payload from `agy` already carries quota
/// and plan tier.
pub fn antigravity_snapshot(body: &str) -> AntigravitySnapshot {
    let Ok(value) = serde_json::from_str::<Value>(body.trim_start_matches('\u{feff}')) else {
        return AntigravitySnapshot::Pending;
    };
    let has_quota = value.get("quota").and_then(Value::as_object).is_some_and(|quota| !quota.is_empty());
    if !has_quota {
        return AntigravitySnapshot::Pending;
    }
    let has_plan = value.get("plan_tier").and_then(Value::as_str).is_some_and(|tier| !tier.trim().is_empty());
    if has_plan { AntigravitySnapshot::Complete } else { AntigravitySnapshot::QuotaOnly }
}

/// Whether Antigravity is configured to use a Gemini API key rather than a
/// Google account plan.
pub fn antigravity_api_key(home: &Path) -> bool {
    read_json(&home.join("antigravity-cli/settings.json"))
        .and_then(|settings| settings.get("modelProvider")?.as_str().map(|provider| provider == "gemini"))
        .unwrap_or(false)
}

/// Shell command for `agy`'s `statusLine` setting that atomically saves the
/// JSON payload piped on stdin into `output`.
pub fn antigravity_statusline_command(output: &Path) -> String {
    let path = output.to_string_lossy();
    #[cfg(unix)]
    {
        let prefix = output.with_extension("").to_string_lossy().into_owned();
        format!("tmp={}.$$.tmp; cat > \"$tmp\" && mv \"$tmp\" {}", crate::shell_quote(&prefix), crate::shell_quote(&path))
    }
    #[cfg(windows)]
    {
        let prefix = output.with_extension("").to_string_lossy().into_owned();
        format!(
            "powershell -NoProfile -NonInteractive -Command \"$t = {} + '.' + $PID + '.tmp'; [Console]::In.ReadToEnd() | Set-Content -LiteralPath $t -Encoding utf8; Move-Item -Force -LiteralPath $t -Destination {}\"",
            crate::powershell_quote(&prefix),
            crate::powershell_quote(&path),
        )
    }
}

/// `--settings` JSON instructing `agy` to emit its `statusLine` payload to
/// `output` for this session only (never saved to `settings.json`).
pub fn antigravity_settings(output: &Path) -> String {
    serde_json::json!({
        "statusLine": {
            "type": "command",
            "command": antigravity_statusline_command(output),
            "enabled": true,
        }
    })
    .to_string()
}

const ANTIGRAVITY_BUCKETS: [(&str, &str); 4] = [
    ("gemini-5h", "Gemini · 5-hour"),
    ("gemini-weekly", "Gemini · Weekly"),
    ("3p-5h", "Claude & GPT · 5-hour"),
    ("3p-weekly", "Claude & GPT · Weekly"),
];

fn antigravity_bucket_name(key: &str) -> String {
    if let Some((_, name)) = ANTIGRAVITY_BUCKETS.iter().find(|(id, _)| *id == key) {
        return (*name).to_owned();
    }
    if let Some(group) = key.strip_suffix("-5h") {
        return format!("{} · 5-hour", title(group));
    }
    if let Some(group) = key.strip_suffix("-weekly") {
        return format!("{} · Weekly", title(group));
    }
    title(key)
}

fn antigravity_usage(value: &Value, now_ms: i64) -> Option<PlanUsage> {
    let quota = value.get("quota")?.as_object()?;
    if quota.is_empty() {
        return None;
    }
    let read_bucket = |key: &str, bucket: &Value| {
        let remaining = bucket.get("remaining_fraction")?.as_f64()?;
        let used = ((1.0 - remaining) * 100.0).clamp(0.0, 100.0);
        // Unused buckets report the Unix epoch (`1970-01-01T00:00:00Z`) and 0 seconds.
        let reset = bucket.get("reset_time").and_then(Value::as_str).and_then(parse_time).filter(|at| *at > 0).or_else(|| {
            bucket.get("reset_in_seconds").and_then(Value::as_i64).filter(|secs| *secs > 0).map(|secs| now_ms + secs * 1000)
        });
        let (used_percent, resets_at_ms) = current(used, reset, now_ms);
        Some(UsageLimit { name: antigravity_bucket_name(key), used_percent, resets_at_ms })
    };
    let mut limits = Vec::new();
    for (key, _) in ANTIGRAVITY_BUCKETS {
        if let Some(bucket) = quota.get(key)
            && let Some(limit) = read_bucket(key, bucket)
        {
            limits.push(limit);
        }
    }
    let mut extra: Vec<&String> = quota.keys().filter(|key| !ANTIGRAVITY_BUCKETS.iter().any(|(known, _)| known == key)).collect();
    extra.sort();
    for key in extra {
        if let Some(limit) = read_bucket(key, &quota[key]) {
            limits.push(limit);
        }
    }
    let plan = value.get("plan_tier").and_then(Value::as_str).map(str::trim).filter(|tier| !tier.is_empty()).map(str::to_owned);
    (!limits.is_empty()).then_some(PlanUsage { plan, limits })
}

fn title(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// An RFC 3339 time with any offset, or a bare date (midnight UTC).
fn parse_time(text: &str) -> Option<i64> {
    if text.len() == 10 {
        return crate::time::parse_timestamp(&format!("{text}T00:00:00Z"));
    }
    if let Some(utc) = text.strip_suffix('Z') {
        return crate::time::parse_timestamp(&format!("{utc}Z"));
    }
    // `+hh:mm` or `-hh:mm` after the time of day.
    let split = text.len().checked_sub(6)?;
    let (local, offset) = (text.get(..split)?, text.get(split..)?);
    let sign = match offset.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let (hours, minutes) = offset[1..].split_once(':')?;
    let offset_ms = (hours.parse::<i64>().ok()? * 60 + minutes.parse::<i64>().ok()?) * 60_000;
    Some(crate::time::parse_timestamp(&format!("{local}Z"))? - sign * offset_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_763_380_800_000; // 2025-11-17T12:00:00Z

    fn home_with(file: &str, contents: &str) -> tempfile::TempDir {
        let home = tempfile::tempdir().expect("tempdir");
        fs::write(home.path().join(file), contents).expect("write");
        home
    }

    #[test]
    fn times_with_offsets_and_bare_dates_read_as_utc() {
        assert_eq!(parse_time("2025-11-17T12:00:00Z"), Some(NOW));
        assert_eq!(parse_time("2025-11-17T14:00:00.123456+02:00"), Some(NOW + 123));
        assert_eq!(parse_time("2025-11-17T07:00:00-05:00"), Some(NOW));
        assert_eq!(parse_time("2025-11-17"), Some(NOW - 12 * 3_600_000));
        assert_eq!(parse_time("soon"), None);
    }

    #[test]
    fn claude_asks_with_its_oauth_sign_in_and_names_its_limits() {
        let home = home_with(
            ".credentials.json",
            r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat","expiresAt":1863380800000,"subscriptionType":"max"}}"#,
        );
        let ask = Provider::Claude.ask(home.path(), None, NOW).expect("ask");
        assert!(ask.headers.contains(&("Authorization".to_owned(), "Bearer sk-ant-oat".to_owned())));
        assert!(!format!("{ask:?}").contains("sk-ant-oat"), "the token stays out of logs");
        let body = r#"{"five_hour":{"utilization":32.0,"resets_at":"2025-11-17T14:00:00.000+00:00"},
            "seven_day":{"utilization":6,"resets_at":"2025-11-18T07:00:00Z"},
            "seven_day_opus":null,"seven_day_oauth_apps":null,"iguana_necktie":null}"#;
        let usage = Provider::Claude.read(home.path(), body, NOW).expect("read");
        assert_eq!(usage.plan.as_deref(), Some("Max"));
        assert_eq!(
            usage.limits,
            vec![
                UsageLimit { name: "5-hour limit".to_owned(), used_percent: 32.0, resets_at_ms: Some(NOW + 2 * 3_600_000) },
                UsageLimit {
                    name: "Weekly · all models".to_owned(),
                    used_percent: 6.0,
                    resets_at_ms: Some(NOW + 19 * 3_600_000)
                },
            ]
        );
    }

    #[test]
    fn claude_s_listed_limits_name_a_weekly_limit_s_model() {
        let home = home_with(".credentials.json", r#"{"claudeAiOauth":{"accessToken":"t","subscriptionType":"max"}}"#);
        // As Anthropic answered on 2026-11-17, trimmed.
        let body = r#"{"five_hour":{"utilization":43.0,"resets_at":"2026-11-17T12:59:59.827986+00:00"},
            "seven_day":{"utilization":7.0,"resets_at":"2026-11-24T05:59:59.828004+00:00"},"seven_day_opus":null,"tangelo":null,
            "limits":[
              {"kind":"session","group":"session","percent":43,"severity":"normal","resets_at":"2026-11-17T12:59:59.827986+00:00","scope":null,"is_active":true},
              {"kind":"weekly_all","group":"weekly","percent":7,"severity":"normal","resets_at":"2026-11-24T05:59:59.828004+00:00","scope":null,"is_active":false},
              {"kind":"weekly_scoped","group":"weekly","percent":0,"severity":"normal","resets_at":"2026-11-24T06:00:00+00:00","scope":{"model":{"id":null,"display_name":"Fable"},"surface":null},"is_active":false},
              {"kind":"something_new","percent":5}]}"#;
        let usage = Provider::Claude.read(home.path(), body, NOW).expect("read");
        assert_eq!(
            usage.limits.iter().map(|limit| (limit.name.as_str(), limit.used_percent)).collect::<Vec<_>>(),
            [("5-hour limit", 43.0), ("Weekly · all models", 7.0), ("Weekly · Fable", 0.0)]
        );
        assert_eq!(usage.limits[2].resets_at_ms, parse_time("2026-11-24T06:00:00Z"));
    }

    #[test]
    fn claude_says_why_it_cant_ask() {
        let expired = home_with(".credentials.json", r#"{"claudeAiOauth":{"accessToken":"t","expiresAt":1}}"#);
        assert_eq!(Provider::Claude.ask(expired.path(), None, NOW), Err(UsageError::Expired("Claude Code")));
        let none = tempfile::tempdir().expect("tempdir");
        assert!(matches!(Provider::Claude.ask(none.path(), None, NOW), Err(UsageError::SignedOut(_) | UsageError::ApiKey(_))));
        assert_eq!(Provider::Claude.read(none.path(), "[]", NOW), Err(UsageError::Unreadable("Claude Code")));
    }

    #[test]
    fn codex_asks_for_its_account_and_reads_both_windows() {
        let home = home_with("auth.json", r#"{"OPENAI_API_KEY":null,"tokens":{"access_token":"at","account_id":"acct"}}"#);
        let ask = Provider::Codex.ask(home.path(), None, NOW).expect("ask");
        assert!(ask.headers.contains(&("ChatGPT-Account-Id".to_owned(), "acct".to_owned())));
        let body = r#"{"plan_type":"plus","rate_limit":{"allowed":true,"limit_reached":false,
            "primary_window":{"used_percent":12,"limit_window_seconds":18000,"reset_after_seconds":3600,"reset_at":1763384400},
            "secondary_window":{"used_percent":40.5,"limit_window_seconds":604800,"reset_at":1763337600}}}"#;
        let usage = Provider::Codex.read(home.path(), body, NOW).expect("read");
        assert_eq!(usage.plan.as_deref(), Some("Plus"));
        assert_eq!(
            usage.limits,
            vec![
                UsageLimit { name: "5-hour limit".to_owned(), used_percent: 12.0, resets_at_ms: Some(NOW + 3_600_000) },
                // Its window ended before now.
                UsageLimit { name: "Weekly limit".to_owned(), used_percent: 0.0, resets_at_ms: None },
            ]
        );
        let keyed = home_with("auth.json", r#"{"OPENAI_API_KEY":"sk-key"}"#);
        assert_eq!(Provider::Codex.ask(keyed.path(), None, NOW), Err(UsageError::ApiKey("Codex")));
    }

    #[test]
    fn codex_s_recorded_limits_are_the_last_a_session_wrote() {
        let rollout = [
            r#"{"timestamp":"2025-11-17T11:00:00Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":1.0,"window_minutes":300,"resets_in_seconds":60}}}}"#,
            r#"{"timestamp":"2025-11-17T11:30:00Z","type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"primary":{"used_percent":20.0,"window_minutes":300,"resets_in_seconds":7200},"secondary":{"used_percent":3.0,"window_minutes":10080,"resets_at":1763900000},"plan_type":"pro"}}}"#,
            r#"{"timestamp":"2025-11-17T11:31:00Z","type":"response_item","payload":{"type":"message"}}"#,
        ]
        .join("\n");
        let usage = codex_recorded_in(&rollout, NOW).expect("recorded");
        assert_eq!(usage.plan.as_deref(), Some("Pro"));
        assert_eq!(
            usage.limits,
            vec![
                UsageLimit { name: "5-hour limit".to_owned(), used_percent: 20.0, resets_at_ms: Some(NOW + 90 * 60_000) },
                UsageLimit { name: "Weekly limit".to_owned(), used_percent: 3.0, resets_at_ms: Some(1_763_900_000_000) },
            ]
        );
        let home = tempfile::tempdir().expect("tempdir");
        let day = home.path().join("sessions/2025/11/17");
        fs::create_dir_all(&day).expect("dirs");
        fs::write(day.join("rollout-2025-11-17T11-00-00-0199a213-81c0-7800-8aa1-bbab2a035a53.jsonl"), &rollout).expect("write");
        assert_eq!(Provider::Codex.recorded(home.path(), NOW), Some(usage));
    }

    #[test]
    fn copilot_counts_the_premium_requests_used_and_leaves_out_unlimited_quotas() {
        let body = r#"{"copilot_plan":"individual","quota_reset_date":"2025-12-01","quota_reset_date_utc":"2025-12-01T00:00:00.000Z",
            "quota_snapshots":{"chat":{"unlimited":true,"percent_remaining":100},"completions":{"unlimited":true},
            "premium_interactions":{"entitlement":300,"remaining":235,"percent_remaining":78.5,"unlimited":false}}}"#;
        let usage = copilot_usage(&serde_json::from_str(body).expect("json")).expect("usage");
        assert_eq!(usage.plan.as_deref(), Some("Pro"));
        assert_eq!(
            usage.limits,
            vec![UsageLimit { name: "Premium requests".to_owned(), used_percent: 21.5, resets_at_ms: parse_time("2025-12-01") }]
        );
        let free = r#"{"copilot_plan":"free","limited_user_reset_date":"2025-12-05",
            "limited_user_quotas":{"chat":40,"completions":2000},"monthly_quotas":{"chat":50,"completions":2000}}"#;
        let usage = copilot_usage(&serde_json::from_str(free).expect("json")).expect("usage");
        assert_eq!(usage.plan.as_deref(), Some("Free"));
        assert_eq!(
            usage.limits.iter().map(|limit| (limit.name.as_str(), limit.used_percent)).collect::<Vec<_>>(),
            [("Chat messages", 20.0), ("Code completions", 0.0)]
        );
    }

    #[test]
    fn a_github_token_comes_from_the_variables_then_the_editors_sign_in() {
        let config = tempfile::tempdir().expect("tempdir");
        fs::create_dir(config.path().join("github-copilot")).expect("dir");
        fs::write(
            config.path().join("github-copilot/apps.json"),
            r#"{"github.com:Iv1.b507a08c87ecfe98":{"user":"u","oauth_token":"gho_apps"}}"#,
        )
        .expect("write");
        let dirs = [PathBuf::from("/nowhere"), config.path().to_owned()];
        assert_eq!(github_token(&|name| (name == "GH_TOKEN").then(|| "gho_env".to_owned()), &dirs).as_deref(), Some("gho_env"));
        assert_eq!(github_token(&|_| None, &dirs).as_deref(), Some("gho_apps"));
        assert_eq!(github_token(&|_| None, &[]), None);
    }

    #[test]
    fn antigravity_reads_statusline_quota_buckets_and_plan_tier() {
        // As `agy` 1.5.18 pipes to its `statusLine` command.
        let body = r#"{"product":"antigravity","plan_tier":"Google AI Pro","quota":{
            "3p-5h":{"remaining_fraction":1,"reset_time":"1970-01-01T00:00:00Z","reset_in_seconds":0},
            "3p-weekly":{"remaining_fraction":0.75,"reset_time":"2025-11-24T12:00:00Z","reset_in_seconds":604800},
            "gemini-5h":{"remaining_fraction":0.9,"reset_time":"2025-11-17T16:00:00Z","reset_in_seconds":14400},
            "gemini-weekly":{"remaining_fraction":0.8,"reset_time":"2025-11-17T10:00:00Z","reset_in_seconds":0},
            "flash-5h":{"remaining_fraction":0.5,"reset_time":"1970-01-01T00:00:00Z","reset_in_seconds":1800}
        }}"#;
        assert_eq!(antigravity_snapshot(body), AntigravitySnapshot::Complete);
        assert_eq!(antigravity_snapshot(r#"{"quota":{"gemini-5h":{"remaining_fraction":1}}}"#), AntigravitySnapshot::QuotaOnly);
        assert_eq!(antigravity_snapshot(r#"{"agent_state":"authenticating"}"#), AntigravitySnapshot::Pending);

        let home = tempfile::tempdir().expect("tempdir");
        let usage = Provider::Antigravity.read(home.path(), body, NOW).expect("read");
        assert_eq!(usage.plan.as_deref(), Some("Google AI Pro"));
        assert_eq!(
            usage
                .limits
                .iter()
                .map(|limit| (limit.name.as_str(), limit.used_percent.round(), limit.resets_at_ms))
                .collect::<Vec<_>>(),
            [
                ("Gemini · 5-hour", 10.0, Some(NOW + 4 * 3_600_000)),
                // Its reset time passed before NOW.
                ("Gemini · Weekly", 0.0, None),
                // Unused bucket reports epoch zero, so no reset time is shown.
                ("Claude & GPT · 5-hour", 0.0, None),
                ("Claude & GPT · Weekly", 25.0, parse_time("2025-11-24T12:00:00Z")),
                // Unknown future bucket falls back cleanly and uses reset_in_seconds when reset_time is epoch zero.
                ("Flash · 5-hour", 50.0, Some(NOW + 1_800_000)),
            ]
        );
    }

    #[test]
    fn antigravity_recognises_gemini_api_key_mode() {
        let home = tempfile::tempdir().expect("tempdir");
        assert!(!antigravity_api_key(home.path()));
        fs::create_dir_all(home.path().join("antigravity-cli")).expect("dir");
        fs::write(home.path().join("antigravity-cli/settings.json"), r#"{"modelProvider":"gemini"}"#).expect("write");
        assert!(antigravity_api_key(home.path()));
        assert_eq!(Provider::Antigravity.ask(home.path(), None, NOW), Err(UsageError::ApiKey("Antigravity")));
    }
}
