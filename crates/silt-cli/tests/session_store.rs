//! A Silt server as the session store of an Agent SDK app (ADR 0042): what
//! one host appends, another loads, and the person's machines see the
//! session as a thread.
// The agents these tests drive are shell-script stand-ins.
#![cfg(unix)]

mod common;

use std::process::Command;

use common::{Machine, Server, eventually, origin};
use serde_json::{Value, json};
use silt_proto::{PROTOCOL, PROTOCOL_HEADER};

const SESSION: &str = "5f1c2a90-7d4e-4b8e-9a51-0c6c1e2d3f40";
const PROJECT_KEY: &str = "-srv-app";

/// An HTTP call to the server, with its status and JSON answer.
fn call(base: &str, token: Option<&str>, method: &str, path: &str, body: Option<&Value>) -> (u16, Value) {
    let mut command = Command::new("curl");
    command.args(["-s", "-w", "\n%{http_code}", "-X", method, "-H", "Content-Type: application/json", "-H"]);
    command.arg(format!("{PROTOCOL_HEADER}: {PROTOCOL}"));
    if let Some(token) = token {
        command.arg("-H").arg(format!("Authorization: Bearer {token}"));
    }
    if let Some(body) = body {
        command.arg("--data").arg(body.to_string());
    }
    let output = command.arg(format!("{base}{path}")).output().expect("curl runs");
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let (json, status) = text.rsplit_once('\n').expect("a status line");
    (status.parse().expect("a status"), serde_json::from_str(json).unwrap_or(Value::Null))
}

/// A device token for an app's host, as `silt auth login` would get one.
fn host_token(server: &Server, name: &str) -> String {
    let code = server.pairing_code_for("owner");
    let (status, reply) = call(&server.url, None, "POST", "/v1/login", Some(&json!({ "code": code, "name": name })));
    assert_eq!(status, 200, "{reply}");
    reply["token"].as_str().expect("a token").to_owned()
}

fn key(subpath: Option<&str>) -> Value {
    match subpath {
        Some(subpath) => json!({ "projectKey": PROJECT_KEY, "sessionId": SESSION, "subpath": subpath }),
        None => json!({ "projectKey": PROJECT_KEY, "sessionId": SESSION }),
    }
}

#[test]
fn a_session_one_host_stores_is_loaded_by_another_and_reaches_the_persons_machines() {
    let root = tempfile::Builder::new().prefix("silt-sdk").tempdir_in("/tmp").expect("temp dir");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let home = Machine::new(root.path(), "home", &origin, &server);
    let (worker, other_worker) = (host_token(&server, "worker-1"), host_token(&server, "worker-2"));

    let prompt = json!({
        "type": "user", "uuid": "u1", "sessionId": SESSION, "cwd": "/srv/app",
        "message": { "role": "user", "content": "Summarize the billing module" },
    });
    let reply = json!({
        "type": "assistant", "uuid": "a1", "sessionId": SESSION,
        "message": { "role": "assistant", "content": [
            { "type": "text", "text": "Billing charges cards monthly with ghp_abcdefghijklmnopqrstuvwxyz0123." },
        ] },
    });
    let subagent = json!({ "type": "user", "uuid": "s1", "isSidechain": true, "sessionId": SESSION, "message": { "role": "user", "content": "Read billing.rs" } });
    let append = |token: &str, subpath: Option<&str>, entries: Vec<&Value>| {
        let body =
            json!({ "key": key(subpath), "entries": entries, "repo": silt_git::normalize_remote(&origin.to_string_lossy()) });
        call(&server.url, Some(token), "POST", "/v1/session-store/append", Some(&body))
    };
    assert_eq!(append(&worker, None, vec![&prompt, &reply]).0, 204);
    assert_eq!(append(&worker, None, vec![&reply]).0, 204, "a retried batch");
    assert_eq!(append(&worker, Some("subagents/agent-1"), vec![&subagent]).0, 204);
    assert_eq!(append(&worker, Some("../escape"), vec![&subagent]).0, 400, "only subagent transcripts sit beside it");

    let load = |token: &str, subpath: Option<&str>| {
        call(&server.url, Some(token), "POST", "/v1/session-store/load", Some(&json!({ "key": key(subpath) }))).1
    };
    let loaded = load(&other_worker, None);
    let entries = loaded.as_array().expect("the session's entries");
    assert_eq!(entries.len(), 2, "the retried entry is kept once: {loaded}");
    assert_eq!(entries[0], prompt, "an entry comes back as it went");
    let said = entries[1]["message"]["content"][0]["text"].as_str().expect("the reply");
    assert!(said.starts_with("Billing charges cards monthly") && !said.contains("ghp_abcdef"), "redacted: {said}");
    assert_eq!(load(&other_worker, Some("subagents/agent-1")), json!([subagent]));
    let (_, listed) =
        call(&server.url, Some(&other_worker), "GET", &format!("/v1/session-store/sessions?project_key={PROJECT_KEY}"), None);
    assert_eq!(listed[0]["sessionId"], SESSION, "{listed}");
    let subkeys = json!({ "projectKey": PROJECT_KEY, "sessionId": SESSION });
    let (_, subkeys) = call(&server.url, Some(&other_worker), "POST", "/v1/session-store/subkeys", Some(&subkeys));
    assert_eq!(subkeys, json!(["subagents/agent-1"]));

    assert!(eventually(|| home.try_stdout(&["search", "billing"]).contains("Summarize the [billing] module")), "searchable");
    assert!(eventually(|| home.try_stdout(&["log"]).contains("Summarize the billing module")), "listed on the machine");
    let thread = home.stdout(&["log"]).split_whitespace().next().expect("the session's thread").to_owned();
    let shown = home.stdout(&["show", &thread]);
    assert!(shown.contains("Summarize the billing module") && shown.contains("Billing charges cards"), "{shown}");

    let (status, _) = call(&server.url, Some(&worker), "POST", "/v1/session-store/delete", Some(&json!({ "key": key(None) })));
    assert_eq!(status, 204);
    assert_eq!(load(&other_worker, None), Value::Null, "a deleted session is unknown");
}
