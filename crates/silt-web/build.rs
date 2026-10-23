//! Builds the web UI into `web/dist` when it isn't there, so that `cargo
//! install --git` gives the whole app (ADR 0042). A checkout that has it,
//! as the gate's and the release workflow's do, is left as it is.
//! `SILT_SKIP_WEB_BUILD=1` builds `silt` without the UI.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// silt-wasm's, which the browser replica's bindings must match.
const WASM_BINDGEN: &str = "0.2.126";

fn main() {
    let web = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it")).join("../../web");
    let index = web.join("dist/index.html");
    println!("cargo::rerun-if-changed={}", index.display());
    println!("cargo::rerun-if-env-changed=SILT_SKIP_WEB_BUILD");
    if index.exists() || env::var_os("SILT_SKIP_WEB_BUILD").is_some_and(|skip| skip != "0") {
        return;
    }
    if let Err(missing) = check_tools() {
        panic!(
            "the web UI isn't built and can't be: {missing}.\n\
             Building it needs Node.js with pnpm, wasm-bindgen {WASM_BINDGEN} \
             (`cargo install wasm-bindgen-cli --version {WASM_BINDGEN}`) and the \
             wasm32-unknown-unknown target (`rustup target add wasm32-unknown-unknown`). \
             Or install a release binary (scripts/install.sh), or set SILT_SKIP_WEB_BUILD=1 \
             for a silt without the web UI."
        );
    }
    // Its own target dir: the cargo running this build holds the lock on
    // the outer one, and would wait on it forever.
    let wasm_target = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets it")).join("wasm");
    run(Command::new("pnpm").args(["install", "--frozen-lockfile"]).current_dir(&web));
    run(Command::new("pnpm")
        .arg("build")
        .current_dir(&web)
        .env("CARGO_TARGET_DIR", &wasm_target)
        // Flags for this build's target, which isn't wasm.
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_BUILD_TARGET"));
    assert!(index.exists(), "the web UI built, but {} isn't there", index.display());
}

fn check_tools() -> Result<(), String> {
    let output = |program: &str, args: &[&str]| {
        Command::new(program)
            .args(args)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    output("pnpm", &["--version"]).ok_or("pnpm isn't installed")?;
    let bindgen = output("wasm-bindgen", &["--version"]).ok_or("wasm-bindgen isn't installed")?;
    if !bindgen.ends_with(WASM_BINDGEN) {
        return Err(format!("wasm-bindgen is {bindgen}, not {WASM_BINDGEN}"));
    }
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let sysroot = output(&rustc, &["--print", "sysroot"]).ok_or("rustc can't be run")?;
    if !Path::new(&sysroot).join("lib/rustlib/wasm32-unknown-unknown").is_dir() {
        return Err("the wasm32-unknown-unknown target isn't installed".to_owned());
    }
    Ok(())
}

fn run(command: &mut Command) {
    let status = command.status().unwrap_or_else(|error| panic!("could not run {command:?}: {error}"));
    assert!(status.success(), "{command:?} failed: {status}");
}
