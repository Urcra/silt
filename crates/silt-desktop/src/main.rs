//! Silt's desktop app: the local daemon's web UI in a window (ADR 0041).
//! Recording never depends on it. The daemon runs on its own, and closing
//! the window leaves it and its agents running, but on the build installed
//! since it started, if one was, once no agent works in it. On Windows the
//! window also shows WSL distributions' daemons (ADR 0045), and anywhere the
//! daemons of machines the person reaches over SSH (ADR 0108).

// A console program would open a console window beside the app's own on
// Windows; debug builds keep it for their logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backends;
mod daemon;
mod machines;
#[cfg(target_os = "linux")]
mod microphone;
mod ssh;
mod theme;
mod zoom;

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use exn::Exn;
use tauri::webview::{NewWindowResponse, PageLoadEvent, PermissionKind, PermissionResponse};
use tauri::window::Color;
use tauri::{AppHandle, Manager, Theme, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::backends::{Backend, Target};
use crate::daemon::{Answer, DesktopError};
use crate::machines::Edit;

const MAIN: &str = "main";

/// The backends the window can show: this computer's and WSL's, found at
/// launch, and the machines the person added.
struct Backends {
    fixed: Vec<Backend>,
    machines: Arc<ssh::Machines>,
}

impl Backends {
    fn known(&self) -> Vec<Backend> {
        let mut known = self.fixed.clone();
        known.extend(self.machines.names().into_iter().map(Backend::Ssh));
        known
    }
}

/// What the window's handlers share.
struct State {
    /// Where the daemon's UI is, once known.
    ui: Mutex<Option<Url>>,
    /// Which backend and project the window should show.
    target: Mutex<Target>,
    /// The backend whose UI the window shows, once it shows one.
    shown: Mutex<Option<Backend>>,
    backends: OnceLock<Arc<Backends>>,
    app: OnceLock<AppHandle>,
    /// What links that edit the machines carry, which only the window's
    /// own page is told: a frame in it, such as an artifact's, could send
    /// such a link too.
    key: String,
}

impl State {
    fn known(&self) -> Vec<Backend> {
        self.backends.get().map_or_else(|| vec![Backend::Native], |backends| backends.known())
    }

    /// The backends as the page lists them, each machine with how its
    /// connection stands, and the one the window is switching to marked.
    fn list(&self) -> serde_json::Value {
        let Some(backends) = self.backends.get() else {
            return serde_json::Value::Array(Vec::new());
        };
        let target = self.target.lock().expect("lock").backend.clone();
        let shown = self.shown.lock().expect("lock").clone();
        let mut list: Vec<serde_json::Value> = backends
            .fixed
            .iter()
            .map(|backend| {
                let mut entry = serde_json::json!({ "id": backend.id(), "label": backend.label(), "kind": backend.kind() });
                // What a machine over SSH is offered to update to (ADR 0109).
                if *backend == Backend::Native
                    && let Some(version) = backends.machines.local_version()
                {
                    entry["version"] = version.into();
                }
                entry
            })
            .collect();
        list.extend(backends.machines.entries());
        if shown.is_some_and(|shown| shown != target) {
            let id = serde_json::Value::String(target.id());
            for entry in list.iter_mut().filter(|entry| entry["id"] == id) {
                entry["switching"] = true.into();
            }
        }
        serde_json::Value::Array(list)
    }

    /// Tells the page what changed in the backends.
    fn tell(&self) {
        if let Some(window) = self.app.get().and_then(|app| app.get_webview_window(MAIN)) {
            let _ = window.eval(format!(
                "window.__siltBackends = {}; window.dispatchEvent(new CustomEvent('silt-backends'));",
                self.list()
            ));
        }
    }

    fn on_machines(&self, event: ssh::Event) {
        match event {
            ssh::Event::Changed => self.tell(),
            ssh::Event::Tested(report) => {
                if let Some(window) = self.app.get().and_then(|app| app.get_webview_window(MAIN)) {
                    let _ = window
                        .eval(format!("window.dispatchEvent(new CustomEvent('silt-machine-test', {{ detail: {report} }}));"));
                }
            }
        }
    }

    /// Whether `link` may edit the machines: only the page of this computer
    /// or a WSL distribution may, not a machine's, which mustn't send the
    /// app elsewhere.
    fn may_edit_machines(&self, link: &Url) -> bool {
        let keyed = link.query_pairs().any(|(name, value)| name == "key" && value == self.key);
        let shown = self.shown.lock().expect("lock");
        keyed && shown.as_ref().is_some_and(|shown| !matches!(shown, Backend::Ssh(_)))
    }

    /// Has the window show `to` instead of `name`, if it shows that machine
    /// or is switching to it.
    fn retarget(&self, name: &str, to: Backend) {
        let mut target = self.target.lock().expect("lock");
        if target.backend == Backend::Ssh(name.to_owned()) {
            *target = Target { backend: to, project: None };
        }
    }
}

fn main() {
    // WSL's distributions are listed while the window's own start goes on:
    // `wsl.exe` can take a second, before anything showed.
    let finding = std::thread::spawn(known_backends);
    let args: Vec<String> = env::args().collect();
    let state = Arc::new(State {
        ui: Mutex::default(),
        target: Mutex::default(),
        shown: Mutex::default(),
        backends: OnceLock::new(),
        app: OnceLock::new(),
        key: new_key(),
    });
    let (linked, closing) = (state.clone(), state.clone());
    tauri::Builder::default()
        // A second launch, a `silt://` link among them, shows the window
        // that's already open.
        .plugin(tauri_plugin_single_instance::init(move |app, argv, _| {
            if let Some(wanted) = link_in(argv, &linked.known()) {
                *linked.target.lock().expect("lock") = wanted;
            }
            show(app);
        }))
        .plugin(tauri_plugin_deep_link::init())
        .setup(move |app| {
            let _ = state.app.set(app.handle().clone());
            let dir = app.path().app_data_dir().ok();
            let fixed = finding.join().unwrap_or_else(|_| vec![Backend::Native]);
            let machines = Arc::new(ssh::Machines::load(dir.as_deref()));
            let told = state.clone();
            machines.on_event(move |event| told.on_machines(event));
            machines.watch();
            let learning = machines.clone();
            std::thread::spawn(move || learning.learn_local(&native_silt()));
            let _ = state.backends.set(Arc::new(Backends { fixed, machines }));
            let known = state.known();
            let asked = link_in(args, &known);
            let opened_by_link = asked.is_some();
            if let Some(asked) = asked {
                *state.target.lock().expect("lock") = asked;
            }
            // Opened by itself, the window shows where it last did, not this computer every time.
            if !opened_by_link && let Some(backend) = dir.as_ref().and_then(|dir| last_backend(&dir.join(LAST_BACKEND), &known)) {
                *state.target.lock().expect("lock") = Target { backend, project: None };
            }
            let silts = Arc::new(Silts::load(dir.as_ref().map(|dir| dir.join(WSL_SILTS))));
            app.manage(silts.clone());
            let shown = state.target.lock().expect("lock").backend.clone();
            let (starting, fixed) = (silts.clone(), state.backends.get().expect("set").fixed.clone());
            std::thread::spawn(move || start_the_others(&fixed, &shown, &native_silt(), &starting));
            open_window(app.handle(), state, silts, dir)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("the desktop app could not start")
        .run(move |app, event| {
            if let tauri::RunEvent::Exit = event
                && let Some(backends) = closing.backends.get()
            {
                backends.machines.disconnect_all();
                if let Some(silts) = app.try_state::<Arc<Silts>>() {
                    restart_replaced(&backends.fixed, &native_silt(), &silts);
                }
            }
        });
}

/// A key no page can guess.
fn new_key() -> String {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).expect("the system's randomness");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Has each backend's daemon restart on the build installed since it
/// started, if one was, as `just install` leaves one running the old build
/// while agents work in it. A distribution whose `silt` isn't known yet is
/// left alone, rather than looked for as the app quits, as are machines
/// over SSH, which have their own installs.
fn restart_replaced(known: &[Backend], program: &Path, silts: &Silts) {
    for backend in known {
        let wsl_silt = match backend {
            Backend::Native | Backend::Ssh(_) => String::new(),
            Backend::Wsl(distribution) => match silts.paths.lock().expect("lock").get(distribution) {
                Some(found) => found.clone(),
                None => continue,
            },
        };
        if let Some(command) = backends::restart_replaced_daemon(backend, program, &wsl_silt) {
            daemon::spawn(command);
        }
    }
}

/// The file in the app's data dir that keeps the backend last shown.
const LAST_BACKEND: &str = "backend";

/// The file in the app's data dir that keeps each distribution's `silt`.
const WSL_SILTS: &str = "wsl-silt.json";

/// The backend last shown, if the app still knows it.
fn last_backend(file: &Path, known: &[Backend]) -> Option<Backend> {
    let text = fs::read_to_string(file).ok()?;
    Backend::parse(text.trim()).filter(|backend| known.contains(backend))
}

/// This computer, and on Windows each WSL distribution.
fn known_backends() -> Vec<Backend> {
    let mut known = vec![Backend::Native];
    if cfg!(windows) {
        let mut list = Command::new("wsl.exe");
        list.args(["--list", "--quiet"]);
        let found = daemon::output_of(list).map(|output| backends::distributions(&output)).unwrap_or_default();
        known.extend(found.into_iter().map(Backend::Wsl));
    }
    known
}

/// The target a `silt://` link among `args` names.
fn link_in(args: impl IntoIterator<Item = String>, known: &[Backend]) -> Option<Target> {
    args.into_iter().filter_map(|arg| Url::parse(&arg).ok()).find_map(|link| backends::target_of(&link, known))
}

/// What the page is told of the backends it can switch between, which it
/// offers where it says where Silt runs, rather than in a menu bar of the
/// window's own, and of this app.
fn desktop_script(state: &State) -> String {
    format!(
        "window.__siltBackends = {}; window.__siltDesktop = {};",
        state.list(),
        serde_json::json!({ "commit": env!("SILT_BUILD_COMMIT"), "key": state.key })
    )
}

/// Tells the page which backend it shows, once it has loaded.
fn current_script(backend: &Backend) -> String {
    format!(
        "window.__siltBackend = {id}; window.dispatchEvent(new CustomEvent('silt-backend', {{ detail: {id} }}));",
        id = serde_json::Value::String(backend.id())
    )
}

fn open_window(app: &AppHandle, state: Arc<State>, silts: Arc<Silts>, dir: Option<PathBuf>) -> tauri::Result<WebviewWindow> {
    let started = AtomicBool::new(false);
    let (navigating, loading, listening) = (state.clone(), state.clone(), state.clone());
    let relaunching = app.clone();
    let theme_file = dir.as_ref().map(|dir| dir.join(theme::LAST_THEME));
    let last_theme = theme_file.as_deref().and_then(theme::last);
    let themed = app.clone();
    let zoom_file = dir.as_ref().map(|dir| dir.join(zoom::LAST_ZOOM));
    let last_zoom = zoom_file.as_deref().and_then(zoom::last);
    let zoomed = app.clone();
    let mut builder = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
        .title("Silt")
        .inner_size(1440.0, 900.0)
        .min_inner_size(390.0, 600.0)
        .initialization_script(desktop_script(&state))
        .initialization_script(theme::WindowTheme::script(last_theme.as_ref()));
    if let Some(last) = last_theme {
        builder = builder.theme(Some(window_theme(&last))).background_color(window_color(&last));
    }
    let window = builder
        .on_navigation(move |to| {
            // The page's way to finish an update that changed this app (ADR 0100).
            if backends::is_relaunch(to) {
                relaunching.request_restart();
                return false;
            }
            // The page's own way to switch backends: the window goes there itself.
            if let Some(wanted) = backends::target_of(to, &navigating.known()) {
                *navigating.target.lock().expect("lock") = wanted;
                let state = navigating.clone();
                std::thread::spawn(move || state.tell());
                return false;
            }
            // And to edit the machines (ADR 0108), which can take a while.
            if let Some(edit) = machines::edit_of(to) {
                if navigating.may_edit_machines(to) {
                    let state = navigating.clone();
                    std::thread::spawn(move || edit_machines(&state, edit));
                }
                return false;
            }
            // And to say which theme it shows, which the window and its next start take (ADR 0099).
            if let Some(shown) = theme::of_link(to) {
                let (app, file) = (themed.clone(), theme_file.clone());
                std::thread::spawn(move || {
                    if let Some(window) = app.get_webview_window(MAIN) {
                        let _ = window.set_theme(Some(window_theme(&shown)));
                        let _ = window.set_background_color(Some(window_color(&shown)));
                    }
                    if let Some(file) = file {
                        theme::keep(&file, &shown);
                    }
                });
                return false;
            }
            // And how far to zoom, as text and layout set it (ADR 0107).
            if let Some(level) = zoom::of_link(to) {
                let (app, file) = (zoomed.clone(), zoom_file.clone());
                std::thread::spawn(move || {
                    if let Some(window) = app.get_webview_window(MAIN) {
                        let _ = window.set_zoom(level.factor());
                    }
                    if let Some(file) = file {
                        zoom::keep(&file, level);
                    }
                });
                return false;
            }
            let ui = navigating.ui.lock().expect("lock");
            daemon::stays_in_app(ui.as_ref(), to) || {
                open_in_browser(to);
                false
            }
        })
        .on_new_window(|to, _| {
            open_in_browser(&to);
            NewWindowResponse::Deny
        })
        .on_permission_request(move |webview, kind| match kind {
            PermissionKind::Microphone => {
                let ui = listening.ui.lock().expect("lock");
                if webview.url().is_ok_and(|page| daemon::may_listen(ui.as_ref(), &page)) {
                    PermissionResponse::Allow
                } else {
                    PermissionResponse::Deny
                }
            }
            PermissionKind::Camera => PermissionResponse::Deny,
            _ => PermissionResponse::Default,
        })
        // Once the starting page has loaded, so it can show what went wrong.
        .on_page_load(move |window, payload| {
            if payload.event() != PageLoadEvent::Finished {
                return;
            }
            let shown = loading.shown.lock().expect("lock").clone();
            let shown = shown.unwrap_or_else(|| loading.target.lock().expect("lock").backend.clone());
            let _ = window.eval(current_script(&shown));
            let state = loading.clone();
            std::thread::spawn(move || state.tell());
            if !started.swap(true, Ordering::SeqCst) {
                let (state, silts, dir) = (loading.clone(), silts.clone(), dir.clone());
                std::thread::spawn(move || keep_ui(&window, &state, &silts, dir.as_deref()));
            }
        })
        .build()?;
    #[cfg(target_os = "linux")]
    microphone::enable(&window)?;
    if let Some(level) = last_zoom {
        window.set_zoom(level.factor())?;
    }
    Ok(window)
}

/// Does what a link from the page asked of the machines. A machine the
/// window shows that's removed or disconnected gives way to this computer.
fn edit_machines(state: &State, edit: Edit) {
    let Some(backends) = state.backends.get() else {
        return;
    };
    let machines = &backends.machines;
    match edit {
        Edit::Add(machine) => {
            machines.add(machine);
        }
        Edit::Change { was, machine } => {
            let name = machine.name.clone();
            if machines.change(&was, machine) {
                state.retarget(&was, Backend::Ssh(name));
            }
        }
        Edit::Remove(name) => {
            state.retarget(&name, Backend::Native);
            machines.remove(&name);
        }
        Edit::Disconnect(name) => {
            state.retarget(&name, Backend::Native);
            machines.disconnect(&name);
        }
        Edit::Test { machine, add } => machines.test(&machine, add),
        Edit::Install { machine, add } => machines.install(&machine, add),
    }
    state.tell();
}

fn window_theme(shown: &theme::WindowTheme) -> Theme {
    if shown.dark { Theme::Dark } else { Theme::Light }
}

fn window_color(shown: &theme::WindowTheme) -> Color {
    let [r, g, b] = shown.bg;
    Color(r, g, b, 255)
}

/// How often to look at the UI the window shows, and how long a look waits.
const CHECK_EVERY: Duration = Duration::from_secs(2);
const CHECK_WAIT: Duration = Duration::from_secs(3);

/// Finds or starts the daemon and loads its UI, then keeps it loaded: when
/// the daemon stops or starts again on a new address, it's found or started
/// again and the window follows it to the same page. When another backend
/// or project is asked for, the window goes there, or stays and says why it
/// can't. Until the UI first loads, the starting page says why it hasn't.
fn keep_ui(window: &WebviewWindow, state: &State, silts: &Silts, dir: Option<&Path>) {
    let program = native_silt();
    let mut showing: Option<Target> = None;
    let mut lost = true;
    let mut failures = 0;
    loop {
        let wanted = state.target.lock().expect("lock").clone();
        let known = state.ui.lock().expect("lock").clone();
        if !lost
            && showing.as_ref() == Some(&wanted)
            && let Some(known) = &known
        {
            match daemon::check(known, CHECK_WAIT) {
                Answer::Up => wait_unless_retargeted(state, &wanted, CHECK_EVERY),
                answer => {
                    eprintln!("silt-desktop: the daemon's web UI is {answer:?}; finding it again");
                    lost = true;
                }
            }
            continue;
        }
        let Ok(shown) = window.url() else {
            return;
        };
        if known.is_none()
            && let Backend::Ssh(name) = &wanted.backend
        {
            let message = serde_json::Value::String(format!("Connecting to {name} over SSH…"));
            let _ = window.eval(format!("showStatus({message})"));
        }
        // Another backend or project starts on its own page.
        let carried = if showing.as_ref() == Some(&wanted) { known.as_ref() } else { None };
        let opened = open_ui(&wanted, &program, silts, state).and_then(|new| {
            let moved = daemon::same_page_on(Some(&shown), carried, &new);
            eprintln!("silt-desktop: showing {}{}", new.origin().ascii_serialization(), moved.path());
            *state.ui.lock().expect("lock") = Some(new);
            *state.shown.lock().expect("lock") = Some(wanted.backend.clone());
            let reload = daemon::only_the_fragment_moves(&shown, &moved);
            window
                .navigate(moved)
                .and_then(|()| if reload { window.reload() } else { Ok(()) })
                .map_err(|error| Exn::from(error).raise(DesktopError::Run("the window".into())))
        });
        // A `silt` moved or uninstalled since it was found is looked for
        // again next time.
        if opened.is_err()
            && let Backend::Wsl(distribution) = &wanted.backend
        {
            silts.forget(distribution);
        }
        match opened {
            Ok(()) => {
                if showing.as_ref().is_none_or(|shown| shown.backend != wanted.backend)
                    && let Some(dir) = dir
                {
                    remember_backend(dir, &wanted.backend);
                }
                if let (Backend::Ssh(name), Some(backends)) = (&wanted.backend, state.backends.get()) {
                    backends.machines.shown(name);
                }
                showing = Some(wanted);
                lost = false;
                failures = 0;
            }
            Err(error) => {
                eprintln!("silt-desktop: {}", explain(&error));
                if known.is_none() {
                    let message = serde_json::Value::String(explain(&error));
                    let elsewhere = wanted.backend != Backend::Native;
                    if window.eval(format!("showError({message}, {elsewhere})")).is_err() {
                        return;
                    }
                } else if let Some(was) = showing.as_ref().filter(|was| **was != wanted) {
                    // A switch that failed leaves the window where it was, and says why.
                    {
                        let mut target = state.target.lock().expect("lock");
                        if *target == wanted {
                            *target = was.clone();
                        }
                    }
                    let failed = serde_json::json!({
                        "id": wanted.backend.id(),
                        "label": wanted.backend.label(),
                        "reason": explain(&error),
                    });
                    let _ = window
                        .eval(format!("window.dispatchEvent(new CustomEvent('silt-backend-failed', {{ detail: {failed} }}));"));
                    state.tell();
                    continue;
                }
                wait_unless_retargeted(state, &wanted, daemon::backoff(failures));
                failures = failures.saturating_add(1);
            }
        }
    }
}

/// Waits `wait`, or less if another backend or project is asked for.
fn wait_unless_retargeted(state: &State, wanted: &Target, wait: Duration) {
    let until = Instant::now() + wait;
    while Instant::now() < until && *state.target.lock().expect("lock") == *wanted {
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The `silt` this computer's backend runs.
fn native_silt() -> PathBuf {
    let exe = env::current_exe().ok();
    daemon::silt_program(env::var_os("SILT_BIN"), exe.as_deref(), |path| path.is_file())
}

/// The target's UI, connecting to it first if it's a machine over SSH.
fn open_ui(target: &Target, program: &Path, silts: &Silts, state: &State) -> Result<Url, Exn<DesktopError>> {
    if let Backend::Ssh(name) = &target.backend {
        let backends = state.backends.get().expect("known before the window shows");
        return backends.machines.connect(name).map_err(|reason| Exn::new(DesktopError::Machine { name: name.clone(), reason }));
    }
    command_for(target, program, silts).and_then(daemon::open_web)
}

/// The command that asks the target's `silt` for its UI, for this computer
/// or a distribution.
fn command_for(target: &Target, program: &Path, silts: &Silts) -> Result<Command, Exn<DesktopError>> {
    let wsl_silt = match &target.backend {
        Backend::Native | Backend::Ssh(_) => String::new(),
        Backend::Wsl(distribution) => silts.find(distribution)?,
    };
    backends::open_web(target, program, &wsl_silt).ok_or_else(|| Exn::new(DesktopError::Run(target.backend.label())))
}

/// Starts the daemon of each backend but the one the window shows, which
/// the window starts itself, so every backend's watchers and sync run from
/// the app's launch rather than once it's picked. Each distribution starts
/// at once, as a distribution's first start can take seconds. A
/// distribution without `silt` is left alone, as are machines over SSH,
/// which are only reached when picked.
fn start_the_others(known: &[Backend], shown: &Backend, program: &Path, silts: &Silts) {
    std::thread::scope(|scope| {
        for backend in known.iter().filter(|backend| *backend != shown) {
            scope.spawn(move || {
                let wsl_silt = match backend {
                    Backend::Native | Backend::Ssh(_) => String::new(),
                    Backend::Wsl(distribution) => match silts.find(distribution) {
                        Ok(found) => found,
                        Err(error) => return eprintln!("silt-desktop: not starting {}: {}", backend.label(), explain(&error)),
                    },
                };
                let Some(command) = backends::start_daemon(backend, program, &wsl_silt) else {
                    return;
                };
                match daemon::run(command) {
                    Ok(()) => eprintln!("silt-desktop: started {}", backend.label()),
                    Err(error) => {
                        if let Backend::Wsl(distribution) = backend {
                            silts.forget(distribution);
                        }
                        eprintln!("silt-desktop: could not start {}: {}", backend.label(), explain(&error));
                    }
                }
            });
        }
    });
}

/// Each distribution's `silt` as last found, shared by the window and the
/// starts at launch, and remembered across launches. A `silt` moved or
/// uninstalled since it was found is forgotten when it fails, and looked
/// for again next time.
struct Silts {
    file: Option<PathBuf>,
    paths: Mutex<HashMap<String, String>>,
}

impl Silts {
    fn load(file: Option<PathBuf>) -> Self {
        let paths = file
            .as_deref()
            .and_then(|file| fs::read_to_string(file).ok())
            .map(|text| backends::remembered_silts(&text))
            .unwrap_or_default();
        Self { file, paths: Mutex::new(paths) }
    }

    /// The distribution's `silt`, looked for once through a login shell.
    fn find(&self, distribution: &str) -> Result<String, Exn<DesktopError>> {
        if let Some(found) = self.paths.lock().expect("lock").get(distribution) {
            return Ok(found.clone());
        }
        let found = daemon::output_of(backends::find_silt(distribution))
            .and_then(|stdout| backends::found_silt(&String::from_utf8_lossy(&stdout)))
            .ok_or_else(|| Exn::new(DesktopError::NoSiltIn(distribution.to_owned())))?;
        let mut paths = self.paths.lock().expect("lock");
        paths.insert(distribution.to_owned(), found.clone());
        self.save(&paths);
        Ok(found)
    }

    fn forget(&self, distribution: &str) {
        let mut paths = self.paths.lock().expect("lock");
        if paths.remove(distribution).is_some() {
            self.save(&paths);
        }
    }

    fn save(&self, paths: &HashMap<String, String>) {
        let Some(file) = &self.file else {
            return;
        };
        let written =
            file.parent().map_or(Ok(()), fs::create_dir_all).and_then(|()| fs::write(file, backends::remember_silts(paths)));
        if let Err(error) = written {
            eprintln!("silt-desktop: could not remember where silt is in each distribution: {error}");
        }
    }
}

fn remember_backend(dir: &Path, backend: &Backend) {
    if let Err(error) = fs::create_dir_all(dir).and_then(|()| fs::write(dir.join(LAST_BACKEND), backend.id())) {
        eprintln!("silt-desktop: could not remember the backend shown: {error}");
    }
}

fn show(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Links out of the app open in the person's browser, as they would from a
/// browser tab.
fn open_in_browser(url: &Url) {
    if !matches!(url.scheme(), "http" | "https") {
        return;
    }
    let url = url.to_string();
    std::thread::spawn(move || {
        // rundll32 hands the address to the default browser without cmd,
        // which would split it at its `&`.
        let mut opener = if cfg!(windows) {
            let mut opener = Command::new("rundll32");
            opener.arg("url.dll,FileProtocolHandler");
            opener
        } else {
            Command::new("xdg-open")
        };
        let _ = opener.arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    });
}

/// The error and what caused it, outermost first, as one line.
fn explain(error: &Exn<DesktopError>) -> String {
    let mut causes = Vec::new();
    let mut frame = error.frame();
    loop {
        causes.push(frame.error().to_string());
        match frame.children().first() {
            Some(child) => frame = child,
            None => break,
        }
    }
    let mut source = frame.error().source();
    while let Some(cause) = source {
        causes.push(cause.to_string());
        source = cause.source();
    }
    causes.join(": ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Starts this computer's backend and a WSL distribution's through the
    /// commands the app runs, when `SILT_TEST_WSL` names a distribution with
    /// `silt`.
    #[test]
    fn the_native_and_a_distribution_backend_start() {
        let Some(distribution) = env::var("SILT_TEST_WSL").ok().filter(|name| !name.is_empty()) else {
            return;
        };
        let program = daemon::silt_program(env::var_os("SILT_BIN"), None, |_| false);
        let silts = Silts::load(None);
        for backend in [Backend::Native, Backend::Wsl(distribution.clone())] {
            let wsl_silt = match &backend {
                Backend::Native | Backend::Ssh(_) => String::new(),
                Backend::Wsl(distribution) => silts.find(distribution).unwrap_or_else(|error| panic!("{}", explain(&error))),
            };
            daemon::run(backends::start_daemon(&backend, &program, &wsl_silt).expect("a command"))
                .unwrap_or_else(|error| panic!("{backend:?} starts: {}", explain(&error)));
            let target = Target { backend, project: None };
            let ui = command_for(&target, &program, &silts)
                .and_then(daemon::open_web)
                .unwrap_or_else(|error| panic!("{target:?}: {}", explain(&error)));
            assert_eq!(daemon::check(&ui, CHECK_WAIT), Answer::Up, "{target:?} answers at {}", ui.origin().ascii_serialization());
        }
        let found = silts.find(&distribution).expect("found before");
        assert!(found.starts_with('/'), "the distribution's silt was found: {found}");
    }

    /// At launch, every backend but the one shown is started; a
    /// distribution whose `silt` can't be found is left alone.
    #[cfg(unix)]
    #[test]
    fn the_backends_not_shown_start_at_launch() {
        use std::os::unix::fs::PermissionsExt;

        let dir = env::temp_dir().join(format!("silt-desktop-start-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("a temporary dir");
        let (program, ran) = (dir.join("silt"), dir.join("ran"));
        fs::write(&program, format!("#!/bin/sh\necho \"$@\" >> '{}'\n", ran.display())).expect("a stand-in silt");
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).expect("executable");
        let silts = Silts::load(Some(dir.join(WSL_SILTS)));
        let known = [Backend::Native, Backend::Wsl("no-such-distribution".into())];

        start_the_others(&known, &known[1], &program, &silts);
        assert_eq!(fs::read_to_string(&ran).expect("silt ran"), "daemon start\n");

        fs::remove_file(&ran).expect("removed");
        start_the_others(&known, &Backend::Native, &program, &silts);
        assert!(!ran.exists(), "only the shown backend was this computer's");
        assert!(silts.paths.lock().expect("lock").is_empty());
        fs::remove_dir_all(&dir).expect("cleaned up");
    }

    /// Quitting has this computer's daemon restart if an install replaced
    /// it; a distribution whose `silt` isn't known yet is left alone.
    #[cfg(unix)]
    #[test]
    fn quitting_restarts_a_replaced_daemon() {
        use std::os::unix::fs::PermissionsExt;

        let dir = env::temp_dir().join(format!("silt-desktop-quit-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("a temporary dir");
        let (program, ran) = (dir.join("silt"), dir.join("ran"));
        fs::write(&program, format!("#!/bin/sh\necho \"$@\" >> '{}'\n", ran.display())).expect("a stand-in silt");
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).expect("executable");
        let silts = Silts::load(None);

        restart_replaced(&[Backend::Native, Backend::Wsl("no-such-distribution".into())], &program, &silts);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !ran.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(fs::read_to_string(&ran).expect("silt ran"), "daemon restart --if-replaced\n");
        fs::remove_dir_all(&dir).expect("cleaned up");
    }
}
