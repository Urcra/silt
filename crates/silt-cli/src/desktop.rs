//! `silt open` across WSL and Windows (ADR 0045): in a
//! distribution it opens the Windows app on this distribution's backend,
//! and on Windows a folder inside a distribution opens there.

use std::path::Path;
use std::process::{Command, Stdio};

/// The `silt://` link that shows `project`, a directory in `distribution`,
/// in the Windows app. It names the place and nothing else: the app checks
/// both, then asks that distribution's `silt` for its own address.
pub fn wsl_link(distribution: &str, project: &str) -> String {
    format!("silt://open/?project={}&backend=wsl:{distribution}", encode(project))
}

fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => char::from(byte).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// Opens this directory of this distribution in the Windows app, when this
/// runs in WSL and the app is installed. Says whether it did.
pub fn open_from_wsl(cwd: &Path) -> bool {
    let Some(distribution) = silt_os::in_wsl() else {
        return false;
    };
    app_installed() && open_link(&wsl_link(&distribution, &cwd.to_string_lossy()))
}

/// Opens a folder inside a distribution, reached from Windows, in that
/// distribution's backend. Says whether it did.
pub fn open_wsl_folder(cwd: &Path) -> bool {
    let Some(inside) = silt_os::wsl_path(&cwd.to_string_lossy()) else {
        return false;
    };
    app_installed() && open_link(&wsl_link(&inside.distribution, &inside.linux))
}

/// Refuses to work on a folder inside a distribution from Windows: its files
/// belong to that distribution's own Silt.
pub fn refuse_wsl_folder(cwd: &Path) -> Result<(), String> {
    match silt_os::wsl_path(&cwd.to_string_lossy()) {
        Some(inside) => Err(format!(
            "{} is inside the WSL distribution {}; run silt there instead: wsl -d {} --cd {}",
            cwd.display(),
            inside.distribution,
            inside.distribution,
            inside.linux
        )),
        None => Ok(()),
    }
}

/// Whether the app registered `silt://` for this user, which its installer
/// does. From WSL this asks Windows through interop.
fn app_installed() -> bool {
    let reg = if cfg!(windows) { "reg" } else { "reg.exe" };
    quietly(Command::new(reg).args(["query", r"HKCU\Software\Classes\silt", "/ve"]))
}

/// Opens `link` with Windows' handler for its scheme. `explorer.exe` takes
/// it as one argument, without cmd's parsing of `&`, and answers 1 whether
/// or not it opened it, so starting it is all that can be checked.
fn open_link(link: &str) -> bool {
    Command::new("explorer.exe").arg(link).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok()
}

fn quietly(command: &mut Command) -> bool {
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_name_the_distribution_and_the_folder_only() {
        assert_eq!(wsl_link("Ubuntu", "/home/me/silt"), "silt://open/?project=/home/me/silt&backend=wsl:Ubuntu");
        assert_eq!(
            wsl_link("Ubuntu-24.04", "/home/me/a b&c=d#e"),
            "silt://open/?project=/home/me/a%20b%26c%3Dd%23e&backend=wsl:Ubuntu-24.04"
        );
    }

    #[test]
    fn folders_inside_a_distribution_are_refused_with_where_to_go() {
        let refused = refuse_wsl_folder(Path::new(r"\\wsl.localhost\Ubuntu\home\me")).expect_err("refused");
        assert!(refused.contains("wsl -d Ubuntu --cd /home/me"), "{refused}");
        assert!(refuse_wsl_folder(Path::new("/home/me")).is_ok());
    }
}
