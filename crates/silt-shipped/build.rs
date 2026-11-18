//! Builds the plugins shipped with Silt (`plugins/`) for `wasm32-wasip2`,
//! for `silt` to embed (ADR 0100). `SILT_SKIP_PLUGIN_BUILD=1` builds Silt
//! without them.

use std::env;
use std::path::PathBuf;
use std::process::Command;

/// The packages shipped, by the name of their folder in `plugins/`.
const SHIPPED: [&str; 2] = ["folders", "github"];

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it")).join("../../plugins");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets it"));
    println!("cargo::rerun-if-env-changed=SILT_SKIP_PLUGIN_BUILD");
    println!("cargo::rerun-if-changed={}", root.join("Cargo.toml").display());
    println!("cargo::rerun-if-changed={}", root.join("Cargo.lock").display());
    println!("cargo::rerun-if-changed={}", root.join("../crates/silt-plugin-api").display());
    println!("cargo::rerun-if-changed={}", root.join("../crates/silt-plugin/src").display());
    let skip = env::var_os("SILT_SKIP_PLUGIN_BUILD").is_some_and(|skip| skip != "0");
    let mut command = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command
        .args(["build", "--release", "--locked", "--target", "wasm32-wasip2", "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        // Its own target dir: the cargo running this build holds the lock on
        // the outer one.
        .arg("--target-dir")
        .arg(out.join("target"))
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_BUILD_TARGET")
        .env_remove("RUSTFLAGS");
    for name in SHIPPED {
        println!("cargo::rerun-if-changed={}", root.join(name).display());
        command.args(["-p", &format!("silt-plugin-{name}")]);
    }
    let built = !skip && {
        let status = command.status().expect("cargo runs");
        assert!(
            status.success(),
            "the plugins shipped with Silt didn't build; SILT_SKIP_PLUGIN_BUILD=1 builds Silt without them"
        );
        true
    };
    for name in SHIPPED {
        let wasm = out.join("target/wasm32-wasip2/release").join(format!("silt_plugin_{name}.wasm"));
        let embedded = out.join(format!("{name}.wasm"));
        if built {
            std::fs::copy(&wasm, &embedded).expect("the built component");
        } else {
            std::fs::write(&embedded, b"").expect("OUT_DIR is writable");
        }
    }
}
