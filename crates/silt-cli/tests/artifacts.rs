//! Artifacts (ADR 0068) between machines: what an agent showed on one
//! machine reaches the person's other machines with the thread.

mod common;

use std::fs;

use base64::Engine as _;
use common::{Machine, Server, eventually, origin};

/// Fields drop in order: the machines stop their daemons before the
/// directories holding their sockets go away.
struct World {
    laptop: Machine,
    home: Machine,
    server: Server,
    _root: tempfile::TempDir,
}

fn world() -> World {
    let root = common::temp_root("silt-artifacts");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    let home = Machine::new(root.path(), "home", &origin, &server);
    let fake = laptop.fake.with_file_name(format!("silt-fake-agent{}", std::env::consts::EXE_SUFFIX));
    let agents = serde_json::json!([{"id": "v1", "name": "Fake v1", "command": fake}]);
    fs::write(laptop.silt_home.join("agents.json"), agents.to_string()).expect("write agents.json");
    World { laptop, home, server, _root: root }
}

fn artifacts(machine: &Machine, thread: &str) -> Vec<serde_json::Value> {
    let (status, history) = machine.web_answer("GET", &format!("/api/threads/{thread}/history"), None);
    if status != "200" {
        return Vec::new();
    }
    let history: serde_json::Value = serde_json::from_str(&history).unwrap_or_default();
    history["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry["kind"] == "artifact")
        .map(|entry| entry["artifact"].clone())
        .collect()
}

#[test]
fn an_artifact_made_on_one_machine_opens_on_another() {
    let world = world();
    let (laptop, home) = (&world.laptop, &world.home);
    let started = laptop.web(
        "POST",
        "/api/agents/start",
        Some(serde_json::json!({"clone": laptop.repo, "agent": "v1", "prompt": "Artifact"})),
    );
    let thread = started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("thread").to_owned());
    assert!(eventually(|| artifacts(laptop, &thread).len() == 1), "the laptop keeps it");

    let mut seen = Vec::new();
    assert!(
        eventually(|| {
            seen = artifacts(home, &thread);
            seen.len() == 1
        }),
        "the other machine pulls it: {seen:?}"
    );
    let blob = seen[0]["blob"].as_str().expect("blob");
    let content = home.web("GET", &format!("/api/threads/{thread}/artifacts/{blob}"), None);
    let bytes = base64::engine::general_purpose::STANDARD.decode(content["data"].as_str().expect("data")).expect("base64");
    assert_eq!(String::from_utf8_lossy(&bytes), "<h1>Rescan benchmark</h1>", "its content comes from the server");
    assert_eq!(content["media_type"], "text/html");
}

/// What `url` answers: its status, the headers asked for, and its body.
fn fetch(url: &str, headers: &[&str]) -> (String, Vec<String>, String) {
    let output = std::process::Command::new("curl").args(["-s", "-D", "-", url]).output().expect("curl runs");
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head.split_whitespace().nth(1).unwrap_or_default().to_owned();
    let found = headers
        .iter()
        .map(|name| {
            head.lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(key, _)| key.eq_ignore_ascii_case(name))
                        .map(|(_, value)| value.trim().to_owned())
                })
                .unwrap_or_default()
        })
        .collect();
    (status, found, body.to_owned())
}

/// A read-only link to one artifact (ADR 0068) shows it alone, its content
/// served under a sandbox that lets it reach nothing; a link names only an
/// artifact the thread has.
#[test]
fn a_read_only_link_shows_one_artifact_sandboxed() {
    let world = world();
    let laptop = &world.laptop;
    let started = laptop.web(
        "POST",
        "/api/agents/start",
        Some(serde_json::json!({"clone": laptop.repo, "agent": "v1", "prompt": "Artifact"})),
    );
    let thread = started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("thread").to_owned());
    let mut shown = Vec::new();
    assert!(
        eventually(|| {
            shown = artifacts(laptop, &thread);
            shown.len() == 1
                && laptop.web_answer("GET", &format!("/api/threads/{thread}/links"), None).0 == "200"
                && laptop
                    .web_answer(
                        "POST",
                        &format!("/api/threads/{thread}/links"),
                        Some(serde_json::json!({"days": 1, "artifact": {"id": "0123456789ab"}})),
                    )
                    .0
                    == "400"
        }),
        "the server has the thread, and refuses an artifact it doesn't have"
    );
    let id = shown[0]["id"].as_str().expect("id").to_owned();
    // The artifact reaches the server after the thread does.
    let mut link = serde_json::Value::Null;
    assert!(
        eventually(|| {
            let body = serde_json::json!({"days": 1, "artifact": {"id": id}});
            let (status, answer) = laptop.web_answer("POST", &format!("/api/threads/{thread}/links"), Some(body));
            link = serde_json::from_str(&answer).unwrap_or_default();
            status == "200"
        }),
        "the server takes a link to the artifact once it has it: {link}"
    );
    let url = link["url"].as_str().expect("the link");
    assert!(url.starts_with(&world.server.url), "{url}");
    assert_eq!(link["artifact"]["id"], id);

    let (status, _, page) = fetch(url, &[]);
    assert_eq!(status, "200", "{page}");
    assert!(page.contains("Rescan benchmark") && page.contains(r#"sandbox="allow-scripts""#), "{page}");
    assert!(!page.contains("Done: Artifact"), "nothing else of the thread: {page}");

    let (status, headers, body) =
        fetch(&format!("{url}/content"), &["content-security-policy", "content-type", "x-content-type-options"]);
    assert_eq!(status, "200");
    assert_eq!(body, "<h1>Rescan benchmark</h1>");
    assert!(headers[0].starts_with("sandbox allow-scripts;") && headers[0].contains("default-src 'none'"), "{headers:?}");
    assert!(headers[0].contains("https://cdnjs.cloudflare.com") && !headers[0].contains("connect-src"), "{headers:?}");
    assert_eq!((headers[1].as_str(), headers[2].as_str()), ("text/html; charset=utf-8", "nosniff"));

    let thread_link = laptop.web("POST", &format!("/api/threads/{thread}/links"), Some(serde_json::json!({"days": 1})));
    let (status, _, _) = fetch(&format!("{}/content", thread_link["url"].as_str().expect("link")), &[]);
    assert_eq!(status, "404", "a link to the conversation serves no artifact content");
}
