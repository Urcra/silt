//! The harness on the probe plugin, which does what a host must allow or
//! refuse.

use serde_json::json;
use silt_plugin_host::testing::{PROBE_MANIFEST, plugin};
use silt_plugin_test::{Effect, Harness, HarnessError, check};

fn probe() -> Harness {
    Harness::new(PROBE_MANIFEST, &plugin("silt-plugin-probe")).expect("the probe loads")
}

#[test]
fn a_call_sends_what_it_prepared_and_the_test_sees_it() {
    let probe = probe();
    assert!(probe.tools().unwrap().iter().any(|tool| tool.name == "change"));
    probe.answer("GET", "https://api.probe.test/status", 200, "green").answer("POST", "https://api.probe.test/", 201, "");
    let prepared = probe.prepare("change", json!({"name": "ada"})).unwrap();
    assert!(matches!(&prepared.effects[..], [Effect::Http { method, url, .. }] if method == "POST" && url.ends_with("/things")));
    let ran = probe.run(&prepared).unwrap();
    assert_eq!(ran.output.unwrap().text, "made ada: 201");
    assert!(ran.refused.is_empty());
    let sent: Vec<String> = probe.requests().into_iter().map(|request| format!("{} {}", request.method, request.url)).collect();
    assert_eq!(sent, ["GET https://api.probe.test/status", "POST https://api.probe.test/things"]);
}

#[test]
fn what_a_call_didnt_prepare_is_refused_and_never_sent() {
    let probe = probe();
    probe.answer("GET", "https://api.probe.test/", 200, "green").answer("POST", "https://api.probe.test/", 201, "");
    let prepared = probe.prepare("sneak", json!({"name": "ada"})).unwrap();
    let ran = probe.run(&prepared).unwrap();
    assert!(!ran.refused.is_empty(), "{:?}", ran.output);
    assert!(!probe.requests().iter().any(|request| request.url.contains("delete-everything")));
}

#[test]
fn preparing_may_not_change_anything() {
    let probe = probe();
    probe.answer("POST", "https://api.probe.test/", 201, "");
    assert!(probe.prepare("change_while_preparing", json!({})).is_err());
    assert!(probe.requests().is_empty());
}

#[test]
fn a_build_importing_what_its_manifest_doesnt_declare_doesnt_load() {
    let undeclared = PROBE_MANIFEST.replace("kind = \"http\"\nhosts = [\"api.probe.test\"]", "kind = \"thread-read\"");
    assert!(matches!(check(&undeclared, &plugin("silt-plugin-probe")), Err(HarnessError::Host(_))));
    assert!(check(PROBE_MANIFEST, &plugin("silt-plugin-probe")).is_ok());
}
