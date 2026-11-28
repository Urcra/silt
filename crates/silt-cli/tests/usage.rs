//! Each agent's plan usage through a real daemon, with stand-ins for
//! Anthropic's, OpenAI's and GitHub's answers (ADR 0095).

mod common;

use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use common::{Machine, Server, origin};
use serde_json::json;

/// Serves `app` on loopback from a thread of its own, for the test's life.
fn serve(app: Router) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!("http://{}", listener.local_addr().expect("address"));
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).expect("listener");
            axum::serve(listener, app).await.expect("serves");
        });
    });
    url
}

fn signed(headers: &HeaderMap, expected: &str) -> bool {
    headers.get("authorization").and_then(|value| value.to_str().ok()) == Some(expected)
}

fn now_ms() -> i64 {
    i64::try_from(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("after the epoch").as_millis())
        .expect("ms")
}

#[test]
fn each_agent_s_plan_usage_is_read_with_its_own_sign_in() {
    let root = common::temp_root("silt-usage");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    let claude_asked = Arc::new(AtomicUsize::new(0));
    let counted = claude_asked.clone();
    let provider = serve(
        Router::new()
            .route(
                "/claude",
                get(move |headers: HeaderMap| async move {
                    counted.fetch_add(1, Ordering::SeqCst);
                    if !signed(&headers, "Bearer claude-token") || headers.get("anthropic-beta").is_none() {
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                    axum::Json(json!({
                        "five_hour": {"utilization": 32.0, "resets_at": "2099-01-01T14:00:00.000+00:00"},
                        "seven_day": {"utilization": 6.0, "resets_at": "2099-01-06T07:00:00Z"},
                        "seven_day_opus": null,
                    }))
                    .into_response()
                }),
            )
            .route("/codex", get(|| async { StatusCode::UNAUTHORIZED }))
            .route(
                "/copilot",
                get(|headers: HeaderMap| async move {
                    if !signed(&headers, "token gho_copilot") {
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                    axum::Json(json!({
                        "copilot_plan": "individual_pro",
                        "quota_reset_date": "2099-02-01",
                        "quota_snapshots": {"premium_interactions": {"unlimited": false, "percent_remaining": 75.0}, "chat": {"unlimited": true}},
                    }))
                    .into_response()
                }),
            ),
    );
    fs::write(
        laptop.claude_dir.join(".credentials.json"),
        json!({"claudeAiOauth": {"accessToken": "claude-token", "expiresAt": 4_102_444_800_000_i64, "subscriptionType": "max"}})
            .to_string(),
    )
    .expect("credentials");
    // Codex's sign-in is refused, so what its last session recorded stands in.
    let codex = laptop.agents.join("codex");
    fs::write(codex.join("auth.json"), json!({"tokens": {"access_token": "stale", "account_id": "acct"}}).to_string())
        .expect("auth");
    let now = now_ms();
    let today = silt_agents::time::format_timestamp(now);
    let day = codex.join("sessions").join(&today[..4]).join(&today[5..7]).join(&today[8..10]);
    fs::create_dir_all(&day).expect("sessions");
    let event = json!({
        "timestamp": today,
        "type": "event_msg",
        "payload": {"type": "token_count", "rate_limits": {
            "primary": {"used_percent": 20.0, "window_minutes": 300, "resets_at": now / 1000 + 3600},
            "secondary": {"used_percent": 4.0, "window_minutes": 10_080, "resets_at": now / 1000 + 86_400},
            "plan_type": "pro",
        }},
    });
    fs::write(day.join("rollout-2025-01-01T00-00-00-0199a213-81c0-7800-8aa1-bbab2a035a53.jsonl"), format!("{event}\n"))
        .expect("rollout");
    // The daemon starts with the stand-ins and GitHub's token in its environment.
    let fake_agy = common::fake_agent("agy").to_string_lossy().into_owned();
    let agy_count = root.path().join("agy-usage-count");
    let agy_count_str = agy_count.to_string_lossy().into_owned();
    let agy_payload = json!({
        "product": "antigravity",
        "plan_tier": "Google AI Pro",
        "quota": {
            "gemini-5h": {"remaining_fraction": 0.9, "reset_time": "2099-01-01T14:00:00Z", "reset_in_seconds": 3600},
            "gemini-weekly": {"remaining_fraction": 0.8, "reset_time": "2099-01-06T07:00:00Z", "reset_in_seconds": 86400},
            "3p-5h": {"remaining_fraction": 1.0, "reset_time": "1970-01-01T00:00:00Z", "reset_in_seconds": 0},
        },
    })
    .to_string();
    laptop.run(&["daemon", "stop"]);
    laptop.silt_with(
        &["open", "--no-browser"],
        &[
            ("SILT_CLAUDE_USAGE_URL", &format!("{provider}/claude")),
            ("SILT_CODEX_USAGE_URL", &format!("{provider}/codex")),
            ("SILT_COPILOT_USAGE_URL", &format!("{provider}/copilot")),
            ("COPILOT_GITHUB_TOKEN", "gho_copilot"),
            ("SILT_ANTIGRAVITY_USAGE_COMMAND", &fake_agy),
            ("FAKE_AGY_USAGE", &agy_payload),
            ("FAKE_AGY_USAGE_COUNT", &agy_count_str),
        ],
    );

    let claude = laptop.web("GET", "/api/agents/claude-acp/usage", None);
    assert_eq!(claude["agent"], "Claude Code");
    assert_eq!(claude["plan"], "Max");
    assert_eq!(claude["page"], "https://claude.ai/settings/usage");
    let limits: Vec<(&str, f64)> = claude["limits"]
        .as_array()
        .expect("limits")
        .iter()
        .map(|limit| (limit["name"].as_str().expect("name"), limit["used_percent"].as_f64().expect("percent")))
        .collect();
    assert_eq!(limits, [("5-hour limit", 32.0), ("Weekly · all models", 6.0)]);
    assert_eq!(claude["limits"][0]["resets_at_ms"], 4_070_959_200_000_i64);
    // A minute's answers come from the daemon, not Anthropic again.
    assert_eq!(laptop.web("GET", "/api/agents/claude-code/usage", None), claude);
    assert_eq!(claude_asked.load(Ordering::SeqCst), 1);

    let codex = laptop.web("GET", "/api/agents/codex-acp/usage", None);
    assert_eq!(codex["recorded"], true, "{codex}");
    assert_eq!(codex["plan"], "Pro");
    assert_eq!(codex["limits"][0]["name"], "5-hour limit");
    assert_eq!(codex["limits"][0]["used_percent"], 20.0);
    assert_eq!(codex["limits"][1]["name"], "Weekly limit");

    let copilot = laptop.web("GET", "/api/agents/copilot-cli/usage", None);
    assert_eq!(copilot["plan"], "Pro+");
    assert_eq!(
        copilot["limits"],
        json!([{"name": "Premium requests", "used_percent": 25.0, "resets_at_ms": 4_073_587_200_000_i64}])
    );

    let agy = laptop.web("GET", "/api/agents/antigravity-acp/usage", None);
    assert_eq!(agy["agent"], "Antigravity");
    assert_eq!(agy["plan"], "Google AI Pro");
    assert_eq!(agy["page"], "https://antigravity.google/docs/plans");
    let agy_limits: Vec<(&str, f64, bool)> = agy["limits"]
        .as_array()
        .expect("limits")
        .iter()
        .map(|limit| {
            (
                limit["name"].as_str().expect("name"),
                limit["used_percent"].as_f64().expect("percent").round(),
                limit["resets_at_ms"].is_i64(),
            )
        })
        .collect();
    assert_eq!(
        agy_limits,
        [("Gemini · 5-hour", 10.0, true), ("Gemini · Weekly", 20.0, true), ("Claude & GPT · 5-hour", 0.0, false)]
    );
    // Subsequent asks come from the daemon cache rather than spawning agy again.
    assert_eq!(laptop.web("GET", "/api/agents/antigravity-cli/usage", None), agy);
    assert_eq!(fs::read_to_string(&agy_count).expect("count").lines().count(), 1);

    let (status, refused): (String, String) = laptop.web_answer("GET", "/api/agents/gemini-cli/usage", None);
    assert_ne!(status, "200");
    assert!(refused.contains("plan usage"), "{refused}");
}
