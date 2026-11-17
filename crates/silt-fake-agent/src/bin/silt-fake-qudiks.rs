//! A stand-in for Qudiks, the Codex fork (`silt_fake_agent::codex`).

use std::process::ExitCode;

use silt_fake_agent::codex::{Fork, run};

fn main() -> ExitCode {
    silt_fake_agent::exit(run(&Fork { command: "qudiks", name: "Qudiks", homes: &["QUDIKS_HOME", "CODEX_HOME"] }))
}
