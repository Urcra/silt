//! `silt access doctor`: whether Silt's sandbox and its gateway work on this
//! machine, and how to fix what doesn't (ADR 0086).

use clap::Subcommand;
use exn::Exn;

use crate::CliError;

#[derive(Debug, Subcommand)]
pub enum AccessCommand {
    /// Check that Silt's sandbox can run here, and say how to fix what can't
    Doctor,
    /// What reached a thread's sandboxes through Silt: plugin calls, grants and what the gateway refused
    Show { thread: String },
}

pub fn run(command: AccessCommand) -> Result<(), Exn<CliError>> {
    match command {
        AccessCommand::Doctor => {
            doctor();
            Ok(())
        }
        AccessCommand::Show { thread } => crate::plugin::access(&thread),
    }
}

fn line(ok: bool, said: &str) {
    println!("{} {said}", if ok { "✓" } else { "✗" });
}

#[cfg(target_os = "linux")]
fn doctor() {
    use silt_os::sandbox;
    use silt_workspace::Network;

    match sandbox::tools(Network::None) {
        Ok(tools) => {
            line(true, &format!("bubblewrap: {}", tools.bwrap.display()));
            match sandbox::check(&tools) {
                Ok(()) => line(true, "a sandbox can be made here"),
                Err(error) => line(false, &error.to_string()),
            }
            match &tools.pasta {
                Some(pasta) => line(true, &format!("pasta, for full internet: {}", pasta.display())),
                None => line(false, "no pasta: full internet needs it (the `passt` package); other networks work without it"),
            }
        }
        Err(error) => line(false, &error.to_string()),
    }
    let restricted =
        std::fs::read_to_string("/proc/sys/kernel/apparmor_restrict_unprivileged_userns").is_ok_and(|value| value.trim() == "1");
    if restricted {
        println!(
            "! AppArmor restricts user namespaces here (Ubuntu 24.04 and later). If the sandbox can't be made, install bubblewrap's profile once, as root:\n    sudo cp /usr/share/apparmor/extra-profiles/bwrap-userns-restrict /etc/apparmor.d/ && sudo systemctl reload apparmor"
        );
    }
    if rustix::process::getuid().is_root() {
        println!(
            "! Silt runs as root: in some containers pasta then can't lock full internet down, and a workspace needs another network"
        );
    }
    credentials();
}

#[cfg(not(target_os = "linux"))]
fn doctor() {
    println!("Silt's own sandbox runs on Linux for now; here, a workspace that requires one uses the agent's own sandbox.");
    credentials();
}

fn credentials() {
    let data = silt_daemon::paths::data_dir();
    let all = silt_daemon::credentials::load(&data);
    let mut workspaces: Vec<&str> = all.iter().map(|stored| stored.workspace.as_str()).collect();
    workspaces.dedup();
    for workspace in workspaces {
        let (_, _, missing) = silt_daemon::credentials::resolve(&data, workspace);
        for stored in all.iter().filter(|stored| stored.workspace == workspace) {
            if missing.contains(&stored.name) {
                line(
                    false,
                    &format!("credential {}: its secret can't be had (the keyring, or its command, gave nothing)", stored.name),
                );
            } else {
                line(true, &format!("credential {} for {} in {workspace}", stored.name, stored.host));
            }
        }
    }
}
