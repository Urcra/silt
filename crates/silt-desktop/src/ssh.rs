//! Reaching the machines the person added over SSH (ADR 0108): the list
//! kept in the app's data dir, and each machine's connection: a probe for
//! its `silt`, its daemon started there, and a tunnel that forwards its
//! port here. A tunnel stays up until it's disconnected, the machine is
//! removed or changed, or the app quits.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use tauri::Url;

use crate::daemon::{self, Answer};
use crate::machines::{self, Failure, Machine, Step, StepState, TestReport};

/// The file in the app's data dir that keeps the machines.
pub const MACHINES: &str = "machines.json";

/// How long the tunnel may take to answer once started.
const FORWARD_WAIT: Duration = Duration::from_secs(15);

/// What the page is told when something changed.
pub enum Event {
    /// The list or a machine's connection changed.
    Changed,
    /// A test moved on a step.
    Tested(serde_json::Value),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    Idle,
    Connecting(String),
    Connected,
    Failed(String),
}

struct Live {
    state: State,
    /// When the state last changed, in milliseconds since the epoch.
    since: u64,
    version: Option<String>,
    url: Option<Url>,
    tunnel: Option<Child>,
}

impl Live {
    fn idle() -> Self {
        Self { state: State::Idle, since: now(), version: None, url: None, tunnel: None }
    }

    fn close(&mut self) {
        if let Some(mut tunnel) = self.tunnel.take() {
            let _ = tunnel.kill();
            let _ = tunnel.wait();
        }
        self.url = None;
    }
}

#[derive(Default)]
struct Inner {
    list: Vec<Machine>,
    live: HashMap<String, Live>,
    /// One connection at a time to each machine.
    busy: HashMap<String, Arc<Mutex<()>>>,
}

pub struct Machines {
    file: Option<PathBuf>,
    /// The shared connections' sockets, where ssh can keep them.
    control: Option<PathBuf>,
    inner: Mutex<Inner>,
    tell: OnceLock<Box<dyn Fn(Event) + Send + Sync>>,
}

/// What reaching a machine left: its UI, and the tunnel to it unless the
/// one it had still forwards it.
struct Reached {
    url: Url,
    version: Option<String>,
    tunnel: Option<Child>,
}

impl Machines {
    /// The machines kept in `dir`, if the app has one.
    pub fn load(dir: Option<&Path>) -> Self {
        let file = dir.map(|dir| dir.join(MACHINES));
        let list =
            file.as_deref().and_then(|file| fs::read_to_string(file).ok()).map(|text| machines::remembered_machines(&text));
        Self {
            file,
            control: dir.and_then(control_dir),
            inner: Mutex::new(Inner { list: list.unwrap_or_default(), ..Inner::default() }),
            tell: OnceLock::new(),
        }
    }

    /// Where changes are told, once the window is there to tell.
    pub fn on_event(&self, tell: impl Fn(Event) + Send + Sync + 'static) {
        let _ = self.tell.set(Box::new(tell));
    }

    fn tell(&self, event: Event) {
        if let Some(tell) = self.tell.get() {
            tell(event);
        }
    }

    pub fn names(&self) -> Vec<String> {
        self.inner.lock().expect("lock").list.iter().map(|machine| machine.name.clone()).collect()
    }

    pub fn get(&self, name: &str) -> Option<Machine> {
        self.inner.lock().expect("lock").list.iter().find(|machine| machine.name == name).cloned()
    }

    /// Each machine as the page lists it.
    pub fn entries(&self) -> Vec<serde_json::Value> {
        let inner = self.inner.lock().expect("lock");
        inner
            .list
            .iter()
            .map(|machine| {
                let live = inner.live.get(&machine.name);
                let (state, reason, step) = match live.map(|live| &live.state) {
                    None | Some(State::Idle) => ("idle", None, None),
                    Some(State::Connecting(step)) => ("connecting", None, Some(step)),
                    Some(State::Connected) => ("connected", None, None),
                    Some(State::Failed(reason)) => ("failed", Some(reason), None),
                };
                serde_json::json!({
                    "id": format!("ssh:{}", machine.name),
                    "label": machine.name,
                    "kind": "ssh",
                    "destination": machine.destination,
                    "port": machine.port,
                    "state": state,
                    "reason": reason,
                    "step": step,
                    "since": live.map(|live| live.since),
                    "lastShown": machine.last_shown,
                    "version": live.and_then(|live| live.version.as_ref()),
                })
            })
            .collect()
    }

    fn save(&self, list: &[Machine]) {
        let Some(file) = &self.file else {
            return;
        };
        let written =
            file.parent().map_or(Ok(()), fs::create_dir_all).and_then(|()| fs::write(file, machines::remember_machines(list)));
        if let Err(error) = written {
            eprintln!("silt-desktop: could not keep the machines: {error}");
        }
    }

    fn set_state(&self, name: &str, state: State) {
        {
            let mut inner = self.inner.lock().expect("lock");
            let live = inner.live.entry(name.to_owned()).or_insert_with(Live::idle);
            live.state = state;
            live.since = now();
        }
        self.tell(Event::Changed);
    }

    fn busy(&self, name: &str) -> Arc<Mutex<()>> {
        self.inner.lock().expect("lock").busy.entry(name.to_owned()).or_default().clone()
    }

    /// Adds `machine`, unless one has its name already.
    pub fn add(&self, machine: Machine) -> bool {
        let mut inner = self.inner.lock().expect("lock");
        if inner.list.iter().any(|known| known.name == machine.name) {
            return false;
        }
        inner.list.push(Machine { last_shown: None, ..machine });
        self.save(&inner.list);
        drop(inner);
        self.tell(Event::Changed);
        true
    }

    /// Replaces the machine named `was` with `machine`, closing its
    /// connection, which went to where it used to point.
    pub fn change(&self, was: &str, machine: Machine) -> bool {
        let mut inner = self.inner.lock().expect("lock");
        let taken = machine.name != was && inner.list.iter().any(|known| known.name == machine.name);
        let Some(at) = inner.list.iter().position(|known| known.name == was).filter(|_| !taken) else {
            return false;
        };
        let last_shown = inner.list[at].last_shown;
        inner.list[at] = Machine { last_shown, ..machine };
        if let Some(mut live) = inner.live.remove(was) {
            live.close();
        }
        self.save(&inner.list);
        drop(inner);
        self.tell(Event::Changed);
        true
    }

    pub fn remove(&self, name: &str) {
        let mut inner = self.inner.lock().expect("lock");
        inner.list.retain(|known| known.name != name);
        if let Some(mut live) = inner.live.remove(name) {
            live.close();
        }
        self.save(&inner.list);
        drop(inner);
        self.tell(Event::Changed);
    }

    pub fn disconnect(&self, name: &str) {
        if let Some(live) = self.inner.lock().expect("lock").live.get_mut(name) {
            live.close();
            live.state = State::Idle;
            live.since = now();
        }
        self.tell(Event::Changed);
    }

    /// Closes every tunnel, as the app quits.
    pub fn disconnect_all(&self) {
        for live in self.inner.lock().expect("lock").live.values_mut() {
            live.close();
        }
    }

    /// Notes that the window shows the machine now.
    pub fn shown(&self, name: &str) {
        let mut inner = self.inner.lock().expect("lock");
        if let Some(machine) = inner.list.iter_mut().find(|machine| machine.name == name) {
            machine.last_shown = Some(now());
            self.save(&inner.list);
        }
    }

    /// The machine's web UI here, connecting to it unless it's connected.
    pub fn connect(&self, name: &str) -> Result<Url, String> {
        let busy = self.busy(name);
        let _one = busy.lock().expect("lock");
        let Some(machine) = self.get(name) else {
            return Err(format!("`{name}` isn't among the machines any more"));
        };
        let live_url = self.inner.lock().expect("lock").live.get(name).and_then(|live| live.url.clone());
        if let Some(url) = live_url
            && daemon::check(&url, Duration::from_secs(3)) == Answer::Up
        {
            return Ok(url);
        }
        let reached = self.reach(&machine, true, &mut |step, state| {
            if let StepState::Running = state {
                self.set_state(name, State::Connecting(format!("{}…", step.doing(&machine))));
            }
        });
        self.settle(&machine, reached)
    }

    /// Keeps what reaching the machine left as its connection.
    fn settle(&self, machine: &Machine, reached: Result<Reached, Failure>) -> Result<Url, String> {
        let mut inner = self.inner.lock().expect("lock");
        let live = inner.live.entry(machine.name.clone()).or_insert_with(Live::idle);
        let result = match reached {
            Ok(reached) => {
                if let Some(tunnel) = reached.tunnel {
                    live.close();
                    live.tunnel = Some(tunnel);
                }
                live.url = Some(reached.url.clone());
                live.version = reached.version;
                live.state = State::Connected;
                Ok(reached.url)
            }
            Err(failure) => {
                live.close();
                let reason = failure.explain(machine);
                live.state = State::Failed(reason.clone());
                Err(reason)
            }
        };
        live.since = now();
        drop(inner);
        self.tell(Event::Changed);
        result
    }

    /// Tries each step of a connection to `machine`, telling the page how
    /// each goes. A machine in the list keeps the connection; one that
    /// isn't is added once the steps pass if `add` is set, else let go.
    pub fn test(&self, machine: &Machine, add: bool) {
        let mut report = TestReport::new(machine);
        let (listed, taken) = {
            let inner = self.inner.lock().expect("lock");
            let listed = inner.list.iter().any(|known| Machine { last_shown: None, ..known.clone() } == *machine);
            (listed, !listed && inner.list.iter().any(|known| known.name == machine.name))
        };
        if add && taken {
            report.update(Step::SignIn, StepState::Failed(format!("There's already a machine named `{}`.", machine.name)));
            report.finish(false);
            return self.tell(Event::Tested(report.json()));
        }
        let busy = self.busy(&machine.name);
        let _one = busy.lock().expect("lock");
        let reached = self.reach(machine, listed, &mut |step, state| {
            if listed && let StepState::Running = state {
                self.set_state(&machine.name, State::Connecting(format!("{}…", step.doing(machine))));
            }
            report.update(step, state);
            self.tell(Event::Tested(report.json()));
        });
        let ok = reached.is_ok();
        if listed {
            let _ = self.settle(machine, reached);
        } else if let Ok(mut reached) = reached {
            if add && self.add(machine.clone()) {
                let _ = self.settle(machine, Ok(reached));
            } else if let Some(tunnel) = &mut reached.tunnel {
                let _ = tunnel.kill();
                let _ = tunnel.wait();
            }
        }
        report.finish(ok);
        self.tell(Event::Tested(report.json()));
    }

    /// Signs in, finds `silt`, starts its daemon and forwards its port.
    /// `listed` is whether the machine's connection is the one it has, which
    /// is kept when it forwards the port the daemon runs on still.
    fn reach(&self, machine: &Machine, listed: bool, step: &mut dyn FnMut(Step, StepState)) -> Result<Reached, Failure> {
        let control = self.control.as_deref();
        let failed = |step: &mut dyn FnMut(Step, StepState), at: Step, failure: Failure| {
            step(at, StepState::Failed(failure.explain(machine)));
            Err(failure)
        };
        step(Step::SignIn, StepState::Running);
        let probe = match run(machines::run_on(machine, control, &machines::probe_line())) {
            Ok(output) => output,
            Err(failure) => return failed(step, Step::SignIn, failure),
        };
        let found = machines::probed(&probe);
        let user = found.user.as_deref().map_or(String::new(), |user| format!(" as `{user}`"));
        step(Step::SignIn, StepState::Done(format!("Signed in to `{}`{user}", machine.host())));
        step(Step::Find, StepState::Running);
        let Some(silt) = found.silt else {
            return failed(step, Step::Find, Failure::NoSilt);
        };
        let version = found.version;
        let with_version = version.as_deref().map_or(String::new(), |version| format!(", {version}"));
        step(Step::Find, StepState::Done(format!("Found `{silt}`{with_version}")));
        step(Step::Start, StepState::Running);
        let opened = run(machines::run_on(machine, control, &machines::in_their_shell(&silt, &["open", "--no-browser"])))
            .and_then(|stdout| daemon::web_url(&stdout).map_err(|error| Failure::Other(error.to_string())));
        let url = match opened {
            Ok(url) => url,
            Err(failure) => return failed(step, Step::Start, failure),
        };
        let port = url.port().expect("the UI's address has a port");
        step(Step::Start, StepState::Done("Started its daemon".to_owned()));
        step(Step::Forward, StepState::Running);
        let forwarding = listed && {
            let mut inner = self.inner.lock().expect("lock");
            inner.live.get_mut(&machine.name).is_some_and(|live| {
                let alive = live.tunnel.as_mut().is_some_and(|tunnel| matches!(tunnel.try_wait(), Ok(None)));
                alive && live.url.as_ref().and_then(Url::port) == Some(port)
            })
        };
        let tunnel = if forwarding && daemon::check(&url, Duration::from_secs(3)) == Answer::Up {
            None
        } else {
            if listed && let Some(live) = self.inner.lock().expect("lock").live.get_mut(&machine.name) {
                live.close();
            }
            match forward(machine, &url) {
                Ok(tunnel) => Some(tunnel),
                Err(failure) => return failed(step, Step::Forward, failure),
            }
        };
        step(Step::Forward, StepState::Done(format!("Forwarded port `{port}` to this computer")));
        Ok(Reached { url, version, tunnel })
    }

    /// Notes tunnels that closed, as when the machine went away, so the
    /// page doesn't show them connected.
    pub fn watch(self: &Arc<Self>) {
        let machines = Arc::downgrade(self);
        std::thread::spawn(move || {
            while let Some(machines) = machines.upgrade() {
                let mut closed = false;
                for live in machines.inner.lock().expect("lock").live.values_mut() {
                    if let Some(tunnel) = &mut live.tunnel
                        && !matches!(tunnel.try_wait(), Ok(None))
                    {
                        live.close();
                        live.state = State::Idle;
                        live.since = now();
                        closed = true;
                    }
                }
                if closed {
                    machines.tell(Event::Changed);
                }
                drop(machines);
                std::thread::sleep(Duration::from_secs(2));
            }
        });
    }
}

/// Runs an `ssh` command and returns what it printed, or why it failed.
fn run(mut command: Command) -> Result<String, Failure> {
    daemon::hide_console(&mut command);
    let output = command.stdin(Stdio::null()).output().map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => Failure::NoSsh,
        _ => Failure::Other(format!("could not run ssh: {error}")),
    })?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(machines::failure_of(&String::from_utf8_lossy(&output.stderr), output.status.code()))
    }
}

/// Starts the tunnel to the daemon at `url` and waits until it answers
/// through it.
fn forward(machine: &Machine, url: &Url) -> Result<Child, Failure> {
    let port = url.port().expect("the UI's address has a port");
    // ssh would say so too, but only after signing in.
    TcpListener::bind(("127.0.0.1", port)).map_err(|_| Failure::PortInUse(port))?;
    let mut command = machines::tunnel(machine, port);
    daemon::hide_console(&mut command);
    let mut tunnel = command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().map_err(|error| {
        match error.kind() {
            io::ErrorKind::NotFound => Failure::NoSsh,
            _ => Failure::Other(format!("could not run ssh: {error}")),
        }
    })?;
    // Read as it comes, so a long-lived tunnel's warnings never fill the pipe.
    let said = Arc::new(Mutex::new(String::new()));
    if let Some(mut stderr) = tunnel.stderr.take() {
        let said = said.clone();
        std::thread::spawn(move || {
            let mut chunk = [0; 1024];
            while let Ok(read @ 1..) = stderr.read(&mut chunk) {
                let mut said = said.lock().expect("lock");
                if said.len() < 16 * 1024 {
                    said.push_str(&String::from_utf8_lossy(&chunk[..read]));
                }
            }
        });
    }
    let deadline = Instant::now() + FORWARD_WAIT;
    loop {
        if let Ok(Some(status)) = tunnel.try_wait() {
            // Give the reader the last of what ssh said.
            std::thread::sleep(Duration::from_millis(50));
            return Err(match machines::failure_of(&said.lock().expect("lock"), status.code().or(Some(255))) {
                Failure::PortInUse(_) => Failure::PortInUse(port),
                failure => failure,
            });
        }
        if daemon::check(url, Duration::from_secs(1)) == Answer::Up {
            return Ok(tunnel);
        }
        if Instant::now() > deadline {
            let _ = tunnel.kill();
            let _ = tunnel.wait();
            return Err(Failure::Other(format!("The daemon on `{}` didn't answer through the tunnel.", machine.host())));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// The directory for the shared connections' sockets, private to the
/// person. None where ssh can't share them, or the path would be too long
/// for a socket's.
fn control_dir(dir: &Path) -> Option<PathBuf> {
    if cfg!(windows) {
        return None;
    }
    let control = dir.join("ssh");
    // `%C` expands to 40 characters, and a socket's path holds about 100.
    if control.as_os_str().len() + 41 > 100 {
        return None;
    }
    fs::create_dir_all(&control).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&control, fs::Permissions::from_mode(0o700)).ok()?;
    }
    Some(control)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Adds, connects to, disconnects from and removes a real machine, when
    /// `SILT_TEST_SSH` names one with `silt`, as `user@host` or a `Host`.
    #[test]
    fn a_machine_is_tested_added_connected_and_let_go() {
        let Some(destination) = std::env::var("SILT_TEST_SSH").ok().filter(|name| !name.is_empty()) else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("silt-desktop-ssh-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("a temporary dir");
        let machines = Arc::new(Machines::load(Some(&dir)));
        let reports = Arc::new(Mutex::new(Vec::new()));
        let told = reports.clone();
        machines.on_event(move |event| {
            if let Event::Tested(report) = event {
                told.lock().expect("lock").push(report);
            }
        });
        let vps = Machine { name: "vps".into(), destination, port: None, last_shown: None };

        machines.test(&vps, true);
        let last = reports.lock().expect("lock").last().cloned().expect("a report");
        assert_eq!(last["ok"], true, "{last:#}");
        assert!(last["steps"].as_array().expect("steps").iter().all(|step| step["state"] == "done"), "{last:#}");
        assert_eq!(machines.names(), ["vps"]);
        assert_eq!(machines.entries()[0]["state"], "connected");
        let remembered = fs::read_to_string(dir.join(MACHINES)).expect("kept");
        assert_eq!(machines::remembered_machines(&remembered), std::slice::from_ref(&vps));

        let url = machines.connect("vps").expect("connects");
        assert_eq!(daemon::check(&url, Duration::from_secs(3)), Answer::Up);
        assert_eq!(machines.connect("vps").expect("still connected"), url, "the connection is kept");

        machines.disconnect("vps");
        assert_eq!(daemon::check(&url, Duration::from_secs(3)), Answer::Gone, "the tunnel closed");
        assert_eq!(machines.entries()[0]["state"], "idle");
        let again = machines.connect("vps").expect("connects again");
        assert_eq!(daemon::check(&again, Duration::from_secs(3)), Answer::Up);

        machines.remove("vps");
        assert_eq!(daemon::check(&again, Duration::from_secs(3)), Answer::Gone);
        assert!(machines.names().is_empty());
        assert!(machines.connect("vps").is_err());

        let nowhere = Machine { name: "nowhere".into(), destination: "nobody@127.0.0.1".into(), port: Some(1), last_shown: None };
        reports.lock().expect("lock").clear();
        machines.test(&nowhere, true);
        let last = reports.lock().expect("lock").last().cloned().expect("a report");
        assert_eq!(last["ok"], false);
        assert_eq!(last["steps"][0]["state"], "failed", "{last:#}");
        assert!(machines.names().is_empty(), "a machine that fails its test isn't added");
        fs::remove_dir_all(&dir).expect("cleaned up");
    }
}
