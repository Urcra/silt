//! macOS's sandbox (ADR 0093): a Seatbelt profile applied by
//! `sandbox-exec`, from the same plan bubblewrap builds a root from on
//! Linux. The profile itself is written by `silt-workspace`.

use std::ffi::OsString;
use std::path::Path;

/// Seatbelt's front end, on every macOS.
pub const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Whether a profile can be applied here, with what went wrong when not.
pub fn check() -> Result<(), String> {
    if !Path::new(SANDBOX_EXEC).is_file() {
        return Err(format!("{SANDBOX_EXEC} isn't here"));
    }
    let output = std::process::Command::new(SANDBOX_EXEC)
        .args(["-p", "(version 1)(allow default)", "/usr/bin/true"])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    // An agent's own sandbox can't apply another inside it.
    Err(if said.is_empty() { format!("sandbox-exec exited with {}", output.status) } else { said })
}

/// `sandbox-exec`'s arguments to run `command` under the profile in
/// `profile`.
pub fn args(profile: &Path, command: &[OsString]) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["-f".into(), profile.into()];
    args.extend(command.iter().cloned());
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_follows_the_profile() {
        let args = args(Path::new("/r/seatbelt.sb"), &["claude".into(), "--resume".into()]);
        assert_eq!(args, ["-f", "/r/seatbelt.sb", "claude", "--resume"]);
    }
}
