//! `silt access doctor`: whether Silt's sandbox and its gateway work on this
//! machine, and how to fix what doesn't (ADR 0093).

use clap::Subcommand;
use exn::Exn;

use crate::CliError;

#[derive(Debug, Subcommand)]
pub enum AccessCommand {
    /// Check that Silt's sandbox can run here, and say how to fix what can't
    Doctor,
    /// What reached a thread's sandboxes through Silt: plugin calls, grants and what the gateway refused
    Show { thread: String },
    /// Let go of a device relayed to a thread's sandboxes, ending its session
    Release { thread: String, device: String },
    /// Take a folder back from a thread's agents; they lose it when their turn ends
    Unfolder { thread: String, path: String },
}

pub fn run(command: AccessCommand) -> Result<(), Exn<CliError>> {
    match command {
        AccessCommand::Doctor => {
            doctor();
            Ok(())
        }
        AccessCommand::Show { thread } => crate::plugin::access(&thread),
        AccessCommand::Unfolder { thread, path } => {
            let said: String = crate::commands::call(&silt_daemon::protocol::Request::FolderRevoke { thread, path })?;
            println!("{said}");
            Ok(())
        }
        AccessCommand::Release { thread, device } => {
            let said: String = crate::commands::call(&silt_daemon::protocol::Request::ReleaseDevice { thread, device })?;
            println!("{said}");
            Ok(())
        }
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

#[cfg(target_os = "macos")]
fn doctor() {
    match silt_os::seatbelt::check() {
        Ok(()) => println!("✓ Seatbelt (sandbox-exec) applies a profile here"),
        Err(error) => println!(
            "✗ Seatbelt can't apply a profile here: {error}. Inside another sandbox, an agent's own included, it can't nest; run Silt outside it"
        ),
    }
    println!(
        "! On macOS full internet goes through Silt's gateway, as HTTP(S) only, and agents' own sandboxes can't run inside Silt's"
    );
    credentials();
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn doctor() {
    println!(
        "Silt's own sandbox runs on Linux and macOS; here, agents don't start in a workspace that runs them in it, and the agent's own sandbox is what a workspace can require."
    );
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
