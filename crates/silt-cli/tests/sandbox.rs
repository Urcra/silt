//! `silt sandbox`: reads anywhere, writes only where allowed, no network.

use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output};

fn sandboxed(args: &[&str], script: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_silt"))
        .arg("sandbox")
        .args(args)
        .args(["--", "sh", "-c", script])
        .output()
        .expect("silt runs")
}

/// Namespaces may be off on this machine; the tests then say so and pass,
/// unless `SILT_REQUIRE_SANDBOX` is set, as the gate does.
fn available() -> bool {
    let output = sandboxed(&[], "true");
    if !output.status.success() {
        assert!(
            std::env::var_os("SILT_REQUIRE_SANDBOX").is_none(),
            "the sandbox is required but unavailable: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        eprintln!("skipping: {}", String::from_utf8_lossy(&output.stderr));
    }
    output.status.success()
}

#[test]
fn writes_only_where_allowed() {
    if !available() {
        return;
    }
    let checkout = tempfile::tempdir().expect("tempdir");
    let elsewhere = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    std::fs::write(elsewhere.path().join("readme"), "read me").expect("writes");
    let script = format!(
        "echo made > {checkout}/made && cat {elsewhere}/readme && \
         ! touch {elsewhere}/nope 2>/dev/null && echo scratch > /tmp/scratch && cat /tmp/scratch",
        checkout = checkout.path().display(),
        elsewhere = elsewhere.path().display(),
    );
    let output = sandboxed(&["--write", &checkout.path().to_string_lossy()], &script);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "read mescratch\n");
    assert_eq!(std::fs::read_to_string(checkout.path().join("made")).expect("written"), "made\n");
    assert!(!elsewhere.path().join("nope").exists());
    assert!(!Path::new("/tmp/scratch").exists(), "the sandbox's /tmp is its own");
}

#[test]
fn has_no_network_unless_asked() {
    if !available() {
        return;
    }
    let listener = TcpListener::bind("127.0.0.1:0").expect("binds");
    let port = listener.local_addr().expect("address").port();
    let connect = format!("python3 -c 'import socket; socket.create_connection((\"127.0.0.1\", {port}), 2)'");
    assert!(!sandboxed(&[], &connect).status.success());
    let output = sandboxed(&["--net"], &connect);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn hides_sockets_and_credentials_and_keeps_git_data_read_only() {
    if !available() {
        return;
    }
    let checkout = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(checkout.path().join(".git/hooks")).expect("git dir");
    let silt_home = tempfile::tempdir().expect("tempdir");
    std::fs::write(silt_home.path().join("daemon.sock"), "").expect("stand-in socket");
    let script = format!(
        "test \"$(ls /run | wc -l)\" = 0 && ! test -e {home}/daemon.sock && test -z \"$SECRET_TOKEN\" \
         && test \"$$\" = 1 && ! touch {checkout}/.git/hooks/pre-commit 2>/dev/null && touch {checkout}/ok",
        home = silt_home.path().display(),
        checkout = checkout.path().display(),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_silt"))
        .args(["sandbox", "--write", &checkout.path().to_string_lossy(), "--", "sh", "-c", &script])
        .env("SILT_HOME", silt_home.path())
        .env("SECRET_TOKEN", "should-not-be-seen")
        .output()
        .expect("silt runs");
    assert!(output.status.success(), "{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(checkout.path().join("ok").exists());
    assert!(!checkout.path().join(".git/hooks/pre-commit").exists());
}

#[test]
fn what_the_command_starts_ends_with_it() {
    if !available() {
        return;
    }
    let marker = format!("silt-sandbox-test-{}", std::process::id());
    let output = sandboxed(&[], &format!("(exec -a {marker} sleep 300 &); exit 3"));
    assert_eq!(output.status.code(), Some(3), "the command's exit status comes through");
    std::thread::sleep(std::time::Duration::from_millis(300));
    let running = Command::new("ps").args(["-eo", "args"]).output().expect("ps");
    assert!(!String::from_utf8_lossy(&running.stdout).contains(&marker), "background work outlives the sandbox");
}

