//! A stand-in for Codex (`silt_fake_agent::codex`).

use std::process::ExitCode;

use silt_fake_agent::codex::{Fork, run};

fn main() -> ExitCode {
    silt_fake_agent::exit(run(&Fork { command: "codex", name: "Codex", homes: &["CODEX_HOME"] }))
}
