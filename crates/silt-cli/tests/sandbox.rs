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

/// Namespaces may be off on this machine; the tests then say so and pass.
fn available() -> bool {
    let output = sandboxed(&[], "true");
    if !output.status.success() {
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
