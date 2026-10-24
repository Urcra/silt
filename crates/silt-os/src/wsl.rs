//! WSL seen from both sides: which distribution this runs in, and which
//! distribution a Windows path into one names (ADR 0045).

/// A path into a WSL distribution, as Windows names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WslPath {
    pub distribution: String,
    /// The same place inside the distribution, `/`-separated.
    pub linux: String,
}

/// The distribution this runs in, when it runs inside WSL and can start
/// Windows programs.
pub fn in_wsl() -> Option<String> {
    let distribution = std::env::var("WSL_DISTRO_NAME").ok().filter(|name| is_distribution_name(name))?;
    let interop = ["/proc/sys/fs/binfmt_misc/WSLInterop", "/proc/sys/fs/binfmt_misc/WSLInterop-late"];
    interop.iter().any(|path| std::path::Path::new(path).exists()).then_some(distribution)
}

/// The distribution and Linux path a `\\wsl.localhost\<distro>\…` or
/// `\\wsl$\<distro>\…` path names, with either slash and with or without
/// the `\\?\UNC\` prefix.
pub fn wsl_path(path: &str) -> Option<WslPath> {
    let normal = path.replace('\\', "/");
    let rest = normal.strip_prefix("//?/UNC/").or_else(|| normal.strip_prefix("//"))?;
    let (host, rest) = rest.split_once('/')?;
    if !host.eq_ignore_ascii_case("wsl.localhost") && !host.eq_ignore_ascii_case("wsl$") {
        return None;
    }
    let (distribution, inside) = rest.split_once('/').unwrap_or((rest, ""));
    if !is_distribution_name(distribution) {
        return None;
    }
    let components: Vec<&str> = inside.split('/').filter(|component| !component.is_empty() && *component != ".").collect();
    if components.contains(&"..") {
        return None;
    }
    Some(WslPath { distribution: distribution.to_owned(), linux: format!("/{}", components.join("/")) })
}

/// What `wsl.exe --install` accepts for a distribution's name, which is also
/// safe to pass to `wsl.exe -d` and put in a link.
pub fn is_distribution_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|char| char.is_ascii_alphanumeric() || matches!(char, '.' | '_' | '-'))
        && !name.starts_with(['-', '.'])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(distribution: &str, linux: &str) -> Option<WslPath> {
        Some(WslPath { distribution: distribution.to_owned(), linux: linux.to_owned() })
    }

    #[test]
    fn windows_paths_into_a_distribution_name_it_and_the_linux_path() {
        assert_eq!(wsl_path(r"\\wsl.localhost\Ubuntu\home\me\silt"), at("Ubuntu", "/home/me/silt"));
        assert_eq!(wsl_path(r"\\wsl$\Ubuntu-24.04\home\me\"), at("Ubuntu-24.04", "/home/me"));
        assert_eq!(wsl_path("//WSL.LOCALHOST/Debian/srv"), at("Debian", "/srv"));
        assert_eq!(wsl_path(r"\\?\UNC\wsl.localhost\Ubuntu\home"), at("Ubuntu", "/home"));
        assert_eq!(wsl_path(r"\\wsl.localhost\Ubuntu"), at("Ubuntu", "/"));
        assert_eq!(wsl_path(r"\\wsl.localhost\Ubuntu\.\home\\me"), at("Ubuntu", "/home/me"));
    }

    #[test]
    fn other_paths_and_odd_names_are_not_wsl_paths() {
        for path in [
            r"C:\Users\me",
            r"\\server\share\x",
            r"\\wsl.localhost\",
            r"\\wsl.localhost\-d\home",
            r"\\wsl.localhost\a b\home",
            r"\\wsl.localhost\Ubuntu\home\..\..\etc",
            "/home/me",
            "",
        ] {
            assert_eq!(wsl_path(path), None, "{path}");
        }
    }

    #[test]
    fn distribution_names_are_safe_as_arguments() {
        assert!(is_distribution_name("Ubuntu-24.04"));
        assert!(is_distribution_name("my_distro"));
        for name in ["", "-d", ".hidden", "a b", "a&b", "a\"b", "a/b", &"x".repeat(65)] {
            assert!(!is_distribution_name(name), "{name}");
        }
    }
}
