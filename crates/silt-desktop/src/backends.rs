//! The daemons one window can show (ADR 0045): this machine's, and on
//! Windows each WSL distribution's, reached through `wsl.exe`.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use tauri::Url;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Backend {
    /// The `silt` installed beside the app.
    #[default]
    Native,
    /// The `silt` inside a WSL distribution, by its name.
    Wsl(String),
}

impl Backend {
    /// `native`, or `wsl:<distribution>` as links name them.
    pub fn parse(text: &str) -> Option<Self> {
        match text.strip_prefix("wsl:") {
            Some(name) if is_distribution_name(name) => Some(Self::Wsl(name.to_owned())),
            Some(_) => None,
            None => (text == "native").then_some(Self::Native),
        }
    }

    pub fn id(&self) -> String {
        match self {
            Self::Native => "native".to_owned(),
            Self::Wsl(name) => format!("wsl:{name}"),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Native => "This computer".to_owned(),
            Self::Wsl(name) => format!("WSL: {name}"),
        }
    }
}

/// What the window shows: a backend, and the directory there whose project
/// `silt open` picks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Target {
    pub backend: Backend,
    pub project: Option<String>,
}

/// What a `silt://open/?project=<dir>&backend=<backend>` link asks for, as
/// `silt open` in WSL sends it. A web page can open such a link too, so it
/// only chooses among the backends the app knows, and a directory, which
/// that backend's `silt` checks like any other.
pub fn target_of(link: &Url, known: &[Backend]) -> Option<Target> {
    if link.scheme() != "silt" || link.host_str() != Some("open") || !matches!(link.path(), "" | "/") {
        return None;
    }
    let mut target = Target::default();
    for (key, value) in link.query_pairs() {
        match &*key {
            "backend" => target.backend = Backend::parse(&value).filter(|backend| known.contains(backend))?,
            "project" => target.project = Some(value.into_owned()),
            _ => {}
        }
    }
    let plain = |project: &String| {
        project.len() <= 4096 && !project.chars().any(char::is_control) && !project.split(['/', '\\']).any(|part| part == "..")
    };
    if !target.project.as_ref().is_none_or(plain) {
        return None;
    }
    Some(target)
}

/// What a distribution's name may be, which is also what's safe to hand to
/// `wsl.exe -d`.
pub fn is_distribution_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|char| char.is_ascii_alphanumeric() || matches!(char, '.' | '_' | '-'))
        && !name.starts_with(['-', '.'])
}

/// The distributions `wsl.exe --list --quiet` printed, in UTF-16 as it
/// writes.
pub fn distributions(output: &[u8]) -> Vec<String> {
    let units: Vec<u16> = output.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
    String::from_utf16_lossy(&units)
        .lines()
        .map(|line| line.trim_matches(|char: char| char.is_whitespace() || char == '\u{feff}' || char == '\0'))
        .filter(|name| is_distribution_name(name))
        .map(str::to_owned)
        .collect()
}

/// Looks for `silt` in a distribution through a login shell, once, so the
/// commands after never depend on shell startup files.
pub fn find_silt(distribution: &str) -> Command {
    let mut command = Command::new("wsl.exe");
    command.args(["-d", distribution, "--exec", "sh", "-lc", "command -v silt"]);
    command
}

/// The absolute path `find_silt` printed.
pub fn found_silt(stdout: &str) -> Option<String> {
    let line = stdout.lines().map(str::trim).rfind(|line| !line.is_empty())?;
    (line.starts_with('/') && !line.chars().any(char::is_control)).then(|| line.to_owned())
}

/// Each distribution's `silt` as last found, from the file the app keeps
/// them in. Entries that aren't a distribution and an absolute path are
/// left out: the file is the person's to edit.
pub fn remembered_silts(text: &str) -> HashMap<String, String> {
    let Ok(serde_json::Value::Object(entries)) = serde_json::from_str(text) else {
        return HashMap::new();
    };
    entries
        .into_iter()
        .filter_map(|(distribution, path)| Some((distribution, found_silt(path.as_str()?)?)))
        .filter(|(distribution, _)| is_distribution_name(distribution))
        .collect()
}

pub fn remember_silts(silts: &HashMap<String, String>) -> String {
    let sorted: std::collections::BTreeMap<_, _> = silts.iter().collect();
    serde_json::to_string_pretty(&sorted).expect("strings serialize")
}

/// Runs `"$0" "$@"` in the person's own shell, as a login and interactive one,
/// so a daemon it starts finds the agents their terminal does: `wsl.exe
/// --exec` alone gives the distribution's bare `PATH`, without `~/.local/bin`
/// or nvm's node. Shells that don't take `$0` and `$@` so get `sh -l`.
const IN_THEIR_SHELL: &str = r#"case "${SHELL##*/}" in bash|zsh|ksh|dash|sh) shell=$SHELL ;; *) shell=/bin/sh ;; esac
exec "$shell" -lic 'exec "$0" "$@"' "$0" "$@""#;

/// The command that attaches to the backend's daemon, starting it if none
/// runs, and prints its web UI's address: in the project's directory, or
/// in `/`, not a clone of any repository.
pub fn open_web(target: &Target, native: &Path, wsl_silt: &str) -> Command {
    let mut command = match &target.backend {
        Backend::Native => {
            let mut command = Command::new(native);
            command.current_dir(target.project.as_deref().unwrap_or("/"));
            command
        }
        Backend::Wsl(distribution) => {
            let mut command = Command::new("wsl.exe");
            command.args(["-d", distribution, "--cd", target.project.as_deref().unwrap_or("/"), "--exec", "sh", "-c"]);
            command.args([IN_THEIR_SHELL, wsl_silt]);
            command
        }
    };
    command.args(["open", "--no-browser"]);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(text: &str) -> Url {
        Url::parse(text).expect("a link")
    }

    #[test]
    fn links_pick_a_known_backend_and_a_directory() {
        let known = [Backend::Native, Backend::Wsl("Ubuntu".into())];
        assert_eq!(
            target_of(&link("silt://open/?project=/home/me/a%20b&backend=wsl:Ubuntu"), &known),
            Some(Target { backend: Backend::Wsl("Ubuntu".into()), project: Some("/home/me/a b".into()) })
        );
        assert_eq!(target_of(&link("silt://open/"), &known), Some(Target::default()));
        assert_eq!(target_of(&link("silt://open/?backend=wsl:Debian"), &known), None, "not a distribution here");
        assert_eq!(target_of(&link("silt://open/?backend=wsl:-d"), &known), None);
        assert_eq!(target_of(&link("silt://open/?project=/home/../etc&backend=native"), &known), None);
        assert_eq!(target_of(&link("silt://open/?project=/a%0Ab"), &known), None);
        assert_eq!(target_of(&link("silt://elsewhere/?backend=native"), &known), None);
        assert_eq!(target_of(&link("https://open/?backend=native"), &known), None);
        assert_eq!(target_of(&link("silt://open/t/abc"), &known), None, "permalinks are for later");
    }

    #[test]
    fn backends_round_trip_through_their_ids() {
        for backend in [Backend::Native, Backend::Wsl("Ubuntu-24.04".into())] {
            assert_eq!(Backend::parse(&backend.id()), Some(backend));
        }
        assert_eq!(Backend::parse("wsl:"), None);
        assert_eq!(Backend::parse("local"), None);
    }

    #[test]
    fn distributions_are_read_from_wsl_s_utf16() {
        let output: Vec<u8> = "\u{feff}Ubuntu\r\nDebian\r\n\r\nbad name\r\n".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(distributions(&output), ["Ubuntu", "Debian"]);
        assert!(distributions(&[]).is_empty());
    }

    #[test]
    fn silt_is_found_by_its_absolute_path() {
        assert_eq!(found_silt("motd\n/home/me/.local/bin/silt\n"), Some("/home/me/.local/bin/silt".into()));
        assert_eq!(found_silt("silt\n"), None, "an alias isn't a path");
        assert_eq!(found_silt(""), None);
    }

    #[test]
    fn distributions_silts_are_remembered_and_what_isnt_one_is_dropped() {
        let silts = HashMap::from([("Ubuntu".to_owned(), "/home/me/.local/bin/silt".to_owned())]);
        assert_eq!(remembered_silts(&remember_silts(&silts)), silts);
        let edited = r#"{"Ubuntu": "/usr/bin/silt", "-d": "/x", "Debian": "silt", "Arch": 3}"#;
        assert_eq!(remembered_silts(edited), HashMap::from([("Ubuntu".to_owned(), "/usr/bin/silt".to_owned())]));
        assert!(remembered_silts("not json").is_empty());
    }

    #[test]
    fn a_distribution_s_silt_runs_in_the_project_in_the_person_s_shell() {
        let target = Target { backend: Backend::Wsl("Ubuntu".into()), project: Some("/home/me/repo".into()) };
        let command = open_web(&target, Path::new("silt"), "/usr/local/bin/silt");
        let args: Vec<_> = command.get_args().map(|arg| arg.to_string_lossy().into_owned()).collect();
        assert_eq!(command.get_program(), "wsl.exe");
        assert_eq!(
            args,
            [
                "-d",
                "Ubuntu",
                "--cd",
                "/home/me/repo",
                "--exec",
                "sh",
                "-c",
                IN_THEIR_SHELL,
                "/usr/local/bin/silt",
                "open",
                "--no-browser"
            ]
        );
        let native = open_web(&Target::default(), Path::new("/opt/silt"), "silt");
        assert_eq!(native.get_program(), "/opt/silt");
        assert_eq!(native.get_current_dir(), Some(Path::new("/")));
    }

    #[cfg(unix)]
    #[test]
    fn the_person_s_shell_runs_the_program_with_its_arguments_as_they_are() {
        for shell in ["/bin/sh", "/usr/bin/fish"] {
            let output = Command::new("sh")
                .args(["-c", IN_THEIR_SHELL, "printf", "[%s]", "a b", "$HOME", "'"])
                .env("SHELL", shell)
                .env("HOME", "/nonexistent")
                .stdin(std::process::Stdio::null())
                .output()
                .expect("sh runs");
            assert_eq!(String::from_utf8_lossy(&output.stdout), "[a b][$HOME][']", "with SHELL={shell}");
        }
    }
}
