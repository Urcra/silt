//! What a machine connects to on its own, by Settings › Network and
//! `SILT_OFFLINE=1` (ADR 0132): offline, it asks no provider for plan usage,
//! fetches no artifact library it hasn't kept and leaves the agent registry
//! as built in.

mod common;

use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::routing::get;
use common::{Machine, origin};
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

#[test]
fn an_offline_machine_connects_nowhere_on_its_own() {
    let root = common::temp_root("silt-offline");
    let origin = origin(root.path());
    let laptop = Machine::without_server(root.path(), "laptop", &origin);
    let asked = Arc::new(AtomicUsize::new(0));
    let counted = asked.clone();
    let provider = serve(Router::new().route(
        "/claude",
        get(move || async move {
            counted.fetch_add(1, Ordering::SeqCst);
            axum::Json(json!({"five_hour": {"utilization": 32.0, "resets_at": "2099-01-01T14:00:00Z"}}))
        }),
    ));
    fs::write(
        laptop.claude_dir.join(".credentials.json"),
        json!({"claudeAiOauth": {"accessToken": "claude-token", "expiresAt": 4_102_444_800_000_i64, "subscriptionType": "max"}})
            .to_string(),
    )
    .expect("credentials");
    laptop.run(&["daemon", "stop"]);
    laptop
        .silt_with(&["open", "--no-browser"], &[("SILT_OFFLINE", "1"), ("SILT_CLAUDE_USAGE_URL", &format!("{provider}/claude"))]);

    let usage = laptop.web("GET", "/api/agents/claude-code/usage", None);
    assert!(usage["unavailable"].as_str().is_some_and(|why| why.contains("offline")), "{usage}");
    assert_eq!(asked.load(Ordering::SeqCst), 0, "the provider was asked");

    let (status, refused) = laptop.web_answer("GET", "/vendor/cdnjs.cloudflare.com/ajax/libs/d3/7.9.0/d3.min.js", None);
    assert_ne!(status, "200");
    assert!(refused.contains("offline"), "{refused}");
    assert!(!laptop.silt_home.join("vendor").exists(), "nothing was kept, as nothing was fetched");

    let agents = laptop.web("GET", "/api/agents", None);
    assert!(agents.to_string().contains("claude"), "the built-in registry answers: {agents}");
    assert!(!laptop.silt_home.join("acp-registry.json").exists(), "the registry was fetched");

    // Artifacts load their libraries through Silt unless the person turns it off.
    assert_eq!(laptop.web("GET", "/api/artifact-settings", None), json!({"proxy": true}));
}

/// Settings › Network turns each connection off, and Offline all of them,
/// without the daemon restarting; a damaged file counts as offline.
#[test]
fn settings_turn_connections_off_while_the_daemon_runs() {
    let root = common::temp_root("silt-network");
    let origin = origin(root.path());
    let laptop = Machine::without_server(root.path(), "laptop", &origin);
    let asked = Arc::new(AtomicUsize::new(0));
    let counted = asked.clone();
    let provider = serve(Router::new().route(
        "/claude",
        get(move || async move {
            counted.fetch_add(1, Ordering::SeqCst);
            axum::Json(json!({"five_hour": {"utilization": 32.0, "resets_at": "2099-01-01T14:00:00Z"}}))
        }),
    ));
    fs::write(
        laptop.claude_dir.join(".credentials.json"),
        json!({"claudeAiOauth": {"accessToken": "claude-token", "expiresAt": 4_102_444_800_000_i64, "subscriptionType": "max"}})
            .to_string(),
    )
    .expect("credentials");
    laptop.run(&["daemon", "stop"]);
    laptop.silt_with(&["open", "--no-browser"], &[("SILT_CLAUDE_USAGE_URL", &format!("{provider}/claude"))]);

    let view = laptop.web("GET", "/api/network-settings", None);
    assert_eq!(view["settings"], json!({"offline": false, "update_checks": true, "plan_usage": true, "agent_registry": true}));
    assert_eq!((view["offline"].as_bool(), view["offline_env"].as_bool()), (Some(false), Some(false)));

    laptop.web(
        "PUT",
        "/api/network-settings",
        Some(json!({"offline": false, "update_checks": true, "plan_usage": false, "agent_registry": true})),
    );
    let usage = laptop.web("GET", "/api/agents/claude-code/usage", None);
    assert!(usage["unavailable"].as_str().is_some_and(|why| why.contains("Settings › Network")), "{usage}");
    assert_eq!(asked.load(Ordering::SeqCst), 0, "the provider was asked with plan usage off");

    laptop.web(
        "PUT",
        "/api/network-settings",
        Some(json!({"offline": true, "update_checks": true, "plan_usage": true, "agent_registry": true})),
    );
    assert_eq!(laptop.web("GET", "/api/network-settings", None)["offline"], true);
    let (status, refused) = laptop.web_answer("GET", "/vendor/cdnjs.cloudflare.com/ajax/libs/d3/7.9.0/d3.min.js", None);
    assert_ne!(status, "200");
    assert!(refused.contains("offline"), "{refused}");
    let usage = laptop.web("GET", "/api/agents/claude-code/usage", None);
    assert!(usage["unavailable"].as_str().is_some_and(|why| why.contains("offline")), "{usage}");
    assert_eq!(asked.load(Ordering::SeqCst), 0, "the provider was asked offline");

    fs::write(laptop.silt_home.join("network.json"), "{\"offline\": fal").expect("damage");
    let (status, why) = laptop.web_answer("GET", "/api/network-settings", None);
    assert_ne!(status, "200");
    assert!(why.contains("network.json"), "{why}");
    let (status, refused) = laptop.web_answer("GET", "/vendor/cdnjs.cloudflare.com/ajax/libs/d3/7.9.0/d3.min.js", None);
    assert!(status != "200" && refused.contains("offline"), "a damaged file counts as offline: {status} {refused}");

    laptop.web(
        "PUT",
        "/api/network-settings",
        Some(json!({"offline": false, "update_checks": true, "plan_usage": true, "agent_registry": true})),
    );
    laptop.web("GET", "/api/agents/claude-code/usage", None);
    assert_eq!(asked.load(Ordering::SeqCst), 1, "saving fixes the file and plan usage asks again");
}
