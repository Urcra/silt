//! The daemons one window can show (ADR 0045): this machine's, on Windows
//! each WSL distribution's, reached through `wsl.exe`, and the machines the
//! person added, reached over SSH (ADR 0108).

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use tauri::Url;

use crate::machines;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Backend {
    /// The `silt` installed beside the app.
    #[default]
    Native,
    /// The `silt` inside a WSL distribution, by its name.
    Wsl(String),
    /// The `silt` on a machine reached over SSH, by its name.
    Ssh(String),
}

impl Backend {
    /// `native`, `wsl:<distribution>` or `ssh:<machine>` as links name them.
    pub fn parse(text: &str) -> Option<Self> {
        if let Some(name) = text.strip_prefix("wsl:") {
            return is_distribution_name(name).then(|| Self::Wsl(name.to_owned()));
        }
        if let Some(name) = text.strip_prefix("ssh:") {
            return machines::is_machine_name(name).then(|| Self::Ssh(name.to_owned()));
        }
        (text == "native").then_some(Self::Native)
    }

    pub fn id(&self) -> String {
        match self {
            Self::Native => "native".to_owned(),
            Self::Wsl(name) => format!("wsl:{name}"),
            Self::Ssh(name) => format!("ssh:{name}"),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Native => "This computer".to_owned(),
            Self::Wsl(name) => format!("WSL: {name}"),
            Self::Ssh(name) => name.clone(),
        }
    }

    /// `native`, `wsl` or `ssh`, as the page tells them apart.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Wsl(_) => "wsl",
            Self::Ssh(_) => "ssh",
        }
    }
}

/// What the window shows: a backend, the directory there whose project
/// `silt open` picks, and the page to open on it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Target {
    pub backend: Backend,
    pub project: Option<String>,
    pub page: Option<Page>,
}

/// A page on another machine the window opens once it shows it (ADR 0132).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Page {
    /// A thread there, by its id.
    Thread(String),
    /// New thread there, with what was typed and the project's origin.
    New { draft: Option<String>, origin: Option<String> },
}

impl Page {
    /// The page's path and query on the backend's UI.
    pub fn path(&self) -> (String, Option<String>) {
        match self {
            Self::Thread(id) => (format!("/t/{id}"), None),
            Self::New { draft, origin } => {
                let mut query = url::form_urlencoded::Serializer::new(String::new());
                if let Some(origin) = origin {
                    query.append_pair("origin", origin);
                }
                if let Some(draft) = draft {
                    query.append_pair("draft", draft);
                }
                let query = query.finish();
                ("/new".to_owned(), (!query.is_empty()).then_some(query))
            }
        }
    }
}

/// A draft carried to another machine's New thread: text, kept short enough for a link.
fn is_draft(text: &str) -> bool {
    text.len() <= 16 * 1024 && !text.chars().any(|char| char.is_control() && !matches!(char, '\n' | '\t'))
}

/// A project's origin, as `silt` normalizes it.
fn is_origin(text: &str) -> bool {
    !text.is_empty() && text.len() <= 1024 && !text.chars().any(char::is_control)
}

/// What a `silt://open/?project=<dir>&backend=<backend>` link asks for, as
/// `silt open` in WSL sends it. A web page can open such a link too, so it
/// only chooses among the backends the app knows, and a directory, which
/// that backend's `silt` checks like any other. No `silt open` on a machine
/// over SSH reaches this window, so none picks a project there.
pub fn target_of(link: &Url, known: &[Backend]) -> Option<Target> {
    if link.scheme() != "silt" || link.host_str() != Some("open") || !matches!(link.path(), "" | "/") {
        return None;
    }
    let mut target = Target::default();
    for (key, value) in link.query_pairs() {
        match &*key {
            "backend" => target.backend = Backend::parse(&value).filter(|backend| known.contains(backend))?,
            "project" => target.project = Some(value.into_owned()),
            "thread" => target.page = Some(Page::Thread(value.into_owned())),
            "new" if value == "1" => {
                let field = |key: &str| link.query_pairs().find(|(name, _)| name == key).map(|(_, value)| value.into_owned());
                target.page = Some(Page::New { draft: field("draft"), origin: field("origin") });
            }
            _ => {}
        }
    }
    let plain = |project: &String| {
        project.len() <= 4096 && !project.chars().any(char::is_control) && !project.split(['/', '\\']).any(|part| part == "..")
    };
    if !target.project.as_ref().is_none_or(plain) || matches!(target.backend, Backend::Ssh(_)) && target.project.is_some() {
        return None;
    }
    let fine = match &target.page {
        None => true,
        Some(Page::Thread(id)) => crate::elsewhere::is_thread_id(id),
        Some(Page::New { draft, origin }) => draft.as_deref().is_none_or(is_draft) && origin.as_deref().is_none_or(is_origin),
    };
    if !fine || target.page.is_some() && target.project.is_some() {
        return None;
    }
    Some(target)
}

/// Whether `link` asks the app to restart itself, as the page does once an
/// update installed a new build of it.
pub fn is_relaunch(link: &Url) -> bool {
    link.scheme() == "silt" && link.host_str() == Some("relaunch") && matches!(link.path(), "" | "/")
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
/// writes, without Docker Desktop's own, which run its engine rather than
/// a person's work.
pub fn distributions(output: &[u8]) -> Vec<String> {
    let units: Vec<u16> = output.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
    String::from_utf16_lossy(&units)
        .lines()
        .map(|line| line.trim_matches(|char: char| char.is_whitespace() || char == '\u{feff}' || char == '\0'))
        .filter(|name| is_distribution_name(name) && !name.starts_with("docker-desktop"))
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
pub const IN_THEIR_SHELL: &str = r#"case "${SHELL##*/}" in bash|zsh|ksh|dash|sh) shell=$SHELL ;; *) shell=/bin/sh ;; esac
exec "$shell" -lic 'exec "$0" "$@"' "$0" "$@""#;

/// The command that attaches to the backend's daemon, starting it if none
/// runs, and prints its web UI's address: in the project's directory, or
/// in `/`, not a clone of any repository. None for a machine over SSH,
/// which is reached otherwise.
pub fn open_web(target: &Target, native: &Path, wsl_silt: &str) -> Option<Command> {
    let mut command = silt_in(target, native, wsl_silt)?;
    command.args(["open", "--no-browser"]);
    Some(command)
}

/// The command that starts the backend's daemon unless one runs, outside
/// any repository, so its watchers and sync run before it's shown.
pub fn start_daemon(backend: &Backend, native: &Path, wsl_silt: &str) -> Option<Command> {
    let mut command = silt_in(&Target { backend: backend.clone(), ..Target::default() }, native, wsl_silt)?;
    command.args(["daemon", "start"]);
    Some(command)
}

/// The command that restarts the backend's daemon on the build installed
/// since it started, if one was, once no agent works in it.
pub fn restart_replaced_daemon(backend: &Backend, native: &Path, wsl_silt: &str) -> Option<Command> {
    let mut command = silt_in(&Target { backend: backend.clone(), ..Target::default() }, native, wsl_silt)?;
    command.args(["daemon", "restart", "--if-replaced"]);
    Some(command)
}

fn silt_in(target: &Target, native: &Path, wsl_silt: &str) -> Option<Command> {
    let dir = target.project.as_deref().unwrap_or("/");
    match &target.backend {
        Backend::Native => {
            let mut command = Command::new(native);
            command.current_dir(dir);
            Some(command)
        }
        Backend::Wsl(distribution) => {
            let mut command = Command::new("wsl.exe");
            command.args(["-d", distribution, "--cd", dir, "--exec", "sh", "-c"]);
            command.args([IN_THEIR_SHELL, wsl_silt]);
            Some(command)
        }
        Backend::Ssh(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(text: &str) -> Url {
        Url::parse(text).expect("a link")
    }

    #[test]
    fn links_pick_a_known_backend_and_a_directory() {
        let known = [Backend::Native, Backend::Wsl("Ubuntu".into()), Backend::Ssh("home gpu".into())];
        assert_eq!(
            target_of(&link("silt://open/?project=/home/me/a%20b&backend=wsl:Ubuntu"), &known),
            Some(Target { backend: Backend::Wsl("Ubuntu".into()), project: Some("/home/me/a b".into()), page: None })
        );
        assert_eq!(target_of(&link("silt://open/"), &known), Some(Target::default()));
        assert_eq!(target_of(&link("silt://open/?backend=wsl:Debian"), &known), None, "not a distribution here");
        assert_eq!(target_of(&link("silt://open/?backend=wsl:-d"), &known), None);
        assert_eq!(target_of(&link("silt://open/?project=/home/../etc&backend=native"), &known), None);
        assert_eq!(target_of(&link("silt://open/?project=/a%0Ab"), &known), None);
        assert_eq!(target_of(&link("silt://elsewhere/?backend=native"), &known), None);
        assert_eq!(target_of(&link("https://open/?backend=native"), &known), None);
        assert_eq!(target_of(&link("silt://open/t/abc"), &known), None, "permalinks are for later");
        assert_eq!(
            target_of(&link("silt://open/?backend=ssh:home%20gpu"), &known),
            Some(Target { backend: Backend::Ssh("home gpu".into()), project: None, page: None })
        );
        assert_eq!(target_of(&link("silt://open/?backend=ssh:vps"), &known), None, "not a machine here");
        assert_eq!(target_of(&link("silt://open/?project=/srv&backend=ssh:home%20gpu"), &known), None, "no project there");
    }

    #[test]
    fn links_open_a_thread_or_new_thread_on_a_known_machine() {
        let known = [Backend::Native, Backend::Ssh("nuc".into())];
        let thread = target_of(&link("silt://open/?backend=ssh:nuc&thread=a81cee7abcb7"), &known).expect("a target");
        assert_eq!(thread.page, Some(Page::Thread("a81cee7abcb7".into())));
        assert_eq!(thread.page.expect("a page").path(), ("/t/a81cee7abcb7".to_owned(), None));
        let new = target_of(&link("silt://open/?backend=ssh:nuc&new=1&origin=github.com/u/silt&draft=fix%20it%0Aplease"), &known)
            .expect("a target");
        assert_eq!(new.page, Some(Page::New { draft: Some("fix it\nplease".into()), origin: Some("github.com/u/silt".into()) }));
        assert_eq!(
            new.page.expect("a page").path(),
            ("/new".to_owned(), Some("origin=github.com%2Fu%2Fsilt&draft=fix+it%0Aplease".to_owned()))
        );
        for wrong in [
            "silt://open/?backend=ssh:nuc&thread=..%2F..%2Fapi",
            "silt://open/?backend=ssh:nuc&thread=ABC",
            "silt://open/?backend=ssh:far&thread=a81cee7abcb7",
            "silt://open/?backend=native&project=/x&thread=a81cee7abcb7",
            "silt://open/?backend=native&new=1&draft=%1B%5B2J",
        ] {
            assert_eq!(target_of(&link(wrong), &known), None, "{wrong}");
        }
    }

    #[test]
    fn only_the_relaunch_link_relaunches() {
        assert!(is_relaunch(&link("silt://relaunch")));
        assert!(is_relaunch(&link("silt://relaunch/")));
        assert!(!is_relaunch(&link("silt://relaunch/now")));
        assert!(!is_relaunch(&link("silt://open/")));
        assert!(!is_relaunch(&link("https://relaunch/")));
    }

    #[test]
    fn backends_round_trip_through_their_ids() {
        for backend in [Backend::Native, Backend::Wsl("Ubuntu-24.04".into()), Backend::Ssh("home gpu".into())] {
            assert_eq!(Backend::parse(&backend.id()), Some(backend));
        }
        assert_eq!(Backend::parse("wsl:"), None);
        assert_eq!(Backend::parse("local"), None);
        assert_eq!(Backend::parse("ssh:"), None);
        assert_eq!(Backend::parse("ssh: x"), None);
    }

    #[test]
    fn distributions_are_read_from_wsl_s_utf16() {
        let output: Vec<u8> = "\u{feff}Ubuntu\r\nDebian\r\n\r\nbad name\r\ndocker-desktop\r\ndocker-desktop-data\r\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
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
        let target = Target { backend: Backend::Wsl("Ubuntu".into()), project: Some("/home/me/repo".into()), page: None };
        let command = open_web(&target, Path::new("silt"), "/usr/local/bin/silt").expect("a command");
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
        let native = open_web(&Target::default(), Path::new("/opt/silt"), "silt").expect("a command");
        assert_eq!(native.get_program(), "/opt/silt");
        assert_eq!(native.get_current_dir(), Some(Path::new("/")));
    }

    #[test]
    fn a_daemon_starts_outside_any_repository_in_the_person_s_shell() {
        let command = start_daemon(&Backend::Wsl("Ubuntu".into()), Path::new("silt"), "/usr/local/bin/silt").expect("a command");
        let args: Vec<_> = command.get_args().map(|arg| arg.to_string_lossy().into_owned()).collect();
        assert_eq!(command.get_program(), "wsl.exe");
        assert_eq!(
            args,
            ["-d", "Ubuntu", "--cd", "/", "--exec", "sh", "-c", IN_THEIR_SHELL, "/usr/local/bin/silt", "daemon", "start"]
        );
        let native = start_daemon(&Backend::Native, Path::new("/opt/silt"), "").expect("a command");
        assert!(start_daemon(&Backend::Ssh("vps".into()), Path::new("/opt/silt"), "").is_none(), "reached otherwise");
        assert_eq!(native.get_program(), "/opt/silt");
        assert!(native.get_args().eq(["daemon", "start"]));
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
