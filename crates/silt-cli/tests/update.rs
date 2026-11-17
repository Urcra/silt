//! `silt update` on a build pointed at a clone behind its upstream, with a
//! stand-in for `just` that echoes the recipe lines the real one does
//! (ADR 0098). Unix only: the stand-in is a shell script, and Windows
//! updates go through WSL.
#![cfg(unix)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::{git, silt_command, temp_root};

const STAND_IN: &str = r#"#!/bin/sh
[ "$1" = update ] || { echo "only update" >&2; exit 2; }
echo "pnpm -C web install --frozen-lockfile" >&2
echo "pnpm -C web build"
echo "cargo install --locked --path crates/silt-cli" >&2
echo "   Compiling silt-types v0.0.1"
echo "   Compiling silt-cli v0.0.1"
if [ -n "${FAIL_THE_BUILD:-}" ]; then
    echo "error: could not compile silt-cli" >&2
    exit 101
fi
echo "cargo install --locked --path crates/silt-voice"
pwd > "$(dirname "$0")/ran-in"
"#;

struct Setup {
    _root: tempfile::TempDir,
    home: PathBuf,
    upstream: PathBuf,
    clone: PathBuf,
    first: String,
    just: PathBuf,
}

impl Setup {
    fn new() -> Self {
        let root = temp_root("silt-update");
        let upstream = root.path().join("upstream");
        fs::create_dir_all(&upstream).expect("dir");
        git(&upstream, &["init", "-q", "--initial-branch=main"]);
        fs::write(upstream.join("README.md"), "one").expect("write");
        git(&upstream, &["add", "-A"]);
        git(&upstream, &["commit", "-q", "-m", "first"]);
        let first = git(&upstream, &["rev-parse", "HEAD"]).trim().to_owned();
        let clone = root.path().join("clone");
        git(root.path(), &["clone", "-q", &upstream.to_string_lossy(), &clone.to_string_lossy()]);
        for (file, message) in [("a.txt", "feat: a"), ("crates/silt-desktop/main.rs", "feat(desktop): b")] {
            let path = upstream.join(file);
            fs::create_dir_all(path.parent().expect("parent")).expect("dir");
            fs::write(&path, message).expect("write");
            git(&upstream, &["add", "-A"]);
            git(&upstream, &["commit", "-q", "-m", message]);
        }
        let bin = root.path().join("bin");
        fs::create_dir_all(&bin).expect("dir");
        let just = bin.join("just");
        fs::write(&just, STAND_IN).expect("write");
        silt_os::make_executable(&just).expect("chmod");
        let home = root.path().join("home");
        fs::create_dir_all(home.join("claude")).expect("dir");
        Self { home, upstream, clone, first, just, _root: root }
    }

    fn silt(&self, args: &[&str], envs: &[(&str, &str)]) -> Command {
        let mut command = silt_command();
        command
            .args(args)
            .current_dir(&self.clone)
            .env("SILT_HOME", self.home.join("silt"))
            .env("HOME", &self.home)
            .env("CLAUDE_CONFIG_DIR", self.home.join("claude"))
            .env("CODEX_HOME", self.home.join("codex"))
            .env("GEMINI_CLI_HOME", self.home.join("gemini"))
            .env("COPILOT_HOME", self.home.join("copilot"))
            .env("SILT_BUILD_SOURCE", &self.clone)
            .env("SILT_BUILD_COMMIT", &self.first)
            .env("SILT_BUILD_BRANCH", "main")
            .env("SILT_BUILD_CHANNEL", "nightly")
            .env("SILT_UPDATE_JUST", &self.just)
            .envs(envs.iter().copied());
        command
    }

    fn run(&self, args: &[&str], envs: &[(&str, &str)]) -> Output {
        common::output_once_released(&mut self.silt(args, envs))
    }

    fn state(&self) -> serde_json::Value {
        let text = fs::read_to_string(self.home.join("silt/update/state.json")).expect("the update's state");
        serde_json::from_str(&text).expect("json")
    }

    fn phases(&self) -> Vec<String> {
        self.state()["steps"]
            .as_array()
            .expect("steps")
            .iter()
            .map(|step| step["phase"].as_str().expect("a phase").to_owned())
            .collect()
    }
}

impl Drop for Setup {
    fn drop(&mut self) {
        let _ = silt_command().args(["daemon", "stop"]).env("SILT_HOME", self.home.join("silt")).output();
    }
}

fn text(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

#[test]
fn an_update_fast_forwards_main_builds_it_and_restarts_the_daemon() {
    let setup = Setup::new();
    let checked = setup.run(&["update", "--check"], &[]);
    assert!(checked.status.success(), "{}", text(&checked));
    let said = String::from_utf8_lossy(&checked.stdout);
    assert!(said.contains("main is 2 commits ahead"), "{said}");
    assert!(said.contains("feat(desktop): b"), "{said}");

    let updated = setup.run(&["update"], &[]);
    assert!(updated.status.success(), "{}", text(&updated));
    let newest = git(&setup.upstream, &["rev-parse", "HEAD"]);
    assert_eq!(git(&setup.clone, &["rev-parse", "HEAD"]), newest, "main fast-forwarded");
    let ran_in = fs::read_to_string(setup.just.with_file_name("ran-in")).expect("the stand-in ran");
    assert_eq!(Path::new(ran_in.trim()).canonicalize().ok(), setup.clone.canonicalize().ok());

    let state = setup.state();
    assert_eq!(state["outcome"]["kind"], "done", "{state}");
    assert_eq!(state["to"].as_str(), Some(newest.trim()));
    assert_eq!(setup.phases(), ["fetch", "web_dependencies", "web_build", "cli", "voice", "restart"]);
    let cli = state["steps"].as_array().expect("steps").iter().find(|step| step["phase"] == "cli").expect("the cli step").clone();
    assert_eq!(cli["compiled"], 2);
    let output = fs::read_to_string(setup.home.join("silt/update/output.log")).expect("output");
    assert!(output.contains("cargo install --locked --path crates/silt-cli"), "{output}");

    // The restart left a daemon running, on this build.
    let status = setup.run(&["daemon", "status"], &[]);
    assert!(status.status.success(), "{}", text(&status));
}

#[test]
fn a_failed_build_names_its_step_and_leaves_the_daemon_alone() {
    let setup = Setup::new();
    let failed = setup.run(&["update"], &[("FAIL_THE_BUILD", "1")]);
    assert!(!failed.status.success());
    let said = text(&failed);
    assert!(said.contains("Build silt failed: error: could not compile silt-cli"), "{said}");
    let state = setup.state();
    assert_eq!(state["outcome"]["kind"], "failed", "{state}");
    assert_eq!(setup.phases().last().map(String::as_str), Some("cli"));
    let status = setup.run(&["daemon", "status"], &[]);
    assert!(text(&status).contains("not running"), "{}", text(&status));
}

#[test]
fn a_clone_with_changes_is_not_updated() {
    let setup = Setup::new();
    fs::write(setup.clone.join("README.md"), "edited").expect("edit");
    let refused = setup.run(&["update"], &[]);
    assert!(!refused.status.success());
    assert!(text(&refused).contains("its clone has uncommitted changes"), "{}", text(&refused));
    assert_eq!(git(&setup.clone, &["rev-parse", "HEAD"]).trim(), setup.first);
}

impl Setup {
    /// The daemon's web API: the status and the answer.
    fn web(&self, method: &str, path: &str) -> (String, serde_json::Value) {
        let opened = self.run(&["open", "--no-browser"], &[]);
        assert!(opened.status.success(), "{}", text(&opened));
        let url = String::from_utf8_lossy(&opened.stdout).into_owned();
        let (base, token) = common::web_ui(&url);
        let output = Command::new("curl")
            .args(["-s", "-w", "\n%{http_code}", "-X", method, "-H"])
            .arg(format!("Authorization: Bearer {token}"))
            .arg(format!("{base}{path}"))
            .output()
            .expect("curl runs");
        let said = String::from_utf8_lossy(&output.stdout).into_owned();
        let (json, status) = said.rsplit_once('\n').expect("a status line");
        (status.to_owned(), serde_json::from_str(json).unwrap_or(serde_json::Value::Null))
    }
}

#[test]
fn the_web_ui_sees_the_update_starts_it_and_reads_it_back_after_the_restart() {
    let setup = Setup::new();
    let mut view = serde_json::Value::Null;
    assert!(
        common::eventually(|| {
            view = setup.web("GET", "/api/update").1;
            view["availability"]["kind"] == "behind"
        }),
        "the daemon's check finds main ahead: {view}"
    );
    assert_eq!(view["availability"]["ahead"], 2);
    assert_eq!(view["build"]["channel"], "nightly");
    // Not inside the desktop app, nothing to relaunch; the app built from the first commit would.
    assert_eq!(view["relaunch"], false);
    let first = setup.first.clone();
    assert_eq!(setup.web("GET", &format!("/api/update?desktop={first}")).1["relaunch"], true);
    assert_eq!(setup.web("POST", "/api/update/restart").0, "409", "nothing waits for a restart");

    let (status, started) = setup.web("POST", "/api/update/start");
    assert_eq!(status, "200", "{started}");
    assert!(
        common::eventually(|| {
            view = setup.web("GET", "/api/update").1;
            view["run"]["outcome"]["kind"] == "done"
        }),
        "the detached update finishes, and the restarted daemon reads it: {view}"
    );
    let newest = git(&setup.upstream, &["rev-parse", "HEAD"]);
    assert_eq!(git(&setup.clone, &["rev-parse", "HEAD"]), newest);
    assert_eq!(view["run"]["percent"], 100);
    assert!(view["output"].as_array().is_some_and(|lines| lines.iter().any(|line| line == "pnpm -C web build")), "{view}");
}
