//! macOS's sandbox (ADR 0086): a Seatbelt profile from the same plan
//! bubblewrap builds a root from. Seatbelt can't give the run a root of its
//! own, so the profile denies reading the home folder and Silt's data, then
//! allows back what the plan mounts; later rules win, as later mounts do.
//! Without a network namespace, "full internet" leaves only through Silt's
//! gateway, on a port of this machine's loopback.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::reach::Network;
use crate::sandbox::{MountKind, SandboxPlan};

/// What the profile needs beyond the plan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SeatbeltPaths {
    /// The person's home folder, unreadable but for what the plan mounts.
    pub home: Option<PathBuf>,
    /// The folder of everyone's homes, `/Users`.
    pub homes: Option<PathBuf>,
}

/// SBPL's string: quoted, with its own escapes.
fn quoted(path: &Path) -> String {
    let text = path.to_string_lossy();
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// What every process needs to start and run: its own processes, the
/// system's services, terminals and the null device.
const BASE: &str = r#"(version 1)
(deny default)
(allow process-exec process-fork)
(allow signal (target same-sandbox))
(allow process-info* (target same-sandbox))
(allow sysctl-read)
(allow user-preference-read)
(allow mach-lookup)
(allow ipc-posix-shm-read* ipc-posix-shm-write-data ipc-posix-sem)
(allow iokit-open)
(allow pseudo-tty)
(allow file-ioctl)
(allow file-read*)
(allow file-write*
  (literal "/dev/null")
  (literal "/dev/zero")
  (literal "/dev/tty")
  (literal "/dev/ptmx")
  (literal "/dev/dtracehelper")
  (regex #"^/dev/ttys[0-9]+$")
  (regex #"^/dev/fd/[0-9]+$"))
"#;

/// The profile for `plan`.
pub fn profile(plan: &SandboxPlan, paths: &SeatbeltPaths) -> String {
    let mut out = String::from(BASE);
    // Like the empty root bubblewrap starts from: nobody's home folder, and
    // the person's least of all, is read but for what the plan names.
    for hidden in [&paths.homes, &paths.home].into_iter().flatten() {
        let _ = writeln!(out, "(deny file-read* file-write* (subpath {}))", quoted(hidden));
    }
    for mount in &plan.mounts {
        let path = quoted(&mount.path);
        match mount.kind {
            MountKind::ReadOnly => {
                let _ = writeln!(out, "(allow file-read* (subpath {path}))");
                let _ = writeln!(out, "(deny file-write* (subpath {path}))");
            }
            MountKind::ReadWrite => {
                let _ = writeln!(out, "(allow file-read* file-write* (subpath {path}))");
            }
            MountKind::Device => {
                let _ = writeln!(out, "(allow file-read* file-write* file-ioctl (literal {path}))");
            }
            // The machine's own `/tmp` can't be swapped for an empty one:
            // the run gets `TMPDIR` in its scratch instead.
            MountKind::Tmpfs => {}
            MountKind::Hide => {
                let _ = writeln!(out, "(deny file-read* file-write* (subpath {path}))");
            }
        }
    }
    out.push_str(&network(plan));
    out
}

fn network(plan: &SandboxPlan) -> String {
    let mut out = String::new();
    for socket in &plan.sockets {
        let _ = writeln!(out, "(allow network-outbound (remote unix-socket (path-literal {})))", quoted(socket));
    }
    match plan.network {
        Network::None => {}
        Network::Gateway | Network::Full => {
            // Its own servers, such as a test's, on its loopback; reaching
            // this machine's others stays refused.
            out.push_str("(allow network-bind network-inbound (local ip \"localhost:*\"))\n");
            if let Some(port) = plan.gateway_port {
                let _ = writeln!(out, "(allow network-outbound (remote ip \"localhost:{port}\"))");
            }
            for port in &plan.ports {
                let _ = writeln!(out, "(allow network-outbound (remote ip \"localhost:{port}\"))");
            }
        }
        Network::Host => out.push_str("(allow network*)\n"),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reach::Reach;
    use crate::sandbox::{RunPaths, plan};

    fn run() -> (SandboxPlan, SeatbeltPaths) {
        let home = PathBuf::from("/Users/ada");
        let paths = RunPaths {
            home: Some(home.clone()),
            checkout: home.join("src/app"),
            agent_homes: vec![home.join(".claude")],
            silt: "/usr/local/bin/silt".into(),
            data_dir: Some(home.join("Library/Application Support/silt")),
            scratch: home.join("Library/Application Support/silt/runs/r1"),
            run_socket: Some(home.join("Library/Application Support/silt/agents.sock")),
            ..RunPaths::default()
        };
        let mut plan = plan(&paths, &Reach::default());
        plan.gateway_port = Some(41234);
        (plan, SeatbeltPaths { home: Some(home), homes: Some("/Users".into()) })
    }

    /// The rule that decides `path` for `operation`: Seatbelt's last match.
    fn decides(profile: &str, operation: &str, path: &str) -> Option<bool> {
        let mut decided = None;
        for line in profile.lines() {
            let allow = line.starts_with("(allow ");
            if !(allow || line.starts_with("(deny ")) || !line.contains(operation) {
                continue;
            }
            let matches = ["subpath", "literal"].iter().any(|kind| {
                line.split(&format!("({kind} \"")).skip(1).any(|rest| {
                    let target = rest.split('"').next().unwrap_or_default();
                    if *kind == "subpath" { Path::new(path).starts_with(target) } else { path == target }
                })
            });
            let everything = line == format!("(allow {operation})") || line.starts_with(&format!("(allow {operation})"));
            if matches || everything {
                decided = Some(allow);
            }
        }
        decided
    }

    #[test]
    fn the_home_is_closed_but_for_what_the_plan_mounts() {
        let (plan, paths) = run();
        let profile = profile(&plan, &paths);
        assert_eq!(decides(&profile, "file-read*", "/Users/ada/.ssh/id_ed25519"), Some(false));
        assert_eq!(decides(&profile, "file-read*", "/Users/ada/Documents/taxes.pdf"), Some(false));
        assert_eq!(decides(&profile, "file-read*", "/Users/bob/notes"), Some(false));
        assert_eq!(decides(&profile, "file-write*", "/Users/ada/src/app/main.rs"), Some(true));
        assert_eq!(decides(&profile, "file-write*", "/Users/ada/src/app/.git/config"), Some(false));
        assert_eq!(decides(&profile, "file-write*", "/Users/ada/src/app/.git/hooks/pre-commit"), Some(false));
        assert_eq!(decides(&profile, "file-read*", "/Users/ada/Library/Application Support/silt/silt.sqlite"), Some(false));
        assert_eq!(decides(&profile, "file-write*", "/Users/ada/Library/Application Support/silt/runs/r1/tmp/x"), Some(true));
        assert_eq!(decides(&profile, "file-read*", "/Users/ada/.cargo/bin/cargo"), Some(true));
        assert_eq!(decides(&profile, "file-write*", "/Users/ada/.cargo/bin/cargo"), Some(false));
        assert_eq!(decides(&profile, "file-read*", "/usr/bin/git"), Some(true));
        assert_eq!(decides(&profile, "file-write*", "/usr/bin/git"), Some(false));
        assert_eq!(decides(&profile, "file-write*", "/Applications/x"), None, "denied by default");
    }

    #[test]
    fn full_internet_leaves_through_the_gateway_only() {
        let (plan, paths) = run();
        let profile = profile(&plan, &paths);
        assert!(profile.contains("(allow network-outbound (remote ip \"localhost:41234\"))"));
        assert!(!profile.contains("(allow network*)"));
        assert!(profile.contains("agents.sock"));
        let host = SandboxPlan { network: Network::Host, ..plan.clone() };
        assert!(super::profile(&host, &paths).contains("(allow network*)"));
        let none = SandboxPlan { network: Network::None, ..plan };
        assert!(!super::profile(&none, &paths).contains("localhost:41234"));
    }

    #[test]
    fn a_path_cannot_break_out_of_its_string() {
        assert_eq!(quoted(Path::new("/a\"b\\c")), "\"/a\\\"b\\\\c\"");
        let (mut plan, paths) = run();
        plan.mounts.push(crate::sandbox::Mount {
            kind: MountKind::ReadWrite,
            path: "/x\")) (allow default) ((".into(),
            source: None,
        });
        assert!(!profile(&plan, &paths).contains("\n(allow default)"));
    }
}
