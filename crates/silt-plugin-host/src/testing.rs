//! For tests: plugins from `plugins/` built for `wasm32-wasip2`, once per
//! test run.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The component of `package` in `plugins/`, built once.
pub fn plugin(package: &str) -> Vec<u8> {
    static BUILT: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();
    let built = BUILT.get_or_init(Default::default);
    let mut built = built.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(wasm) = built.get(package) {
        return wasm.clone();
    }
    let target = repo().join("target/plugins");
    let status = std::process::Command::new(env!("CARGO"))
        .args(["build", "--release", "--target", "wasm32-wasip2", "-p", package, "--manifest-path"])
        .arg(repo().join("plugins/Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .status()
        .expect("cargo runs");
    assert!(status.success(), "{package} builds");
    let wasm = std::fs::read(target.join("wasm32-wasip2/release").join(format!("{}.wasm", package.replace('-', "_"))))
        .expect("its component");
    built.insert(package.to_owned(), wasm.clone());
    wasm
}

/// The probe's manifest, which the host's and the daemon's tests share.
pub const PROBE_MANIFEST: &str = r#"
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

/// The local probe's manifest, its MCP server started by `mcp`: TCP to
/// this machine's loopback, SSH to a test fleet, and the server.
pub fn probe_local_manifest(mcp: &str) -> String {
    format!(
        r#"
id = "test.probe-local"
name = "Local probe"
version = "1.0.0"
publisher = "test"
description = "Reaches what Silt can't check."
license = "AGPL-3.0-only"
world = "silt:plugin@1"
build = {{ cargo = {{ package = "silt-plugin-probe-local", toolchain = "1.99.1" }} }}

[[capabilities]]
kind = "sockets"
hosts = ["127.0.0.1"]
reason = "Talks to a service on this machine."

[[capabilities]]
kind = "ssh"
hosts = ["fleet.example.com"]
reason = "Runs a command on the fleet."

[[capabilities]]
kind = "mcp-process"
command = [{mcp:?}]
hosts = []
reason = "Wraps a local MCP server."

[[actions]]
id = "reach"
label = "Reach"
description = "Reaches something Silt can't check."
kind = "change"
default = "ask"
"#
    )
}
