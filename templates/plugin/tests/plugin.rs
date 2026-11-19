//! The plugin in Silt's stand-in host: what each call prepares, and that it
//! sends only what was approved.

use serde_json::json;
use silt_plugin_test::{Effect, Harness};

fn notes() -> Harness {
    Harness::build(env!("CARGO_MANIFEST_DIR").as_ref()).expect("it builds and loads")
}

#[test]
fn listing_reads_and_adding_sends_exactly_the_note_shown() {
    let notes = notes();
    let tools: Vec<String> = notes.tools().unwrap().into_iter().map(|tool| tool.name).collect();
    assert_eq!(tools, ["list", "add"]);

    notes.answer("GET", "https://api.notes.example/notebooks/work/notes", 200, r#"[{"text":"hi"}]"#);
    let listed = notes.prepare("list", json!({"notebook": "work"})).unwrap();
    assert_eq!(listed.action, "list");
    let ran = notes.run(&listed).unwrap();
    assert!(ran.output.unwrap().text.contains("hi"));

    let added = notes.prepare("add", json!({"notebook": "work", "text": "ship it"})).unwrap();
    assert!(matches!(&added.effects[..], [Effect::Http { method, .. }] if method == "POST"));
    notes.answer("POST", "https://api.notes.example/", 201, "{}");
    let ran = notes.run(&added).unwrap();
    assert!(ran.output.is_ok() && ran.refused.is_empty());
    let sent = notes.requests().pop().unwrap();
    assert_eq!(sent.body.as_deref(), Some(br#"{"text":"ship it"}"#.as_slice()));
}

#[test]
fn a_notebook_that_could_reach_elsewhere_is_refused() {
    assert!(notes().prepare("list", json!({"notebook": "../admin"})).is_err());
}
