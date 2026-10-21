//! `silt server init`: nothing is written when it is refused, and replacing
//! a config keeps the settings its options don't cover.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn init(data: &Path, args: &[&str], password: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_silt"))
        .args(["server", "init", "--data"])
        .arg(data)
        .args(["--password-stdin"])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("runs");
    writeln!(child.stdin.take().expect("stdin"), "{password}").expect("writes");
    child.wait_with_output().expect("finishes")
}

#[test]
fn a_refused_init_writes_nothing_and_force_keeps_sign_in_settings() {
    let dir = tempfile::tempdir().expect("temp dir");
    let data = dir.path().join("server");
    let config = data.join("server.toml");

    assert!(!init(&data, &["--admin", "root"], "short").status.success(), "a weak password is refused");
    assert!(!config.exists(), "and leaves no config");
    assert!(!init(&data, &["--admin", "root", "--listen", "nowhere"], "correct horse battery").status.success());
    assert!(!config.exists());

    assert!(init(&data, &["--admin", "root"], "correct horse battery").status.success());
    let mut text = fs::read_to_string(&config).expect("reads");
    text = text.replace("passwords = true", "passwords = false");
    fs::write(&config, &text).expect("writes");

    let again = init(&data, &["--admin", "second", "--force", "--listen", "127.0.0.1:9999"], "correct horse battery");
    assert!(again.status.success(), "{}", String::from_utf8_lossy(&again.stderr));
    let replaced = fs::read_to_string(&config).expect("reads");
    assert!(replaced.contains("passwords = false"), "sign-in settings survive --force: {replaced}");
    assert!(replaced.contains("127.0.0.1:9999"), "{replaced}");
}
