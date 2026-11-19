//! Machines reached over SSH (ADR 0108): what the person adds, the links the
//! page edits them with, the `ssh` commands that reach them, and what ssh's
//! failures mean. Nothing here runs anything.

use std::path::Path;
use std::process::Command;

use tauri::Url;

use crate::backends::IN_THEIR_SHELL;

/// A computer the person reaches with their own `ssh`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Machine {
    /// What Runs on shows, and what its backend's id is made of.
    pub name: String,
    /// `user@host`, `host`, or a `Host` alias from `~/.ssh/config`.
    pub destination: String,
    pub port: Option<u16>,
    /// When the window last showed it, in milliseconds since the epoch.
    pub last_shown: Option<u64>,
}

impl Machine {
    /// The host part of the destination, as messages name it.
    pub fn host(&self) -> &str {
        self.destination.rsplit_once('@').map_or(&*self.destination, |(_, host)| host)
    }
}

pub fn is_machine_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= 64
        && name.trim() == name
        && name.chars().all(|char| char.is_alphanumeric() || matches!(char, '.' | '_' | '-' | ' '))
}

/// An optional `user@` and a host, neither of which can become an option
/// of `ssh`'s.
pub fn is_destination(destination: &str) -> bool {
    let (user, host) = match destination.split_once('@') {
        Some((user, host)) => (Some(user), host),
        None => (None, destination),
    };
    let plain = |part: &str, extra: &[char]| {
        !part.is_empty()
            && !part.starts_with('-')
            && part.chars().all(|char| char.is_ascii_alphanumeric() || matches!(char, '.' | '_' | '-') || extra.contains(&char))
    };
    destination.len() <= 255 && user.is_none_or(|user| plain(user, &[])) && plain(host, &[':'])
}

pub fn is_port(port: u32) -> bool {
    (1..=65535).contains(&port)
}

/// The machines kept in the app's data dir. Entries that don't pass the
/// checks are left out, as are later ones of a name already taken: the
/// file is the person's to edit.
pub fn remembered_machines(text: &str) -> Vec<Machine> {
    let Ok(serde_json::Value::Array(entries)) = serde_json::from_str(text) else {
        return Vec::new();
    };
    let mut machines: Vec<Machine> = Vec::new();
    for entry in entries {
        let name = entry.get("name").and_then(|name| name.as_str()).filter(|name| is_machine_name(name));
        let destination = entry.get("destination").and_then(|destination| destination.as_str()).filter(|it| is_destination(it));
        let port = match entry.get("port") {
            None | Some(serde_json::Value::Null) => Some(None),
            Some(port) => port
                .as_u64()
                .and_then(|port| u32::try_from(port).ok())
                .filter(|port| is_port(*port))
                .map(|port| Some(u16::try_from(port).expect("ports fit"))),
        };
        let last_shown = entry.get("last_shown").and_then(serde_json::Value::as_u64);
        if let (Some(name), Some(destination), Some(port)) = (name, destination, port)
            && !machines.iter().any(|machine| machine.name == name)
        {
            machines.push(Machine { name: name.to_owned(), destination: destination.to_owned(), port, last_shown });
        }
    }
    machines
}

pub fn remember_machines(machines: &[Machine]) -> String {
    let entries: Vec<serde_json::Value> = machines
        .iter()
        .map(|machine| {
            let mut entry = serde_json::json!({ "name": machine.name, "destination": machine.destination });
            if let Some(port) = machine.port {
                entry["port"] = port.into();
            }
            if let Some(shown) = machine.last_shown {
                entry["last_shown"] = shown.into();
            }
            entry
        })
        .collect();
    serde_json::to_string_pretty(&entries).expect("values serialize")
}

/// What a `silt://machines/<action>?…` link asks the app to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    Add(Machine),
    /// The machine named `was`, changed to `machine`.
    Change {
        was: String,
        machine: Machine,
    },
    Remove(String),
    /// Tries the steps of a connection, and adds the machine once they
    /// pass when `add` is set.
    Test {
        machine: Machine,
        add: bool,
    },
    Disconnect(String),
}

/// The edit a link asks for, if it's one and its fields pass the checks.
pub fn edit_of(link: &Url) -> Option<Edit> {
    if link.scheme() != "silt" || link.host_str() != Some("machines") {
        return None;
    }
    let field = |key: &str| link.query_pairs().find(|(name, _)| name == key).map(|(_, value)| value.into_owned());
    let name = || field("name").filter(|name| is_machine_name(name));
    let machine = || {
        let port = match field("port").filter(|port| !port.is_empty()) {
            None => None,
            Some(port) => Some(port.parse::<u16>().ok().filter(|port| is_port(u32::from(*port)))?),
        };
        Some(Machine {
            name: name()?,
            destination: field("destination").filter(|it| is_destination(it))?,
            port,
            last_shown: None,
        })
    };
    match link.path() {
        "/add" => machine().map(Edit::Add),
        "/change" => Some(Edit::Change { was: field("was").filter(|was| is_machine_name(was))?, machine: machine()? }),
        "/remove" => name().map(Edit::Remove),
        "/test" => Some(Edit::Test { machine: machine()?, add: field("add").as_deref() == Some("1") }),
        "/disconnect" => name().map(Edit::Disconnect),
        _ => None,
    }
}

/// Quotes `text` as one word for a POSIX shell, which fish and csh read
/// the same way, as long as it has no backslash or line break.
pub fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The options every `ssh` run takes: no prompts, as the window has no
/// terminal to type into, and a bound on the wait.
fn ssh_base(machine: &Machine, control: Option<&Path>) -> Command {
    let mut command = Command::new(if cfg!(windows) { "ssh.exe" } else { "ssh" });
    command.args(["-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10"]);
    // A machine the page named mustn't get the person's agent or display.
    command.args(["-o", "ForwardAgent=no", "-o", "ForwardX11=no"]);
    if let Some(dir) = control {
        command.args(["-o", "ControlMaster=auto", "-o", "ControlPersist=60", "-o"]);
        command.arg(format!("ControlPath={}", dir.join("%C").display()));
    }
    if let Some(port) = machine.port {
        command.args(["-p", &port.to_string()]);
    }
    command
}

/// The line the remote login shell runs: `program` with `args` in the
/// person's own shell, from `/`, as a distribution's `silt` runs.
pub fn in_their_shell(program: &str, args: &[&str]) -> String {
    let script = format!("cd / && {}", IN_THEIR_SHELL.replace('\n', "; "));
    let mut line = format!("exec sh -c {} {}", sh_quote(&script), sh_quote(program));
    for arg in args {
        line.push(' ');
        line.push_str(&sh_quote(arg));
    }
    line
}

/// Runs `line` on the machine. `control` is the directory of the shared
/// connections' sockets, where there are any.
pub fn run_on(machine: &Machine, control: Option<&Path>, line: &str) -> Command {
    let mut command = ssh_base(machine, control);
    // Forwards from `~/.ssh/config` would ride along on every run.
    command.args(["-o", "ClearAllForwardings=yes", "--", &machine.destination, line]);
    command
}

/// Signs in, says as whom, and looks for `silt` where the person's
/// terminal finds it, and its version. The lines are marked, as shell
/// startup files may print their own.
pub fn probe_line() -> String {
    let probe = r#"printf 'silt-user %s\n' "$(id -un)"; p=$(command -v silt) || exit 0; printf 'silt-path %s\n' "$p"; printf 'silt-version %s\n' "$("$p" --version 2>/dev/null)""#;
    in_their_shell("sh", &["-c", probe])
}

/// What the probe found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Probed {
    pub user: Option<String>,
    pub silt: Option<String>,
    pub version: Option<String>,
}

pub fn probed(stdout: &str) -> Probed {
    let mut found = Probed::default();
    for line in stdout.lines().map(str::trim_end) {
        let value = |prefix: &str| {
            line.strip_prefix(prefix).map(str::trim).filter(|value| !value.is_empty() && !value.chars().any(char::is_control))
        };
        if let Some(user) = value("silt-user ") {
            found.user = Some(user.to_owned());
        } else if let Some(path) = value("silt-path ") {
            found.silt = Some(path.to_owned()).filter(|path| is_silt_path(path));
        } else if let Some(version) = value("silt-version ") {
            found.version = Some(version.strip_prefix("silt ").unwrap_or(version).to_owned());
        }
    }
    found
}

/// An absolute path the remote shell takes as it is.
pub fn is_silt_path(path: &str) -> bool {
    path.starts_with('/') && path.len() <= 4096 && !path.chars().any(|char| char.is_control() || char == '\\')
}

/// Forwards `port` here to the same port there, where the daemon listens:
/// it answers only requests addressed to its own port. It holds a
/// connection of its own, so stopping it closes the forward.
pub fn tunnel(machine: &Machine, port: u16) -> Command {
    let mut command = ssh_base(machine, None);
    command.args(["-N", "-o", "ExitOnForwardFailure=yes", "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=3"]);
    command.args(["-o", "ControlPath=none", "-L", &format!("127.0.0.1:{port}:127.0.0.1:{port}")]);
    command.args(["--", &machine.destination]);
    command
}

/// The steps of reaching a machine, as a test shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    SignIn,
    Find,
    Start,
    Forward,
}

pub const STEPS: [Step; 4] = [Step::SignIn, Step::Find, Step::Start, Step::Forward];

impl Step {
    /// The step while it's to come or under way.
    pub fn doing(self, machine: &Machine) -> String {
        match self {
            Self::SignIn => format!("Signing in to `{}`", machine.host()),
            Self::Find => "Finding `silt` there".to_owned(),
            Self::Start => "Starting its daemon".to_owned(),
            Self::Forward => "Forwarding its port to this computer".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepState {
    Running,
    Done(String),
    Failed(String),
}

/// What a test of a machine shows: each step as it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestReport {
    machine: Machine,
    steps: Vec<(&'static str, String)>,
    done: bool,
    ok: bool,
}

impl TestReport {
    pub fn new(machine: &Machine) -> Self {
        let steps = STEPS.iter().map(|step| ("waiting", step.doing(machine))).collect();
        Self { machine: machine.clone(), steps, done: false, ok: false }
    }

    pub fn update(&mut self, step: Step, state: StepState) {
        let at = STEPS.iter().position(|each| *each == step).expect("a step");
        self.steps[at] = match state {
            StepState::Running => ("running", format!("{}…", step.doing(&self.machine))),
            StepState::Done(text) => ("done", text),
            StepState::Failed(text) => ("failed", text),
        };
    }

    pub fn finish(&mut self, ok: bool) {
        self.done = true;
        self.ok = ok;
    }

    pub fn json(&self) -> serde_json::Value {
        let steps: Vec<_> = self.steps.iter().map(|(state, text)| serde_json::json!({ "state": state, "text": text })).collect();
        serde_json::json!({
            "name": self.machine.name,
            "destination": self.machine.destination,
            "port": self.machine.port,
            "steps": steps,
            "done": self.done,
            "ok": self.ok,
        })
    }
}

/// Why reaching a machine failed, each with its next step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// Its host key isn't in `known_hosts`.
    UnknownHost,
    /// Its host key isn't the one in `known_hosts`.
    ChangedHost,
    /// No key or agent signed in.
    SignIn,
    /// It can't be reached, with ssh's reason.
    Unreachable(String),
    /// There's no `ssh` here.
    NoSsh,
    /// There's no `silt` there.
    NoSilt,
    /// The daemon's port is taken here.
    PortInUse(u16),
    /// Anything else, as ssh or `silt` said it.
    Other(String),
}

/// What a failed `ssh` run's stderr and exit code mean. 255 is ssh's own
/// failure; any other code is the remote command's.
pub fn failure_of(stderr: &str, code: Option<i32>) -> Failure {
    let has = |text: &str| stderr.contains(text);
    let last = stderr.lines().map(str::trim).rfind(|line| !line.is_empty()).unwrap_or("it failed");
    let last = last.strip_prefix("silt: ").unwrap_or(last).to_owned();
    if code != Some(255) {
        return Failure::Other(last);
    }
    if has("REMOTE HOST IDENTIFICATION HAS CHANGED") || has("Host key for") && has("has changed") {
        Failure::ChangedHost
    } else if has("Host key verification failed") || has("No ") && has("host key is known") {
        Failure::UnknownHost
    } else if has("Permission denied") || has("Too many authentication failures") {
        Failure::SignIn
    } else if has("Address already in use") || has("cannot listen to port") || has("Could not request local forwarding") {
        Failure::PortInUse(0)
    } else if [
        "Could not resolve hostname",
        "Connection refused",
        "Connection timed out",
        "Operation timed out",
        "No route to host",
        "Network is unreachable",
        "Connection closed by",
        "Connection reset by",
    ]
    .iter()
    .any(|text| has(text))
    {
        let reason = last.rsplit_once(": ").map_or(&*last, |(_, reason)| reason);
        Failure::Unreachable(reason.trim_end_matches('.').to_owned())
    } else {
        Failure::Other(last)
    }
}

impl Failure {
    /// The failure and its next step for `machine`, with commands and
    /// paths in backticks, which the page shows as code.
    pub fn explain(&self, machine: &Machine) -> String {
        let (host, destination) = (machine.host(), &machine.destination);
        let ssh = match machine.port {
            Some(port) => format!("ssh -p {port} {destination}"),
            None => format!("ssh {destination}"),
        };
        match self {
            Self::UnknownHost => format!(
                "The host key of `{host}` isn't known here, so Silt won't trust it on its own. Run `{ssh}` once in a terminal to check and accept it, then retry."
            ),
            Self::ChangedHost => format!(
                "The host key of `{host}` isn't the one known here, which can mean someone is in between. Check it, fix `~/.ssh/known_hosts`, then retry."
            ),
            Self::SignIn => format!(
                "`{host}` refused the sign-in: none of your keys was accepted, and Silt can't type a password. Add one with `ssh-copy-id {destination}`, then retry."
            ),
            Self::Unreachable(reason) => {
                format!("Couldn't reach `{host}`: {}. Check it's on and reachable from here, then retry.", reason.to_lowercase())
            }
            Self::NoSsh => "`ssh` isn't installed on this computer. Install OpenSSH's client, then retry.".to_owned(),
            Self::NoSilt => format!(
                "Signed in, but `silt` isn't installed on `{host}`. Install it there with `curl -fsSL https://raw.githubusercontent.com/Urcra/silt/main/scripts/install.sh | sh`, then retry."
            ),
            Self::PortInUse(port) => format!(
                "Port {port} is already in use on this computer, so the daemon on `{host}` can't be forwarded to it. Run `silt daemon restart` there to have it pick another, then retry."
            ),
            Self::Other(reason) => reason.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vps() -> Machine {
        Machine { name: "vps".into(), destination: "me@vps.example.com".into(), port: None, last_shown: None }
    }

    fn link(text: &str) -> Url {
        Url::parse(text).expect("a link")
    }

    #[test]
    fn names_and_destinations_can_t_become_options() {
        for name in ["vps", "home gpu", "build-box.2", "Pí"] {
            assert!(is_machine_name(name), "{name}");
        }
        for name in ["", " vps", "a/b", "a\nb", &"x".repeat(65)] {
            assert!(!is_machine_name(name), "{name:?}");
        }
        for destination in ["vps", "me@vps.example.com", "ci@10.0.4.12", "fe80::1", "a_b-c"] {
            assert!(is_destination(destination), "{destination}");
        }
        for destination in ["", "-oProxyCommand=x", "me@-x", "@host", "me@", "a b", "me@host;x", "a@b@c", &"h".repeat(256)] {
            assert!(!is_destination(destination), "{destination:?}");
        }
        assert!(is_port(22) && is_port(65535) && !is_port(0) && !is_port(65536));
    }

    #[test]
    fn machines_are_remembered_and_what_doesn_t_pass_is_dropped() {
        let machines =
            [vps(), Machine { name: "gpu".into(), destination: "gpu-box".into(), port: Some(2222), last_shown: Some(7) }];
        assert_eq!(remembered_machines(&remember_machines(&machines)), machines);
        let edited = r#"[{"name": "vps", "destination": "vps"}, {"name": "vps", "destination": "other"},
            {"name": "x", "destination": "-oProxyCommand=y"}, {"name": "y", "destination": "y", "port": 0}, "z"]"#;
        assert_eq!(
            remembered_machines(edited),
            [Machine { name: "vps".into(), destination: "vps".into(), port: None, last_shown: None }]
        );
        assert!(remembered_machines("not json").is_empty());
    }

    #[test]
    fn links_edit_machines_only_with_fields_that_pass() {
        let gpu = Machine { name: "home gpu".into(), destination: "gpu-box".into(), port: Some(2222), last_shown: None };
        assert_eq!(
            edit_of(&link("silt://machines/add?name=home%20gpu&destination=gpu-box&port=2222")),
            Some(Edit::Add(gpu.clone()))
        );
        assert_eq!(
            edit_of(&link("silt://machines/change?was=gpu&name=home%20gpu&destination=gpu-box&port=2222")),
            Some(Edit::Change { was: "gpu".into(), machine: gpu.clone() })
        );
        assert_eq!(
            edit_of(&link("silt://machines/test?name=vps&destination=me@vps.example.com&port=&add=1")),
            Some(Edit::Test { machine: vps(), add: true })
        );
        assert_eq!(edit_of(&link("silt://machines/remove?name=vps")), Some(Edit::Remove("vps".into())));
        assert_eq!(edit_of(&link("silt://machines/disconnect?name=vps")), Some(Edit::Disconnect("vps".into())));
        for wrong in [
            "silt://machines/add?name=vps&destination=-oProxyCommand=x",
            "silt://machines/add?name=vps&destination=vps&port=0",
            "silt://machines/add?name=vps&destination=vps&port=x",
            "silt://machines/add?destination=vps",
            "silt://machines/launch?name=vps",
            "silt://open/add?name=vps&destination=vps",
            "https://machines/add?name=vps&destination=vps",
        ] {
            assert_eq!(edit_of(&link(wrong)), None, "{wrong}");
        }
    }

    fn args(command: &Command) -> Vec<String> {
        command.get_args().map(|arg| arg.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn runs_take_no_prompts_and_share_a_connection_where_they_can() {
        let shared = run_on(&vps(), Some(Path::new("/data/ssh")), "true");
        let shared = args(&shared);
        for option in ["BatchMode=yes", "ForwardAgent=no", "ForwardX11=no", "ClearAllForwardings=yes", "ControlMaster=auto"] {
            assert!(shared.windows(2).any(|pair| pair == ["-o", option]), "{option} in {shared:?}");
        }
        assert!(shared.windows(2).any(|pair| pair == ["-o", "ControlPath=/data/ssh/%C"]));
        assert_eq!(shared[shared.len() - 3..], ["--", "me@vps.example.com", "true"]);
        let ported = run_on(&Machine { port: Some(2222), ..vps() }, None, "true");
        let ported = args(&ported);
        assert!(ported.windows(2).any(|pair| pair == ["-p", "2222"]));
        assert!(!ported.iter().any(|arg| arg.starts_with("Control")), "{ported:?}");
    }

    #[test]
    fn the_tunnel_forwards_the_same_port_on_its_own_connection() {
        let command = tunnel(&vps(), 4321);
        let args = args(&command);
        assert!(args.windows(2).any(|pair| pair == ["-L", "127.0.0.1:4321:127.0.0.1:4321"]));
        assert!(args.windows(2).any(|pair| pair == ["-o", "ExitOnForwardFailure=yes"]));
        assert!(args.windows(2).any(|pair| pair == ["-o", "ControlPath=none"]));
        assert!(args.contains(&"-N".to_owned()) && args.contains(&"BatchMode=yes".to_owned()));
        assert_eq!(args[args.len() - 2..], ["--", "me@vps.example.com"]);
    }

    #[cfg(unix)]
    #[test]
    fn the_remote_line_runs_the_program_with_its_arguments_as_they_are() {
        for shell in ["/bin/sh", "/usr/bin/fish"] {
            let line = in_their_shell("printf", &["[%s]", "a b", "$HOME", "it's"]);
            let output = Command::new("sh")
                .args(["-c", &line])
                .env("SHELL", shell)
                .env("HOME", "/nonexistent")
                .stdin(std::process::Stdio::null())
                .output()
                .expect("sh runs");
            assert_eq!(String::from_utf8_lossy(&output.stdout), "[a b][$HOME][it's]", "with SHELL={shell}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_probe_finds_silt_and_who_signed_in() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("silt-desktop-probe-{}", std::process::id()));
        let (with, without) = (dir.join("with"), dir.join("without"));
        std::fs::create_dir_all(with.join("bin")).expect("a temporary dir");
        std::fs::create_dir_all(&without).expect("a temporary dir");
        let silt = with.join("bin/silt");
        std::fs::write(&silt, "#!/bin/sh\necho 'silt 0.0.1 (nightly abc1234)'\n").expect("a stand-in silt");
        std::fs::set_permissions(&silt, std::fs::Permissions::from_mode(0o755)).expect("executable");
        // Found as a terminal finds it: through the login shell's profile.
        std::fs::write(with.join(".profile"), format!("PATH='{}/bin':\"$PATH\"\n", with.display())).expect("a profile");
        let run = |home: &Path| {
            let output = Command::new("sh")
                .args(["-c", &probe_line()])
                .env("SHELL", "/bin/sh")
                .env("HOME", home)
                .env("PATH", "/usr/bin:/bin")
                .stdin(std::process::Stdio::null())
                .output()
                .expect("sh runs");
            probed(&String::from_utf8_lossy(&output.stdout))
        };
        let found = run(&with);
        assert!(found.user.is_some());
        assert_eq!(found.silt.as_deref(), Some(&*silt.to_string_lossy()));
        assert_eq!(found.version.as_deref(), Some("0.0.1 (nightly abc1234)"));
        let missing = run(&without);
        assert!(missing.user.is_some() && missing.silt.is_none(), "{missing:?}");
        std::fs::remove_dir_all(&dir).expect("cleaned up");
    }

    #[test]
    fn the_probe_s_lines_are_read_past_whatever_startup_files_print() {
        let found = probed("Welcome!\nsilt-user me\nsilt-path /home/me/.local/bin/silt\nsilt-version silt 0.0.1\n");
        assert_eq!(
            found,
            Probed { user: Some("me".into()), silt: Some("/home/me/.local/bin/silt".into()), version: Some("0.0.1".into()) }
        );
        assert_eq!(probed("silt-path silt\n").silt, None, "not a path");
        assert_eq!(probed("silt-path /a\\b\n").silt, None);
    }

    #[test]
    fn ssh_s_failures_say_what_to_do() {
        let failure = |stderr: &str| failure_of(stderr, Some(255));
        assert_eq!(
            failure(
                "No ED25519 host key is known for vps and you have requested strict checking.\nHost key verification failed.\n"
            ),
            Failure::UnknownHost
        );
        assert_eq!(
            failure("@@@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! @@@\nHost key verification failed.\n"),
            Failure::ChangedHost
        );
        assert_eq!(failure("me@vps: Permission denied (publickey).\n"), Failure::SignIn);
        assert_eq!(
            failure("ssh: connect to host vps port 22: Connection refused\n"),
            Failure::Unreachable("Connection refused".into())
        );
        assert_eq!(
            failure("ssh: Could not resolve hostname nowhere: Name or service not known\n"),
            Failure::Unreachable("Name or service not known".into())
        );
        assert_eq!(
            failure("bind [127.0.0.1]:4321: Address already in use\nCould not request local forwarding.\n"),
            Failure::PortInUse(0)
        );
        assert_eq!(failure_of("silt: no daemon\n", Some(1)), Failure::Other("no daemon".into()));
        let explained = Failure::SignIn.explain(&vps());
        assert!(explained.starts_with("`vps.example.com` refused the sign-in"), "{explained}");
        assert!(explained.contains("`ssh-copy-id me@vps.example.com`"));
        assert!(
            Failure::UnknownHost.explain(&Machine { port: Some(2222), ..vps() }).contains("`ssh -p 2222 me@vps.example.com`")
        );
    }
}
