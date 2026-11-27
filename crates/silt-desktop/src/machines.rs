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
    /// Puts this computer's `silt` there, in place of the one it has if
    /// any (ADR 0109), and connects; adds the machine once it's connected
    /// when `add` is set.
    Install {
        machine: Machine,
        add: bool,
    },
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
        "/install" => Some(Edit::Install { machine: machine()?, add: field("add").as_deref() == Some("1") }),
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

/// Signs in, says as whom, on what, and where home is, and looks for
/// `silt` where the person's terminal finds it, else where Silt installs
/// it, and its version. The lines are marked, as shell startup files may
/// print their own.
pub fn probe_line() -> String {
    let probe = r#"printf 'silt-user %s\n' "$(id -un)"; printf 'silt-system %s\n' "$(uname -sm)"; printf 'silt-home %s\n' "$HOME"; p=$(command -v silt) || p=$HOME/.local/bin/silt; [ -x "$p" ] || exit 0; printf 'silt-path %s\n' "$p"; printf 'silt-version %s\n' "$("$p" --version 2>/dev/null)""#;
    in_their_shell("sh", &["-c", probe])
}

/// What the probe found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Probed {
    pub user: Option<String>,
    pub platform: Option<Platform>,
    pub home: Option<String>,
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
        } else if let Some(system) = value("silt-system ") {
            found.platform = Platform::of_uname(system);
        } else if let Some(home) = value("silt-home ") {
            found.home = Some(home.trim_end_matches('/').to_owned()).filter(|home| is_silt_path(home));
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

/// What to say when this computer's build doesn't run there.
const BUILD_THERE: &str =
    "Build it there with `cargo install --locked --git https://github.com/Urcra/silt silt-cli`, then try again.";

/// An OS and a CPU, named as Rust names them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub os: String,
    pub arch: String,
}

impl Platform {
    /// The one this program was built for.
    pub fn here() -> Self {
        Self { os: std::env::consts::OS.to_owned(), arch: std::env::consts::ARCH.to_owned() }
    }

    /// The one `uname -sm` names, such as `Linux x86_64` or `Darwin arm64`.
    pub fn of_uname(text: &str) -> Option<Self> {
        let (os, arch) = text.trim().split_once(' ')?;
        let os = match os {
            "Linux" => "linux".to_owned(),
            "Darwin" => "macos".to_owned(),
            os => os.to_lowercase(),
        };
        let arch = match arch.trim() {
            "arm64" | "aarch64" => "aarch64".to_owned(),
            "amd64" | "x86_64" => "x86_64".to_owned(),
            arch => arch.to_owned(),
        };
        let plain = |part: &str| !part.is_empty() && part.chars().all(|char| char.is_ascii_alphanumeric() || char == '_');
        (plain(&os) && plain(&arch)).then_some(Self { os, arch })
    }
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let os = match &*self.os {
            "linux" => "Linux",
            "macos" => "macOS",
            "windows" => "Windows",
            os => os,
        };
        write!(f, "{os} {}", self.arch)
    }
}

/// `silt --version`'s answer without its `silt `.
pub fn version_of(stdout: &str) -> Option<String> {
    // The last line, past whatever the login shell's startup files print.
    let line = stdout.lines().map(str::trim).rfind(|line| !line.is_empty())?;
    let version = line.strip_prefix("silt ").unwrap_or(line);
    (!version.chars().any(char::is_control)).then(|| version.to_owned())
}

/// How a machine gets this computer's build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Copied as it is: the machine has this computer's OS and CPU.
    Copy,
    /// It can't be: why, and what to do instead.
    Cannot(String),
}

pub fn plan(here: &Platform, there: &Platform, host: &str) -> Plan {
    if here == there {
        return Plan::Copy;
    }
    Plan::Cannot(format!("`{host}` runs {there}, while this computer’s `silt` is built for {here}. {BUILD_THERE}"))
}

/// `path` with the home directory as `~`, as messages show it.
pub fn shown_path(path: &str, home: Option<&str>) -> String {
    match home.and_then(|home| path.strip_prefix(home)).filter(|rest| rest.starts_with('/')) {
        Some(rest) => format!("~{rest}"),
        None => path.to_owned(),
    }
}

/// How far a copy got, as `41 of 96 MB`.
pub fn progress(sent: u64, total: u64) -> String {
    let megabytes = |bytes: u64| bytes.div_ceil(1_000_000);
    format!("{} of {} MB", megabytes(sent).min(megabytes(total)), megabytes(total))
}

/// `script` run by `sh` there with `args`, whatever the login shell: these
/// need nothing of the person's own.
fn plain_sh(script: &str, args: &[&str]) -> String {
    let mut line = format!("exec sh -c {} sh", sh_quote(script));
    for arg in args {
        line.push(' ');
        line.push_str(&sh_quote(arg));
    }
    line
}

/// Each program goes to `.<name>.new` beside where it belongs, and is
/// renamed into place only once it ran there, so a running `silt` keeps
/// its old binary and a copy that fails leaves nothing behind.
const BESIDE: &str = r#"d=${t%/*}; n=${t##*/}; new="$d/.$n.new""#;

/// Writes stdin to beside `target`, refusing a directory the person
/// can't write to: Silt never uses `sudo`.
pub fn copy_line(target: &str) -> String {
    let script = format!(
        r#"t=$1; {BESIDE}; mkdir -p "$d" 2>/dev/null; [ -w "$d" ] || {{ echo silt-unwritable >&2; exit 3; }}; cat > "$new" && chmod 755 "$new""#
    );
    plain_sh(&script, &[target])
}

/// The copy beside `target` says its version.
pub fn check_line(target: &str) -> String {
    plain_sh(&format!(r#"t=$1; {BESIDE}; exec "$new" --version"#), &[target])
}

/// Renames each copy over what it replaces.
pub fn place_line(targets: &[&str]) -> String {
    plain_sh(&format!(r#"for t in "$@"; do {BESIDE}; mv -f "$new" "$t" || exit 1; done"#), targets)
}

/// Removes the copies, as a check that failed leaves them.
pub fn discard_line(targets: &[&str]) -> String {
    plain_sh(&format!(r#"for t in "$@"; do {BESIDE}; rm -f "$new"; done"#), targets)
}

/// Why a copy didn't run there, from what running it said: most often a C
/// library older than the one it was built against.
pub fn wont_run_of(stderr: &str) -> String {
    let needs = stderr.split(|char: char| !(char.is_ascii_alphanumeric() || matches!(char, '_' | '.'))).find(|word| {
        word.strip_prefix("GLIBC_")
            .is_some_and(|version| !version.is_empty() && version.chars().all(|char| char.is_ascii_digit() || char == '.'))
    });
    if let Some(needs) = needs {
        return format!("it needs `{needs}`, newer than the system there");
    }
    let last = stderr.lines().map(str::trim).rfind(|line| !line.is_empty()).unwrap_or("it didn’t start");
    let last = last.rsplit_once(": ").map_or(last, |(_, reason)| reason);
    last.chars().filter(|char| !matches!(char, '`' | '\'')).collect::<String>().trim_end_matches('.').to_lowercase()
}

/// What `silt daemon restart --if-replaced` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Restarted {
    Now,
    /// Once the agents working in it finish, this many.
    Waiting(usize),
    NotRunning,
    /// Left as it was, with what it said.
    Unchanged(String),
}

pub fn restarted(stdout: &str) -> Restarted {
    let line = stdout.lines().map(str::trim).rfind(|line| !line.is_empty()).unwrap_or_default();
    if line == "restarted" {
        Restarted::Now
    } else if line == "not running" {
        Restarted::NotRunning
    } else if let Some(working) = line.strip_prefix("restarts once no agent works in it: ") {
        let agents =
            if working.starts_with("an agent") { 1 } else { working.split(' ').next().and_then(|n| n.parse().ok()).unwrap_or(1) };
        Restarted::Waiting(agents)
    } else {
        Restarted::Unchanged(line.to_owned())
    }
}

impl Restarted {
    pub fn said(&self) -> String {
        match self {
            Self::Now => "Restarted its daemon on it".to_owned(),
            Self::Waiting(1) => "Its daemon restarts on it once its agent finishes".to_owned(),
            Self::Waiting(agents) => format!("Its daemon restarts on it once its {agents} agents finish"),
            Self::NotRunning => "Its daemon wasn’t running".to_owned(),
            Self::Unchanged(said) if said.is_empty() => "Left its daemon as it was".to_owned(),
            Self::Unchanged(said) => format!("Left its daemon as it was: {said}"),
        }
    }
}

/// The steps of reaching a machine, as a test shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    SignIn,
    Find,
    /// What the machine runs, and so how it gets this computer's build.
    Platform,
    Copy,
    Check,
    Start,
    Restart,
    Forward,
}

/// A test's steps.
pub const STEPS: [Step; 4] = [Step::SignIn, Step::Find, Step::Start, Step::Forward];
/// Installing `silt` where there's none.
pub const INSTALL_STEPS: [Step; 6] = [Step::SignIn, Step::Platform, Step::Copy, Step::Check, Step::Start, Step::Forward];
/// Replacing the `silt` there.
pub const UPDATE_STEPS: [Step; 6] = [Step::SignIn, Step::Find, Step::Copy, Step::Check, Step::Restart, Step::Forward];

impl Step {
    /// The step while it's to come or under way.
    pub fn doing(self, machine: &Machine) -> String {
        match self {
            Self::SignIn => format!("Signing in to `{}`", machine.host()),
            Self::Find => "Finding `silt` there".to_owned(),
            Self::Start => "Starting its daemon".to_owned(),
            Self::Platform => format!("Checking what `{}` runs", machine.host()),
            Self::Copy => "Installing this computer’s `silt` there".to_owned(),
            Self::Check => "Checking it runs there".to_owned(),
            Self::Restart => "Restarting its daemon on it".to_owned(),
            Self::Forward => "Forwarding its port to this computer".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepState {
    Running,
    /// Under way, saying how far it got.
    Doing(String),
    Done(String),
    Failed(String),
}

/// What a test of a machine, or an install there, shows: each step as it
/// stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestReport {
    machine: Machine,
    action: &'static str,
    steps: Vec<(Step, &'static str, String)>,
    done: bool,
    ok: bool,
    /// It failed for want of `silt` there, which Silt can install.
    can_install: bool,
}

impl TestReport {
    pub fn new(machine: &Machine) -> Self {
        Self::of("test", machine, &STEPS)
    }

    pub fn install(machine: &Machine) -> Self {
        Self::of("install", machine, &UPDATE_STEPS)
    }

    fn of(action: &'static str, machine: &Machine, steps: &[Step]) -> Self {
        let mut report = Self { machine: machine.clone(), action, steps: Vec::new(), done: false, ok: false, can_install: false };
        report.lay_out(steps);
        report
    }

    /// Shows `steps`, keeping how the ones shown already stand.
    pub fn lay_out(&mut self, steps: &[Step]) {
        let was = std::mem::take(&mut self.steps);
        self.steps = steps
            .iter()
            .map(|step| {
                was.iter()
                    .find(|(each, ..)| each == step)
                    .cloned()
                    .unwrap_or_else(|| (*step, "waiting", step.doing(&self.machine)))
            })
            .collect();
    }

    pub fn update(&mut self, step: Step, state: StepState) {
        let entry = match state {
            StepState::Running => (step, "running", format!("{}…", step.doing(&self.machine))),
            StepState::Doing(text) => (step, "running", text),
            StepState::Done(text) => (step, "done", text),
            StepState::Failed(text) => (step, "failed", text),
        };
        match self.steps.iter_mut().find(|(each, ..)| *each == step) {
            Some(shown) => *shown = entry,
            None => self.steps.push(entry),
        }
    }

    pub fn offer_install(&mut self) {
        self.can_install = true;
    }

    pub fn finish(&mut self, ok: bool) {
        self.done = true;
        self.ok = ok;
    }

    pub fn json(&self) -> serde_json::Value {
        let steps: Vec<_> =
            self.steps.iter().map(|(_, state, text)| serde_json::json!({ "state": state, "text": text })).collect();
        serde_json::json!({
            "name": self.machine.name,
            "destination": self.machine.destination,
            "port": self.machine.port,
            "action": self.action,
            "steps": steps,
            "done": self.done,
            "ok": self.ok,
            "canInstall": self.can_install,
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
    /// The signed-in user can't write where `silt` goes, at `path`.
    NotWritable { path: String, user: Option<String> },
    /// This computer's `silt` didn't run there, and why.
    WontRun(String),
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
                "Signed in, but `silt` isn’t installed on `{host}`. Silt can install this computer’s build there, or build it there yourself with `cargo install --locked --git https://github.com/Urcra/silt silt-cli`."
            ),
            Self::NotWritable { path, user } => {
                let whose = user.as_deref().map_or_else(|| "yours".to_owned(), |user| format!("`{user}`’s"));
                format!(
                    "`{path}` on `{host}` isn’t {whose} to replace, and Silt won’t use `sudo`. Update it there as its owner, or remove it so Silt installs its own in `~/.local/bin`."
                )
            }
            Self::WontRun(why) => {
                format!("This computer’s `silt` doesn’t run on `{host}`: {why}. Nothing was installed. {BUILD_THERE}")
            }
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
        assert_eq!(
            edit_of(&link("silt://machines/install?name=vps&destination=me@vps.example.com&add=1")),
            Some(Edit::Install { machine: vps(), add: true })
        );
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
        let control = format!("ControlPath={}", Path::new("/data/ssh").join("%C").display());
        assert!(shared.windows(2).any(|pair| pair == ["-o", control.as_str()]));
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
        assert_eq!(found.platform, Some(Platform::here()));
        assert_eq!(found.home.as_deref(), Some(&*with.to_string_lossy()));
        let missing = run(&without);
        assert!(missing.user.is_some() && missing.silt.is_none(), "{missing:?}");
        // Where Silt installs it, though a non-interactive shell's PATH lacks it.
        std::fs::create_dir_all(without.join(".local/bin")).expect("a temporary dir");
        let installed = without.join(".local/bin/silt");
        std::fs::copy(&silt, &installed).expect("installed");
        assert_eq!(run(&without).silt.as_deref(), Some(&*installed.to_string_lossy()));
        std::fs::remove_dir_all(&dir).expect("cleaned up");
    }

    #[test]
    fn the_probe_s_lines_are_read_past_whatever_startup_files_print() {
        let found = probed("Welcome!\nsilt-user me\nsilt-path /home/me/.local/bin/silt\nsilt-version silt 0.0.1\n");
        assert_eq!(
            found,
            Probed {
                user: Some("me".into()),
                silt: Some("/home/me/.local/bin/silt".into()),
                version: Some("0.0.1".into()),
                ..Probed::default()
            }
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

    #[test]
    fn a_machine_gets_this_computer_s_build_a_release_or_how_to_build_it() {
        let linux = |arch: &str| Platform { os: "linux".into(), arch: arch.into() };
        assert_eq!(Platform::of_uname("Linux x86_64\n"), Some(linux("x86_64")));
        assert_eq!(Platform::of_uname("Linux aarch64"), Some(linux("aarch64")));
        assert_eq!(Platform::of_uname("Darwin arm64"), Some(Platform { os: "macos".into(), arch: "aarch64".into() }));
        assert_eq!(Platform::of_uname("Linux"), None);
        assert_eq!(Platform::of_uname("Linux x86_64; rm"), None);
        assert_eq!(linux("aarch64").to_string(), "Linux aarch64");

        assert_eq!(version_of("silt 0.0.1 (nightly a1b2c3d)\n").as_deref(), Some("0.0.1 (nightly a1b2c3d)"));
        assert_eq!(version_of("Welcome!\nsilt 0.0.1 (nightly a1b2c3d)\n\n").as_deref(), Some("0.0.1 (nightly a1b2c3d)"));

        assert_eq!(plan(&linux("x86_64"), &linux("x86_64"), "vps"), Plan::Copy);
        let Plan::Cannot(why) = plan(&linux("x86_64"), &linux("aarch64"), "vps") else { panic!("no plan") };
        assert!(why.starts_with("`vps` runs Linux aarch64, while this computer’s `silt` is built for Linux x86_64"), "{why}");
        assert!(why.contains("`cargo install --locked --git https://github.com/Urcra/silt silt-cli`"));
        let windows = Platform { os: "windows".into(), arch: "x86_64".into() };
        assert!(matches!(plan(&windows, &linux("x86_64"), "vps"), Plan::Cannot(_)));
    }

    #[test]
    fn what_an_install_says() {
        assert_eq!(shown_path("/home/me/.local/bin/silt", Some("/home/me")), "~/.local/bin/silt");
        assert_eq!(shown_path("/home/meg/silt", Some("/home/me")), "/home/meg/silt");
        assert_eq!(progress(41_000_000, 95_100_000), "41 of 96 MB");
        assert_eq!(progress(0, 0), "0 of 0 MB");
        assert_eq!(
            wont_run_of(
                "/home/me/.local/bin/.silt.new: /lib/x86_64-linux-gnu/libc.so.6: version `GLIBC_2.39' not found (required by /home/me/.local/bin/.silt.new)\n"
            ),
            "it needs `GLIBC_2.39`, newer than the system there"
        );
        assert_eq!(wont_run_of("sh: 1: /home/me/.local/bin/.silt.new: Exec format error\n"), "exec format error");
        assert_eq!(restarted("restarted\n"), Restarted::Now);
        assert_eq!(restarted("restarts once no agent works in it: 2 agents work in it now\n"), Restarted::Waiting(2));
        assert_eq!(restarted("restarts once no agent works in it: an agent works in it now\n"), Restarted::Waiting(1));
        assert_eq!(restarted("not running\n"), Restarted::NotRunning);
        assert_eq!(
            restarted("the daemon runs the build installed\n"),
            Restarted::Unchanged("the daemon runs the build installed".into())
        );
        assert_eq!(Restarted::Waiting(2).said(), "Its daemon restarts on it once its 2 agents finish");
        let wont = Failure::WontRun("it needs `GLIBC_2.39`, newer than the system there".into()).explain(&vps());
        assert!(wont.starts_with("This computer’s `silt` doesn’t run on `vps.example.com`: it needs `GLIBC_2.39`"), "{wont}");
        let unwritable = Failure::NotWritable { path: "/usr/local/bin/silt".into(), user: Some("ubuntu".into()) }.explain(&vps());
        assert!(unwritable.starts_with("`/usr/local/bin/silt` on `vps.example.com` isn’t `ubuntu`’s to replace"), "{unwritable}");
    }

    #[test]
    fn an_install_s_report_lays_out_its_steps_as_it_learns_what_it_does() {
        let mut report = TestReport::install(&vps());
        report.update(Step::SignIn, StepState::Done("Signed in".into()));
        report.lay_out(&INSTALL_STEPS);
        report.update(Step::Copy, StepState::Doing("Copying… 1 of 2 MB".into()));
        let json = report.json();
        assert_eq!(json["action"], "install");
        let steps: Vec<_> =
            json["steps"].as_array().expect("steps").iter().map(|step| (step["state"].clone(), step["text"].clone())).collect();
        assert_eq!(steps.len(), 6);
        assert_eq!(steps[0], ("done".into(), "Signed in".into()));
        assert_eq!(steps[1], ("waiting".into(), "Checking what `vps.example.com` runs".into()));
        assert_eq!(steps[2], ("running".into(), "Copying… 1 of 2 MB".into()));
        assert_eq!(json["canInstall"], false);
    }

    /// The scripts an install runs there, run here by `sh` as ssh would.
    #[cfg(unix)]
    #[test]
    fn a_copy_is_checked_beside_its_place_then_renamed_into_it() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("silt-desktop-copy-{}", std::process::id()));
        let target = dir.join("home/.local/bin/silt");
        let target_text = target.to_string_lossy().into_owned();
        let sh = |line: &str, input: &[u8]| {
            let mut child = Command::new("sh")
                .args(["-c", line])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("sh runs");
            let _ = child.stdin.take().expect("stdin").write_all(input);
            child.wait_with_output().expect("sh ran")
        };
        let copied = sh(&copy_line(&target_text), b"#!/bin/sh\necho 'silt 0.0.2 (nightly b2c3d4e)'\n");
        assert!(copied.status.success(), "{copied:?}");
        let new = dir.join("home/.local/bin/.silt.new");
        assert!(new.is_file() && !target.exists(), "copied beside it, not over it");
        let checked = sh(&check_line(&target_text), b"");
        assert_eq!(version_of(&String::from_utf8_lossy(&checked.stdout)).as_deref(), Some("0.0.2 (nightly b2c3d4e)"));
        assert!(sh(&place_line(&[&target_text]), b"").status.success());
        assert!(target.is_file() && !new.exists());

        sh(&copy_line(&target_text), b"x");
        assert!(sh(&discard_line(&[&target_text]), b"").status.success());
        assert!(!new.exists(), "discarded");

        let locked = dir.join("locked");
        std::fs::create_dir_all(&locked).expect("a temporary dir");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).expect("read-only");
        let refused = sh(&copy_line(&locked.join("silt").to_string_lossy()), b"x");
        // Root writes anywhere, so only someone else sees it refused.
        if !String::from_utf8_lossy(&sh("id -u", b"").stdout).trim().eq("0") {
            assert_eq!(refused.status.code(), Some(3));
            assert!(String::from_utf8_lossy(&refused.stderr).contains("silt-unwritable"));
        }
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).expect("writable");
        std::fs::remove_dir_all(&dir).expect("cleaned up");
    }
}
