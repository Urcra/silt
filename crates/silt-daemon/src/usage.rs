//! How much of each agent's plan is used, asked of its provider with the
//! sign-in the agent keeps, and kept a minute so composers looking often
//! don't ask each time (ADR 0095).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use silt_agents::sign_in::{Login, LoginEvent};
use silt_agents::usage::{AntigravitySnapshot, PlanUsage, Provider, UsageError};

use crate::core::{CoreError, now_ms};
use crate::protocol::{AgentUsage, UsageLimit};

/// How long an answer stands before the provider is asked again.
const KEEP: Duration = Duration::from_secs(60);
/// Antigravity spawns `agy` to read its `statusLine`, so keep it longer.
const KEEP_ANTIGRAVITY: Duration = Duration::from_secs(180);
/// How long a provider may take to answer.
const WAIT: Duration = Duration::from_secs(10);
/// How long to wait for `plan_tier` after Antigravity's `quota` arrives.
const ANTIGRAVITY_PLAN_WAIT: Duration = Duration::from_secs(2);
const ANTIGRAVITY_POLL: Duration = Duration::from_millis(100);

pub(crate) struct Usage {
    client: reqwest::Client,
    /// The last answer for each agent and home, and when it came.
    kept: tokio::sync::Mutex<HashMap<(Provider, PathBuf), (Instant, AgentUsage)>>,
}

impl Usage {
    pub(crate) fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(WAIT)
            .user_agent(concat!("silt/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();
        Self { client, kept: tokio::sync::Mutex::default() }
    }

    /// `agent`'s usage, with the sign-in it would run with in `cwd`; only
    /// what the agent recorded when `off` says why its provider isn't asked.
    pub(crate) async fn of(&self, agent: &str, cwd: Option<&Path>, off: Option<&str>) -> Result<AgentUsage, CoreError> {
        let provider = Provider::for_agent(silt_workspace::agent_key(agent).as_str())
            .ok_or_else(|| CoreError::Invalid(format!("Silt doesn't know how to read {agent}'s plan usage")))?;
        let home = provider.home(cwd).ok_or_else(|| CoreError::Invalid("HOME is not set".to_owned()))?;
        if let Some(why) = off {
            let now = now_ms();
            return Ok(match provider.recorded(&home, now) {
                Some(recorded) => view(provider, recorded, None, true, now),
                None => {
                    let why = format!("Silt doesn't ask {}'s provider: {why}", provider.agent());
                    view(provider, PlanUsage { plan: None, limits: Vec::new() }, Some(why), false, now)
                }
            });
        }
        // Held across the ask, so composers opening together ask once.
        let mut kept = self.kept.lock().await;
        let key = (provider, home);
        let keep = match provider {
            Provider::Antigravity => KEEP_ANTIGRAVITY,
            Provider::Claude | Provider::Codex | Provider::Copilot => KEEP,
        };
        if let Some((at, usage)) = kept.get(&key)
            && at.elapsed() < keep
        {
            return Ok(usage.clone());
        }
        let last = kept.get(&key).map(|(_, usage)| usage.clone()).filter(|usage| usage.unavailable.is_none());
        let usage = self.ask(provider, &key.1, last).await;
        kept.insert(key, (Instant::now(), usage.clone()));
        Ok(usage)
    }

    async fn ask(&self, provider: Provider, home: &Path, last: Option<AgentUsage>) -> AgentUsage {
        let now = now_ms();
        let asked = match provider {
            Provider::Antigravity => self.ask_antigravity(home, now).await,
            Provider::Claude | Provider::Codex | Provider::Copilot => {
                let github = match provider {
                    Provider::Copilot => github_token().await,
                    Provider::Claude | Provider::Codex | Provider::Antigravity => None,
                };
                match provider.ask(home, github.as_deref(), now) {
                    Ok(ask) => self
                        .fetch(provider, ask)
                        .await
                        .and_then(|body| provider.read(home, &body, now).map_err(|error| error.to_string())),
                    Err(error) => Err(error.to_string()),
                }
            }
        };
        match asked {
            Ok(usage) => view(provider, usage, None, false, now),
            Err(why) => {
                if let Some(recorded) = provider.recorded(home, now) {
                    return view(provider, recorded, None, true, now);
                }
                // A refusal for asking too often, or a network gone a moment, leaves the last answer standing.
                match last {
                    Some(last) => AgentUsage { recorded: true, ..last },
                    None => view(provider, PlanUsage { plan: None, limits: Vec::new() }, Some(why), false, now),
                }
            }
        }
    }

    /// Reads Antigravity's quota from `agy`'s own `statusLine` setting in a
    /// scratch `--gemini_dir`, so Silt never reads Antigravity's OAuth tokens
    /// or writes into the person's `.gemini` folder (ADR 0095).
    async fn ask_antigravity(&self, home: &Path, now: i64) -> Result<PlanUsage, String> {
        let agent = Provider::Antigravity.agent();
        if silt_agents::usage::antigravity_api_key(home) {
            return Err(UsageError::ApiKey(agent).to_string());
        }
        let scratch = tempfile::tempdir().map_err(|_| format!("couldn't prepare a folder for {agent}"))?;
        let gemini = scratch.path().join("gemini");
        let app = gemini.join("antigravity-cli");
        let _ = std::fs::create_dir_all(&app);
        let real_settings = home.join("antigravity-cli/settings.json");
        if real_settings.is_file() {
            let _ = std::fs::copy(&real_settings, app.join("settings.json"));
        }
        let status_file = scratch.path().join("status.json");
        let settings = silt_agents::usage::antigravity_settings(&status_file);
        let program = std::env::var("SILT_ANTIGRAVITY_USAGE_COMMAND").unwrap_or_else(|_| "agy".to_owned());
        let args = vec!["--gemini_dir".to_owned(), gemini.to_string_lossy().into_owned(), "--settings".to_owned(), settings];
        let mut env: Vec<(String, String)> =
            std::env::vars().filter(|(name, _)| !name.eq_ignore_ascii_case("AGY_CLI_DISABLE_AUTO_UPDATE")).collect();
        env.push(("AGY_CLI_DISABLE_AUTO_UPDATE".to_owned(), "true".to_owned()));
        let exited = Arc::new(AtomicBool::new(false));
        let done = exited.clone();
        let mut login = Login::start(&program, &args, &env, scratch.path(), move |event| {
            if matches!(event, LoginEvent::Exited { .. }) {
                done.store(true, Ordering::SeqCst);
            }
        })
        .map_err(|_| format!("couldn't run {program} to check {agent}'s usage"))?;
        let deadline = Instant::now() + WAIT;
        let mut quota_since: Option<Instant> = None;
        let mut latest: Option<String> = None;
        loop {
            if let Ok(body) = tokio::fs::read_to_string(&status_file).await {
                match silt_agents::usage::antigravity_snapshot(&body) {
                    AntigravitySnapshot::Complete => {
                        latest = Some(body);
                        break;
                    }
                    AntigravitySnapshot::QuotaOnly => {
                        latest = Some(body);
                        quota_since.get_or_insert_with(Instant::now);
                    }
                    AntigravitySnapshot::Pending => {}
                }
            }
            if exited.load(Ordering::SeqCst) {
                if let Ok(body) = tokio::fs::read_to_string(&status_file).await
                    && silt_agents::usage::antigravity_snapshot(&body) != AntigravitySnapshot::Pending
                {
                    latest = Some(body);
                }
                break;
            }
            if Instant::now() >= deadline || quota_since.is_some_and(|at| at.elapsed() >= ANTIGRAVITY_PLAN_WAIT) {
                break;
            }
            tokio::time::sleep(ANTIGRAVITY_POLL).await;
        }
        login.stop();
        drop(login);
        let body = latest.ok_or_else(|| UsageError::SignedOut(agent).to_string())?;
        Provider::Antigravity.read(home, &body, now).map_err(|error| error.to_string())
    }

    async fn fetch(&self, provider: Provider, ask: silt_agents::usage::Ask) -> Result<String, String> {
        let mut request = self.client.get(&ask.url);
        for (name, value) in &ask.headers {
            request = request.header(name, value);
        }
        let agent = provider.agent();
        let response = request.send().await.map_err(|_| format!("couldn't reach {agent}'s provider"))?;
        match response.status() {
            status if status.is_success() => response.text().await.map_err(|_| UsageError::Unreadable(agent).to_string()),
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => Err(UsageError::Expired(agent).to_string()),
            reqwest::StatusCode::TOO_MANY_REQUESTS => {
                Err(format!("{agent}'s provider asks to be asked less often; try again shortly"))
            }
            status => Err(format!("{agent}'s provider answered {status}")),
        }
    }
}

fn view(provider: Provider, usage: PlanUsage, unavailable: Option<String>, recorded: bool, now: i64) -> AgentUsage {
    AgentUsage {
        agent: provider.agent().to_owned(),
        plan: usage.plan,
        limits: usage
            .limits
            .into_iter()
            .map(|limit| UsageLimit { name: limit.name, used_percent: limit.used_percent, resets_at_ms: limit.resets_at_ms })
            .collect(),
        page: provider.page().to_owned(),
        unavailable,
        recorded,
        read_ms: now,
    }
}

/// The GitHub sign-in Copilot's usage is asked with: the variables and
/// Copilot editors' sign-in, else GitHub CLI's.
async fn github_token() -> Option<String> {
    let mut config_dirs: Vec<PathBuf> = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| silt_os::home_dir().map(|home| home.join(".config")))
        .into_iter()
        .collect();
    config_dirs.extend(std::env::var_os("LOCALAPPDATA").map(PathBuf::from));
    if let Some(token) = silt_agents::usage::github_token(&|name| std::env::var(name).ok(), &config_dirs) {
        return Some(token);
    }
    let mut gh = tokio::process::Command::new("gh");
    gh.args(["auth", "token"]).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true);
    let output = tokio::time::timeout(WAIT, gh.output()).await.ok()?.ok()?;
    let token = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !token.is_empty()).then_some(token)
}
