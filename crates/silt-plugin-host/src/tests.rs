//! The host against the probe plugin (`plugins/probe`), built for
//! `wasm32-wasip2` the first time a test needs it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::json;
use silt_plugin::{Context, Effect, Manifest, Observed, Refused};

use crate::{Capabilities, Env, Host, HostError, HttpRequest, HttpResponse, Limits, Scope};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The probe, built once per test run.
fn probe() -> &'static [u8] {
    static WASM: OnceLock<Vec<u8>> = OnceLock::new();
    WASM.get_or_init(|| {
        let target = repo().join("target/plugins");
        let status = std::process::Command::new(env!("CARGO"))
            .args(["build", "--release", "--target", "wasm32-wasip2", "-p", "silt-plugin-probe", "--manifest-path"])
            .arg(repo().join("plugins/Cargo.toml"))
            .arg("--target-dir")
            .arg(&target)
            .status()
            .expect("cargo runs");
        assert!(status.success(), "the probe builds");
        std::fs::read(target.join("wasm32-wasip2/release/silt_plugin_probe.wasm")).expect("the probe's component")
    })
}

const MANIFEST: &str = r#"
id = "test.probe"
name = "Probe"
version = "1.0.0"
publisher = "test"
description = "Does what the host must allow or refuse."
license = "AGPL-3.0-only"
world = "silt:plugin@1"
build = { cargo = { package = "silt-plugin-probe", toolchain = "1.99.1" } }

[[capabilities]]
kind = "http"
hosts = ["api.probe.test"]
reason = "Reads and makes things."

[[actions]]
id = "look"
label = "Look"
description = "Reads."
kind = "read"
default = "allow"
fields = ["name"]

[[actions]]
id = "change"
label = "Change"
description = "Makes a thing."
kind = "change"
default = "ask"
fields = ["name"]
"#;

#[derive(Default)]
struct Fake {
    sent: Mutex<Vec<HttpRequest>>,
}

impl Capabilities for Fake {
    fn http(&self, _scope: &Scope, request: HttpRequest) -> Result<HttpResponse, String> {
        self.sent.lock().unwrap().push(request.clone());
        Ok(HttpResponse { status: if request.method == "POST" { 201 } else { 200 }, headers: vec![], body: b"green".to_vec() })
    }
}

fn granted(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

struct World {
    host: Host,
    fake: Arc<Fake>,
    env: Env,
    _dir: tempfile::TempDir,
}

fn world(limits: Limits) -> World {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::default());
    let env = Env {
        capabilities: fake.clone(),
        scope: Scope { plugin: "test.probe".into(), thread: "t1".into(), workspace: "work".into(), environment: None },
        data_dir: dir.path().join("data"),
        granted: granted(&["http"]),
    };
    World { host: Host::new(limits).unwrap(), fake, env, _dir: dir }
}

fn context() -> Context {
    Context { thread: "t1".into(), workspace: "work".into(), ..Context::default() }
}

#[test]
fn a_plugin_describes_its_tools_and_reads_while_it_runs() {
    let world = world(Limits::default());
    let loaded = world.host.load(Manifest::parse(MANIFEST).unwrap(), probe(), &world.env.granted).unwrap();
    let tools = world.host.describe(&loaded, &world.env).unwrap();
    assert!(tools.iter().any(|tool| tool.name == "look" && tool.action == "look"));
    let prepared = world.host.prepare(&loaded, &world.env, "look", &json!({}), &context()).unwrap();
    assert_eq!((prepared.plugin.as_str(), prepared.action.as_str()), ("test.probe", "look"));
    let ran = world.host.run(&loaded, &world.env, &prepared, &context()).unwrap();
    assert_eq!(ran.output.unwrap().text, "green");
    assert!(ran.refused.is_empty());
}

#[test]
fn a_change_sends_only_what_was_approved() {
    let world = world(Limits::default());
    let loaded = world.host.load(Manifest::parse(MANIFEST).unwrap(), probe(), &world.env.granted).unwrap();
    let prepared = world.host.prepare(&loaded, &world.env, "change", &json!({"name": "ada"}), &context()).unwrap();
    assert_eq!(prepared.fields.get("name").map(String::as_str), Some("ada"));
    assert!(prepared.card.title.contains("was green"), "it looked while preparing: {}", prepared.card.title);
    let Effect::Http { body_sha256, .. } = &prepared.effects[0] else { panic!("{:?}", prepared.effects) };
    assert_eq!(body_sha256.as_deref(), Some(silt_plugin::sha256_hex(br#"{"name":"ada"}"#).as_str()));
    let ran = world.host.run(&loaded, &world.env, &prepared, &context()).unwrap();
    assert_eq!(ran.output.unwrap().text, "made ada: 201");
    let posts: Vec<_> = world.fake.sent.lock().unwrap().iter().filter(|request| request.method == "POST").cloned().collect();
    assert_eq!(posts.len(), 1);

    // The same call with another name isn't what was approved.
    let mut tampered = prepared.clone();
    tampered.input = json!({"name": "eve"});
    let ran = world.host.run(&loaded, &world.env, &tampered, &context()).unwrap();
    assert!(ran.output.is_err());
    assert!(matches!(&ran.refused[..], [Refused::Http { .. }]), "{:?}", ran.refused);
}

#[test]
fn an_undeclared_change_is_refused_and_reported() {
    let world = world(Limits::default());
    let loaded = world.host.load(Manifest::parse(MANIFEST).unwrap(), probe(), &world.env.granted).unwrap();
    let prepared = world.host.prepare(&loaded, &world.env, "sneak", &json!({"name": "ada"}), &context()).unwrap();
    let ran = world.host.run(&loaded, &world.env, &prepared, &context()).unwrap();
    let refused = ran.output.unwrap_err();
    assert!(refused.contains("delete-everything"), "{refused}");
    assert_eq!(ran.refused.len(), 1);
    assert!(!world.fake.sent.lock().unwrap().iter().any(|request| request.url.contains("delete-everything")), "never sent");
    assert!(ran.observed.iter().all(|observed| !matches!(observed, Observed::Http { url, .. } if url.contains("delete"))));
}

#[test]
fn nothing_changes_while_a_call_is_prepared() {
    let world = world(Limits::default());
    let loaded = world.host.load(Manifest::parse(MANIFEST).unwrap(), probe(), &world.env.granted).unwrap();
    let error = world.host.prepare(&loaded, &world.env, "change_while_preparing", &json!({}), &context()).unwrap_err();
    assert!(error.to_string().contains("can't change anything while it prepares"), "{error}");
    assert!(world.fake.sent.lock().unwrap().iter().all(|request| request.method != "POST"));
}

#[test]
fn a_capability_not_granted_keeps_the_plugin_from_loading() {
    let world = world(Limits::default());
    let error = world.host.load(Manifest::parse(MANIFEST).unwrap(), probe(), &BTreeSet::new()).err().unwrap();
    assert!(matches!(&error, HostError::NotGranted { capability, .. } if capability == "http"), "{error}");
}

#[test]
fn a_plugin_past_its_time_stops() {
    let world = world(Limits { run_seconds: 1, ..Limits::default() });
    let loaded = world.host.load(Manifest::parse(MANIFEST).unwrap(), probe(), &world.env.granted).unwrap();
    let prepared = world.host.prepare(&loaded, &world.env, "spin", &json!({}), &context()).unwrap();
    let started = std::time::Instant::now();
    let ran = world.host.run(&loaded, &world.env, &prepared, &context()).unwrap();
    assert!(ran.output.unwrap_err().contains("ran past its time"));
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}

#[test]
fn a_plugin_sees_only_its_own_folder() {
    let world = world(Limits::default());
    let loaded = world.host.load(Manifest::parse(MANIFEST).unwrap(), probe(), &world.env.granted).unwrap();
    let prepared = world.host.prepare(&loaded, &world.env, "files", &json!({}), &context()).unwrap();
    let ran = world.host.run(&loaded, &world.env, &prepared, &context()).unwrap();
    assert_eq!(ran.output.unwrap().text, "wrote /data/note; no /etc/passwd");
    assert_eq!(std::fs::read(world.env.data_dir.join("note")).unwrap(), b"kept");
}
