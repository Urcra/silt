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

/// `path` without the Windows drives WSL appends to it, `/mnt/<drive>/…`:
/// what's there is a Windows program, and each look on those drives
/// crosses into Windows, slowly. Elsewhere `path` as it is.
pub fn linux_path(path: &std::ffi::OsStr) -> std::ffi::OsString {
    if in_wsl().is_none() {
        return path.to_owned();
    }
    without_windows_drives(path)
}

fn without_windows_drives(path: &std::ffi::OsStr) -> std::ffi::OsString {
    let on_a_drive = |dir: &std::path::Path| {
        let mut components = dir.components().skip_while(|component| *component == std::path::Component::RootDir);
        components.next().is_some_and(|first| first.as_os_str() == "mnt")
            && components
                .next()
                .and_then(|drive| drive.as_os_str().to_str())
                .is_some_and(|drive| drive.len() == 1 && drive.chars().all(|letter| letter.is_ascii_alphabetic()))
    };
    std::env::join_paths(std::env::split_paths(path).filter(|dir| !on_a_drive(dir))).unwrap_or_else(|_| path.to_owned())
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

    #[cfg(unix)]
    #[test]
    fn the_windows_drives_leave_the_path() {
        let path = std::ffi::OsStr::new("/home/me/.local/bin:/mnt/c/Program Files/nodejs:/usr/bin:/mnt/d/tools:/mnt/data/bin");
        assert_eq!(without_windows_drives(path), "/home/me/.local/bin:/usr/bin:/mnt/data/bin");
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
