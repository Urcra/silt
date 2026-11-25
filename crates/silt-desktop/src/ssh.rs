//! Reaching the machines the person added over SSH (ADR 0108): the list
//! kept in the app's data dir, and each machine's connection: a probe for
//! its `silt`, its daemon started there, and a tunnel that forwards its
//! port here. A tunnel stays up until it's disconnected, the machine is
//! removed or changed, or the app quits.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use tauri::Url;

use crate::daemon::{self, Answer};
use crate::machines::{self, Failure, Machine, Plan, Platform, Step, StepState, TestReport};

/// The file in the app's data dir that keeps the machines.
pub const MACHINES: &str = "machines.json";

/// How long the tunnel may take to answer once started.
const FORWARD_WAIT: Duration = Duration::from_secs(15);

/// The sandbox's programs, which `silt` finds beside it (ADR 0093).
const BESIDE_SILT: [&str; 3] = ["silt-bwrap", "silt-pasta", "silt-pasta.avx2"];

/// How often a copy says how far it got.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

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
    /// `silt` is being installed or updated there.
    installing: bool,
    /// It failed for want of `silt` there, which Silt can install.
    no_silt: bool,
    /// An update's daemon restarts once this many agents finish.
    pending: Option<usize>,
}

impl Live {
    fn idle() -> Self {
        Self {
            state: State::Idle,
            since: now(),
            version: None,
            url: None,
            tunnel: None,
            installing: false,
            no_silt: false,
            pending: None,
        }
    }

    fn close(&mut self) {
        if let Some(mut tunnel) = self.tunnel.take() {
            let _ = tunnel.kill();
            let _ = tunnel.wait();
        }
        self.url = None;
        // The restart it waited for happens with no tunnel to see it.
        self.pending = None;
    }
}

#[derive(Default)]
struct Inner {
    list: Vec<Machine>,
    live: HashMap<String, Live>,
    /// One connection at a time to each machine.
    busy: HashMap<String, Arc<Mutex<()>>>,
}

/// This computer's `silt`, which a machine gets (ADR 0109).
struct Local {
    program: PathBuf,
    version: String,
}

pub struct Machines {
    file: Option<PathBuf>,
    local: OnceLock<Local>,
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
    pending: Option<usize>,
}

impl Machines {
    /// The machines kept in `dir`, if the app has one.
    pub fn load(dir: Option<&Path>) -> Self {
        let file = dir.map(|dir| dir.join(MACHINES));
        let list =
            file.as_deref().and_then(|file| fs::read_to_string(file).ok()).map(|text| machines::remembered_machines(&text));
        Self {
            file,
            local: OnceLock::new(),
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

    /// Learns this computer's `silt` and its version, which takes running
    /// it, so it's done once, off the window's thread.
    pub fn learn_local(&self, program: &Path) {
        let Some(program) = on_path(program) else {
            return;
        };
        let program = fs::canonicalize(&program).unwrap_or(program);
        let mut command = Command::new(&program);
        daemon::hide_console(&mut command);
        let Ok(output) = command.arg("--version").stdin(Stdio::null()).stderr(Stdio::null()).output() else {
            return;
        };
        if let Some(version) = machines::version_of(&String::from_utf8_lossy(&output.stdout)).filter(|_| output.status.success())
        {
            let _ = self.local.set(Local { program, version });
            self.tell(Event::Changed);
        }
    }

    /// The version of this computer's `silt`, once learned.
    pub fn local_version(&self) -> Option<&str> {
        self.local.get().map(|local| &*local.version)
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
                    "installing": live.is_some_and(|live| live.installing),
                    "canInstall": live.is_some_and(|live| live.no_silt && matches!(live.state, State::Failed(_))),
                    "pendingRestart": live.and_then(|live| live.pending),
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
        live.installing = false;
        let result = match reached {
            Ok(reached) => {
                if let Some(tunnel) = reached.tunnel {
                    live.close();
                    live.tunnel = Some(tunnel);
                }
                live.url = Some(reached.url.clone());
                live.version = reached.version;
                live.pending = reached.pending;
                live.no_silt = false;
                live.state = State::Connected;
                Ok(reached.url)
            }
            Err(failure) => {
                live.close();
                live.pending = None;
                live.no_silt = failure == Failure::NoSilt;
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
        let (listed, taken) = self.listed(machine);
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
        if matches!(reached, Err(Failure::NoSilt)) {
            report.offer_install();
        }
        if listed {
            let _ = self.settle(machine, reached);
        } else {
            self.keep_if_added(machine, add, reached);
        }
        report.finish(ok);
        self.tell(Event::Tested(report.json()));
    }

    /// Whether `machine` is in the list as it is, and else whether another
    /// has its name.
    fn listed(&self, machine: &Machine) -> (bool, bool) {
        let inner = self.inner.lock().expect("lock");
        let listed = inner.list.iter().any(|known| Machine { last_shown: None, ..known.clone() } == *machine);
        (listed, !listed && inner.list.iter().any(|known| known.name == machine.name))
    }

    /// Adds a machine that isn't listed once it's reached, if asked to, and
    /// else lets its connection go.
    fn keep_if_added(&self, machine: &Machine, add: bool, reached: Result<Reached, Failure>) {
        if let Ok(mut reached) = reached {
            if add && self.add(machine.clone()) {
                let _ = self.settle(machine, Ok(reached));
            } else if let Some(tunnel) = &mut reached.tunnel {
                let _ = tunnel.kill();
                let _ = tunnel.wait();
            }
        }
    }

    /// Puts this computer's `silt` on `machine`, in place of the one it has
    /// if any, restarts its daemon on it, and connects, telling the page
    /// each step (ADR 0109). A machine that isn't listed is added once
    /// connected if `add` is set. An update that fails before it changed
    /// anything leaves a connected machine connected.
    pub fn install(&self, machine: &Machine, add: bool) {
        let mut report = TestReport::install(machine);
        let (listed, taken) = self.listed(machine);
        if add && taken {
            report.update(Step::SignIn, StepState::Failed(format!("There's already a machine named `{}`.", machine.name)));
            report.finish(false);
            return self.tell(Event::Tested(report.json()));
        }
        let busy = self.busy(&machine.name);
        let _one = busy.lock().expect("lock");
        if listed {
            self.set_installing(&machine.name, true);
        }
        let reached = self.put(machine, listed, &mut report);
        let ok = reached.is_ok();
        if listed {
            if reached.is_err() && self.still_connected(&machine.name) {
                self.set_installing(&machine.name, false);
            } else {
                let _ = self.settle(machine, reached);
            }
        } else {
            self.keep_if_added(machine, add, reached);
        }
        report.finish(ok);
        self.tell(Event::Tested(report.json()));
    }

    fn told(&self, report: &mut TestReport, step: Step, state: StepState) {
        report.update(step, state);
        self.tell(Event::Tested(report.json()));
    }

    fn set_installing(&self, name: &str, installing: bool) {
        self.inner.lock().expect("lock").live.entry(name.to_owned()).or_insert_with(Live::idle).installing = installing;
        self.tell(Event::Changed);
    }

    /// Whether the machine's tunnel still reaches its daemon.
    fn still_connected(&self, name: &str) -> bool {
        let url = {
            let mut inner = self.inner.lock().expect("lock");
            let Some(live) = inner.live.get_mut(name).filter(|live| live.state == State::Connected) else {
                return false;
            };
            let alive = live.tunnel.as_mut().is_some_and(|tunnel| matches!(tunnel.try_wait(), Ok(None)));
            live.url.clone().filter(|_| alive)
        };
        url.is_some_and(|url| daemon::check(&url, Duration::from_secs(3)) == Answer::Up)
    }

    /// The steps of [`Self::install`].
    fn put(&self, machine: &Machine, listed: bool, report: &mut TestReport) -> Result<Reached, Failure> {
        let control = self.control.as_deref();
        let host = machine.host();
        let fail = |report: &mut TestReport, at: Step, failure: Failure| {
            self.told(report, at, StepState::Failed(failure.explain(machine)));
            Err(failure)
        };
        let Some(local) = self.local.get() else {
            return fail(
                report,
                Step::SignIn,
                Failure::Other("Couldn't run this computer's `silt`, to install it there.".to_owned()),
            );
        };
        self.told(report, Step::SignIn, StepState::Running);
        let found = match run(machines::run_on(machine, control, &machines::probe_line())) {
            Ok(probe) => machines::probed(&probe),
            Err(failure) => return fail(report, Step::SignIn, failure),
        };
        let user = found.user.as_deref().map_or(String::new(), |user| format!(" as `{user}`"));
        self.told(report, Step::SignIn, StepState::Done(format!("Signed in to `{host}`{user}")));
        let home = found.home.as_deref();
        let updating = found.silt.is_some();
        let (target, planned_at) = if let Some(silt) = &found.silt {
            report.lay_out(&machines::UPDATE_STEPS);
            let version = found.version.as_deref().map_or(String::new(), |version| format!(", {version}"));
            self.told(report, Step::Find, StepState::Done(format!("Found `{silt}`{version}")));
            (silt.clone(), Step::Copy)
        } else {
            report.lay_out(&machines::INSTALL_STEPS);
            self.told(report, Step::Platform, StepState::Running);
            let Some(home) = home else {
                return fail(report, Step::Platform, Failure::Other(format!("Couldn't tell where home is on `{host}`.")));
            };
            (format!("{home}/.local/bin/silt"), Step::Platform)
        };
        let Some(there) = &found.platform else {
            return fail(report, planned_at, Failure::Other(format!("Couldn't tell what `{host}` runs.")));
        };
        let plan = machines::plan(&Platform::here(), there, host);
        match &plan {
            Plan::Cannot(why) => return fail(report, planned_at, Failure::Other(why.clone())),
            Plan::Copy if !updating => {
                self.told(report, Step::Platform, StepState::Done(format!("`{host}` runs {there}, as this computer does")));
            }
            _ => {}
        }
        let shown = machines::shown_path(&target, home);
        let dir = target.rsplit_once('/').map_or("/", |(dir, _)| dir).to_owned();
        let version = &local.version;
        match plan {
            Plan::Copy => {
                let mut files = vec![(local.program.clone(), target.clone())];
                if let Some(beside) = local.program.parent() {
                    files.extend(
                        BESIDE_SILT
                            .iter()
                            .map(|name| (beside.join(name), format!("{dir}/{name}")))
                            .filter(|(file, _)| file.is_file()),
                    );
                }
                let total = files.iter().filter_map(|(file, _)| fs::metadata(file).ok()).map(|meta| meta.len()).sum();
                let copying = |sent| {
                    let progress = machines::progress(sent, total);
                    if updating {
                        format!("Copying silt {version} over it… {progress}")
                    } else {
                        format!("Copying silt {version} to `{shown}`… {progress}")
                    }
                };
                self.told(report, Step::Copy, StepState::Doing(copying(0)));
                let mut sent_before = 0;
                for (file, to) in &files {
                    let mut last = Instant::now();
                    let sent = self.copy(machine, file, to, found.user.clone(), &mut |sent| {
                        if last.elapsed() >= PROGRESS_EVERY {
                            last = Instant::now();
                            self.told(report, Step::Copy, StepState::Doing(copying(sent_before + sent)));
                        }
                    });
                    match sent {
                        Ok(sent) => sent_before += sent,
                        Err(failure) => {
                            let targets: Vec<&str> = files.iter().map(|(_, to)| &**to).collect();
                            let _ = run(machines::run_on(machine, control, &machines::discard_line(&targets)));
                            return fail(report, Step::Copy, failure);
                        }
                    }
                }
                let copied = if updating {
                    format!("Copied silt {version} over `{shown}`")
                } else {
                    format!("Copied silt {version} to `{shown}`")
                };
                self.told(report, Step::Copy, StepState::Done(copied));
                self.told(report, Step::Check, StepState::Running);
                let targets: Vec<&str> = files.iter().map(|(_, to)| &**to).collect();
                let checked = exchange(machines::run_on(machine, control, &machines::check_line(&target)), None, &mut |_| {});
                let ran = match checked {
                    Ok(output) if output.status.success() => {
                        let said = machines::version_of(&String::from_utf8_lossy(&output.stdout));
                        if said.as_deref() == Some(version) {
                            Ok(())
                        } else {
                            Err(Failure::WontRun(format!("it says it's {}", said.as_deref().unwrap_or("nothing"))))
                        }
                    }
                    Ok(output) if output.status.code() != Some(255) => {
                        Err(Failure::WontRun(machines::wont_run_of(&String::from_utf8_lossy(&output.stderr))))
                    }
                    Ok(output) => Err(machines::failure_of(&String::from_utf8_lossy(&output.stderr), output.status.code())),
                    Err(failure) => Err(failure),
                };
                let placed =
                    ran.and_then(|()| run(machines::run_on(machine, control, &machines::place_line(&targets))).map(drop));
                if let Err(failure) = placed {
                    let _ = run(machines::run_on(machine, control, &machines::discard_line(&targets)));
                    return fail(report, Step::Check, failure);
                }
                self.told(report, Step::Check, StepState::Done(format!("It runs there: silt {version}")));
            }
            Plan::Cannot(_) => unreachable!("failed above"),
        }
        let (url, pending, start) = if updating {
            self.told(report, Step::Restart, StepState::Running);
            let line = machines::in_their_shell(&target, &["daemon", "restart", "--if-replaced"]);
            let restarted = match run(machines::run_on(machine, control, &line)) {
                Ok(said) => machines::restarted(&said),
                Err(failure) => return fail(report, Step::Restart, failure),
            };
            let pending = match restarted {
                machines::Restarted::Waiting(agents) => Some(agents),
                _ => None,
            };
            (self.open_there(machine, &target), pending, (Step::Restart, restarted.said()))
        } else {
            self.told(report, Step::Start, StepState::Running);
            (self.open_there(machine, &target), None, (Step::Start, "Started its daemon".to_owned()))
        };
        let url = match url {
            Ok(url) => url,
            Err(failure) => return fail(report, start.0, failure),
        };
        self.told(report, start.0, StepState::Done(start.1));
        self.told(report, Step::Forward, StepState::Running);
        let tunnel = match self.forwarded(machine, listed, &url) {
            Ok(tunnel) => tunnel,
            Err(failure) => return fail(report, Step::Forward, failure),
        };
        let port = url.port().expect("the UI's address has a port");
        self.told(report, Step::Forward, StepState::Done(format!("Forwarded port `{port}` to this computer")));
        Ok(Reached { url, version: Some(version.clone()), tunnel, pending })
    }

    /// Sends `file` to beside `to` there, saying how much went as it goes,
    /// and returns its size.
    fn copy(
        &self,
        machine: &Machine,
        file: &Path,
        to: &str,
        user: Option<String>,
        sent: &mut dyn FnMut(u64),
    ) -> Result<u64, Failure> {
        let mut from =
            fs::File::open(file).map_err(|error| Failure::Other(format!("Couldn't read `{}`: {error}", file.display())))?;
        let size = from.metadata().map_or(0, |meta| meta.len());
        let output =
            exchange(machines::run_on(machine, self.control.as_deref(), &machines::copy_line(to)), Some(&mut from), sent)?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        match output.status.code() {
            Some(0) => Ok(size),
            Some(3) if stderr.contains("silt-unwritable") => Err(Failure::NotWritable { path: to.to_owned(), user }),
            code => Err(machines::failure_of(&stderr, code)),
        }
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
        let url = match self.open_there(machine, &silt) {
            Ok(url) => url,
            Err(failure) => return failed(step, Step::Start, failure),
        };
        step(Step::Start, StepState::Done("Started its daemon".to_owned()));
        step(Step::Forward, StepState::Running);
        let tunnel = match self.forwarded(machine, listed, &url) {
            Ok(tunnel) => tunnel,
            Err(failure) => return failed(step, Step::Forward, failure),
        };
        let port = url.port().expect("the UI's address has a port");
        step(Step::Forward, StepState::Done(format!("Forwarded port `{port}` to this computer")));
        Ok(Reached { url, version, tunnel, pending: None })
    }

    /// Has `silt` there start its daemon unless one runs, and says where
    /// its UI is.
    fn open_there(&self, machine: &Machine, silt: &str) -> Result<Url, Failure> {
        let line = machines::in_their_shell(silt, &["open", "--no-browser"]);
        run(machines::run_on(machine, self.control.as_deref(), &line))
            .and_then(|stdout| daemon::web_url(&stdout).map_err(|error| Failure::Other(error.to_string())))
    }

    /// The tunnel to the daemon at `url`, or none when the machine's own
    /// still forwards its port.
    fn forwarded(&self, machine: &Machine, listed: bool, url: &Url) -> Result<Option<Child>, Failure> {
        let port = url.port().expect("the UI's address has a port");
        let forwarding = listed && {
            let mut inner = self.inner.lock().expect("lock");
            inner.live.get_mut(&machine.name).is_some_and(|live| {
                let alive = live.tunnel.as_mut().is_some_and(|tunnel| matches!(tunnel.try_wait(), Ok(None)));
                alive && live.url.as_ref().and_then(Url::port) == Some(port)
            })
        };
        if forwarding && daemon::check(url, Duration::from_secs(3)) == Answer::Up {
            return Ok(None);
        }
        if listed && let Some(live) = self.inner.lock().expect("lock").live.get_mut(&machine.name) {
            live.close();
        }
        forward(machine, url).map(Some)
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

/// Runs an `ssh` command with `input` on its stdin, saying how many bytes
/// went as they go, and returns what it did, failed or not.
fn exchange(mut command: Command, input: Option<&mut dyn Read>, sent: &mut dyn FnMut(u64)) -> Result<Output, Failure> {
    daemon::hide_console(&mut command);
    let stdin = if input.is_some() { Stdio::piped() } else { Stdio::null() };
    let mut child =
        command.stdin(stdin).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => Failure::NoSsh,
            _ => Failure::Other(format!("could not run ssh: {error}")),
        })?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        let mut chunk = vec![0; 256 * 1024];
        let mut total = 0;
        loop {
            let read = match input.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Failure::Other(format!("Couldn't read what to send: {error}")));
                }
            };
            // It ended early: what it said says why.
            if stdin.write_all(&chunk[..read]).is_err() {
                break;
            }
            total += read as u64;
            sent(total);
        }
    }
    child.wait_with_output().map_err(|error| Failure::Other(format!("could not run ssh: {error}")))
}

/// `program` as a file: as it is when it's a path, else found on `PATH`.
fn on_path(program: &Path) -> Option<PathBuf> {
    if program.components().count() > 1 {
        return program.is_file().then(|| program.to_owned());
    }
    let named = |dir: PathBuf| {
        let found = dir.join(program);
        // Windows finds `silt.exe` for `silt`.
        let exe = cfg!(windows).then(|| found.with_extension("exe"));
        std::iter::once(found).chain(exe)
    };
    std::env::split_paths(&std::env::var_os("PATH")?).flat_map(named).find(|path| path.is_file())
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

    /// `SILT_TEST_SSH_BARE` names a machine without `silt` whose
    /// `~/.local/bin` this computer's build may go in, and `SILT_TEST_SILT`
    /// the build, else `silt` on `PATH`.
    #[test]
    fn silt_is_installed_on_a_machine_without_it_then_updated() {
        let Some(destination) = std::env::var("SILT_TEST_SSH_BARE").ok().filter(|name| !name.is_empty()) else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("silt-desktop-install-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("a temporary dir");
        let machines = Arc::new(Machines::load(Some(&dir)));
        machines.learn_local(Path::new(&std::env::var("SILT_TEST_SILT").unwrap_or_else(|_| "silt".to_owned())));
        let version = machines.local_version().expect("this computer's build").to_owned();
        let reports = Arc::new(Mutex::new(Vec::new()));
        let told = reports.clone();
        machines.on_event(move |event| {
            if let Event::Tested(report) = event {
                told.lock().expect("lock").push(report);
            }
        });
        let vps = Machine { name: "bare".into(), destination, port: None, last_shown: None };
        let said = |report: &serde_json::Value, starts: &[&str]| {
            let steps = report["steps"].as_array().expect("steps");
            assert_eq!(steps.len(), starts.len(), "{report:#}");
            for (step, start) in steps.iter().zip(starts) {
                assert_eq!(step["state"], "done", "{report:#}");
                assert!(step["text"].as_str().expect("text").starts_with(start), "{report:#}");
            }
        };

        machines.test(&vps, false);
        let tested = reports.lock().expect("lock").last().cloned().expect("a report");
        assert_eq!(tested["canInstall"], true, "{tested:#}");

        machines.install(&vps, true);
        let installed = reports.lock().expect("lock").last().cloned().expect("a report");
        assert_eq!(installed["ok"], true, "{installed:#}");
        assert_eq!(installed["action"], "install");
        let copied = format!("Copied silt {version} to `~/.local/bin/silt`");
        let runs = format!("It runs there: silt {version}");
        let platform = format!("`vps-test` runs {}, as this computer does", Platform::here());
        said(&installed, &["Signed in to `vps-test`", &platform, &copied, &runs, "Started its daemon", "Forwarded port"]);
        let shown = reports.lock().expect("lock").iter().any(|report| {
            report["steps"].as_array().is_some_and(|steps| {
                steps
                    .iter()
                    .any(|step| step["state"] == "running" && step["text"].as_str().is_some_and(|text| text.contains(" MB")))
            })
        });
        assert!(shown, "the copy said how far it got");
        assert_eq!(machines.names(), ["bare"]);
        let entry = &machines.entries()[0];
        assert_eq!(entry["state"], "connected", "{entry:#}");
        assert_eq!(entry["version"], version.as_str());
        let url = machines.connect("bare").expect("connects");
        assert_eq!(daemon::check(&url, Duration::from_secs(3)), Answer::Up);

        reports.lock().expect("lock").clear();
        machines.install(&vps, false);
        let updated = reports.lock().expect("lock").last().cloned().expect("a report");
        assert_eq!(updated["ok"], true, "{updated:#}");
        let copied = format!("Copied silt {version} over `~/.local/bin/silt`");
        said(&updated, &["Signed in to `vps-test`", "Found `", &copied, &runs, "Restarted its daemon on it", "Forwarded port"]);
        assert_eq!(machines.entries()[0]["state"], "connected");
        assert_eq!(daemon::check(&machines.connect("bare").expect("connects"), Duration::from_secs(3)), Answer::Up);

        let stop = machines::run_on(&vps, None, "exec sh -c '\"$HOME/.local/bin/silt\" daemon stop'");
        machines.remove("bare");
        let _ = run(stop);
        fs::remove_dir_all(&dir).expect("cleaned up");
    }
}
