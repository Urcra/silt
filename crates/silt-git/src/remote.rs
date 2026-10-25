/// A remote URL reduced to `host/path`, so the SSH and HTTPS forms of one
/// repository compare equal: `git@GitHub.com:Urcra/silt.git` and
/// `https://github.com/Urcra/silt` both become `github.com/Urcra/silt`.
pub fn normalize_remote(url: &str) -> String {
    let url = url.trim();
    let without_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    if let Some(path) = drive_path(without_scheme) {
        return path;
    }
    let without_user =
        without_scheme.split_once('@').filter(|(user, _)| !user.contains('/')).map_or(without_scheme, |(_, rest)| rest);
    let (host, path) = match without_user.split_once('/') {
        Some((host, path)) if !host.contains(':') || url.contains("://") => (host, path),
        _ => without_user.split_once(':').unwrap_or((without_user, "")),
    };
    let host = host.split(':').next().unwrap_or(host).to_lowercase();
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if path.is_empty() { host } else { format!("{host}/{path}") }
}

/// A path on a Windows drive as WSL names it, so a clone on Windows and
/// one in a distribution of the same repository agree: `C:\src\app.git`
/// and `C:/src/app.git` become `/mnt/c/src/app`, as `/mnt/c/src/app.git`
/// does (ADR 0045). Without this, `C:` would read as an SSH host.
fn drive_path(path: &str) -> Option<String> {
    // `file:///C:/src` leaves a slash before the drive.
    let path = path.strip_prefix('/').filter(|rest| rest.as_bytes().get(1) == Some(&b':')).unwrap_or(path);
    let [drive, b':', rest @ ..] = path.as_bytes() else {
        return None;
    };
    if !drive.is_ascii_alphabetic() || !matches!(rest.first(), None | Some(b'/' | b'\\')) {
        return None;
    }
    let rest = path[2..].replace('\\', "/");
    let rest = rest.trim_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    Some(format!("/mnt/{}/{rest}", drive.to_ascii_lowercase() as char).trim_end_matches('/').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_and_https_forms_agree() {
        for url in [
            "git@github.com:Urcra/silt.git",
            "git@GitHub.com:Urcra/silt",
            "https://github.com/Urcra/silt",
            "https://github.com/Urcra/silt.git/",
            "ssh://git@github.com/Urcra/silt.git",
            "ssh://git@github.com:22/Urcra/silt.git",
            "https://user@github.com/Urcra/silt.git",
        ] {
            assert_eq!(normalize_remote(url), "github.com/Urcra/silt", "{url}");
        }
    }

    #[test]
    fn a_windows_drive_path_agrees_with_wsl_s_name_for_it() {
        for url in [r"C:\src\app.git", "C:/src/app.git", "c:/src/app/", "file:///C:/src/app.git", "/mnt/c/src/app.git"] {
            assert_eq!(normalize_remote(url), "/mnt/c/src/app", "{url}");
        }
        assert_eq!(normalize_remote("c:src/app"), "c/src/app", "a one-letter host is still a host");
    }

    #[test]
    fn local_paths_keep_their_path() {
        assert_eq!(normalize_remote("/srv/git/project.git"), "/srv/git/project");
        assert_eq!(normalize_remote("file:///srv/git/project.git"), "/srv/git/project");
    }
}
