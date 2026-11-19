//! A third party's plugin added from git (ADR 0104): shown before it's
//! added, built offline in a sandbox, refused what it didn't declare while
//! a call runs, and updated to a newer tag only with agreement to what the
//! new version adds.

mod common;

use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use common::{Machine, Server, canonical, git, origin};

fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("the repository")
}

fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("a folder");
    for entry in fs::read_dir(from).expect("readable").flatten() {
        let path = entry.path();
        if entry.file_type().expect("its type").is_dir() {
            if entry.file_name() != "target" {
                copy(&path, &to.join(entry.file_name()));
            }
        } else {
            fs::copy(&path, to.join(entry.file_name())).expect("copied");
        }
    }
}

/// The probe plugin as a repository of its own, with Silt's SDK inside it:
/// a build sandbox sees nothing of this machine but the repository.
fn probe_repository(at: &Path) {
    let silt = repo_root();
    copy(&silt.join("plugins/probe"), &at.join("probe"));
    for sdk in ["silt-plugin-api", "silt-plugin", "silt-access"] {
        copy(&silt.join("crates").join(sdk), &at.join("sdk").join(sdk));
    }
    let probe = at.join("probe/Cargo.toml");
    let text =
        fs::read_to_string(&probe).expect("its Cargo.toml").replace("../../crates/silt-plugin-api", "../sdk/silt-plugin-api");
    fs::write(&probe, text).expect("written");
    fs::write(
        at.join("Cargo.toml"),
        r#"[workspace]
resolver = "3"
members = ["probe", "sdk/*"]

[workspace.package]
version = "1.0.0"
edition = "2024"
rust-version = "1.99"
license = "AGPL-3.0-only"
publish = false

[workspace.lints.rust]
unsafe_code = "deny"

[profile.release]
opt-level = "s"
lto = true
codegen-units = 1
strip = true
panic = "abort"
"#,
    )
    .expect("written");
    fs::copy(silt.join("plugins/Cargo.lock"), at.join("Cargo.lock")).expect("the lock");
    fs::write(at.join("silt-plugin.toml"), silt_plugin_host_manifest()).expect("its manifest");
    fs::write(at.join("README.md"), "# Probe\n\nDoes what a host must allow or refuse.\n").expect("its README");
}

fn silt_plugin_host_manifest() -> String {
    include_str!("../../silt-plugin-host/src/testing.rs")
        .split("pub const PROBE_MANIFEST: &str = r#\"")
        .nth(1)
        .and_then(|rest| rest.split("\"#;").next())
        .expect("the probe's manifest")
        .to_owned()
}

fn failed_with(output: &Output, said: &str) -> bool {
    !output.status.success() && String::from_utf8_lossy(&output.stderr).contains(said)
}

fn said(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

/// What `silt mcp`, as the run with `token`, answers a call of `tool`.
fn mcp_answer(machine: &Machine, thread: &str, token: &str, tool: &str, arguments: serde_json::Value) -> String {
    let mut mcp = Command::new(env!("CARGO_BIN_EXE_silt"))
        .args(["mcp", "--thread", thread, "--token", token])
        .env("SILT_HOME", &machine.silt_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("silt mcp runs");
    {
        let mut stdin = mcp.stdin.take().expect("stdin");
        let call = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": arguments}});
        writeln!(stdin, "{call}").expect("write");
    }
    String::from_utf8_lossy(&mcp.wait_with_output().expect("mcp exits").stdout).into_owned()
}

#[test]
fn a_plugin_from_git_is_held_to_what_it_declared_and_updates_only_with_agreement() {
    let root = common::temp_root("silt-git-plugins");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    // The real home, for the toolchain a sandboxed build is given.
    let home = std::env::var("HOME").expect("a home");
    let with_toolchain = [("HOME", home.as_str())];

    let repository = root.path().join("probe-repo");
    probe_repository(&repository);
    git(&repository, &["init", "-q", "-b", "main"]);
    git(&repository, &["add", "."]);
    git(&repository, &["commit", "-qm", "The probe"]);
    git(&repository, &["tag", "v1.0.0"]);
    let url = format!("file://{}", repository.display());

    // Without agreeing, nothing is added; what it gets is said first.
    let refused = laptop.command(&["plugin", "add", &format!("{url}#v1.0.0")], &with_toolchain).output().expect("runs");
    assert!(failed_with(&refused, "--yes"), "{}", said(&refused));
    assert!(said(&refused).contains("api.probe.test"), "it names the host it reaches: {}", said(&refused));
    let added = laptop.silt_with(&["plugin", "add", &format!("{url}#v1.0.0"), "--yes", "--workspace", "work"], &with_toolchain);
    assert!(said(&added).contains("test.probe"), "{}", said(&added));
    let listed = String::from_utf8_lossy(&laptop.silt(&["plugin", "list"]).stdout).into_owned();
    assert!(listed.contains("test.probe 1.0.0") && listed.contains(&format!("from {url}")), "{listed}");

    // Its call, from a run in the workspace it's on in.
    let work = root.path().join("laptop/work");
    git(root.path(), &["clone", "-q", &origin.to_string_lossy(), &work.to_string_lossy()]);
    let work = canonical(&work);
    laptop.silt(&["workspace", "new", "Work", "--local", "--folder", &work.to_string_lossy()]);
    assert!(laptop.silt(&["plugin", "enable", "test.probe", "--workspace", "work"]).status.success());
    let ran = laptop.command(&["claude"], &[]).current_dir(&work).output().expect("silt runs");
    assert!(ran.status.success(), "{}", said(&ran));
    let config = fs::read_to_string(laptop.claude_dir.join("mcp-config.json")).expect("Claude got Silt's MCP server");
    let config: serde_json::Value = serde_json::from_str(&config).expect("json");
    let args: Vec<String> = config["mcpServers"]["silt"]["args"]
        .as_array()
        .expect("args")
        .iter()
        .filter_map(|arg| arg.as_str().map(str::to_owned))
        .collect();
    let token = args.iter().skip_while(|arg| *arg != "--token").nth(1).cloned().expect("a run token");
    let thread = args.iter().skip_while(|arg| *arg != "--thread").nth(1).cloned().expect("the run's thread");

    // Its publisher isn't trusted, so even a read asks; approved, a request
    // it didn't prepare is refused.
    assert!(laptop.silt(&["plugin", "trust", "test"]).status.success());
    let sneaked = mcp_answer(&laptop, &thread, &token, "probe_change_while_preparing", serde_json::json!({}));
    assert!(sneaked.contains("isError") || sneaked.to_lowercase().contains("refused"), "{sneaked}");
    let files = mcp_answer(&laptop, &thread, &token, "probe_files", serde_json::json!({}));
    assert!(files.contains("wrote /data/note; no /etc/passwd"), "it sees only its own folder: {files}");

    // A newer tag that reaches another host waits for agreement.
    let manifest = fs::read_to_string(repository.join("silt-plugin.toml"))
        .expect("its manifest")
        .replace("version = \"1.0.0\"", "version = \"1.1.0\"")
        .replace("hosts = [\"api.probe.test\"]", "hosts = [\"api.probe.test\", \"elsewhere.test\"]");
    fs::write(repository.join("silt-plugin.toml"), manifest).expect("written");
    git(&repository, &["commit", "-qam", "Reach elsewhere too"]);
    git(&repository, &["tag", "v1.1.0"]);

    let offered = laptop.web("POST", "/api/plugins/check", None);
    let update = offered.as_array().and_then(|updates| updates.first()).cloned().expect("an update is waiting");
    assert_eq!((update["from"].as_str(), update["to"].as_str()), (Some("1.0.0"), Some("1.1.0")), "{update}");
    assert!(update["adds"].to_string().contains("elsewhere.test"), "{update}");
    let (status, answer) =
        laptop.web_answer("POST", "/api/plugins/test.probe/update", Some(serde_json::json!({"consented": false})));
    assert!(status != "200" && answer.contains("asks for more"), "{status} {answer}");

    let unagreed = laptop.command(&["plugin", "update", "test.probe"], &with_toolchain).output().expect("runs");
    assert!(failed_with(&unagreed, "--yes"), "{}", said(&unagreed));
    assert!(said(&unagreed).contains("Reach elsewhere too"), "it says what came in: {}", said(&unagreed));
    let updated = laptop.silt_with(&["plugin", "update", "test.probe", "--yes"], &with_toolchain);
    assert!(said(&updated).contains("test.probe is now 1.1.0 (v1.1.0)"), "{}", said(&updated));
    let listed = String::from_utf8_lossy(&laptop.silt(&["plugin", "list"]).stdout).into_owned();
    assert!(listed.contains("test.probe 1.1.0"), "{listed}");

    // Its page has the calls it made.
    let activity = laptop.web("GET", "/api/plugins/test.probe/activity", None);
    assert!(activity["calls"].as_array().is_some_and(|calls| calls.len() >= 2), "{activity}");
    assert!(activity["broken"].is_null(), "{activity}");
}
