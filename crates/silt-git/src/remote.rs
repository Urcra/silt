/// A remote URL reduced to `host/path`, so the SSH and HTTPS forms of one
/// repository compare equal: `git@GitHub.com:Urcra/silt.git` and
/// `https://github.com/Urcra/silt` both become `github.com/Urcra/silt`.
pub fn normalize_remote(url: &str) -> String {
    let url = url.trim();
    let without_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let without_user = without_scheme
        .split_once('@')
        .filter(|(user, _)| !user.contains('/'))
        .map_or(without_scheme, |(_, rest)| rest);
    let (host, path) = match without_user.split_once('/') {
        Some((host, path)) if !host.contains(':') || url.contains("://") => (host, path),
        _ => without_user.split_once(':').unwrap_or((without_user, "")),
    };
    let host = host.split(':').next().unwrap_or(host).to_lowercase();
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if path.is_empty() {
        host
    } else {
        format!("{host}/{path}")
    }
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
    fn local_paths_keep_their_path() {
        assert_eq!(normalize_remote("/srv/git/project.git"), "/srv/git/project");
        assert_eq!(normalize_remote("file:///srv/git/project.git"), "/srv/git/project");
    }
}
