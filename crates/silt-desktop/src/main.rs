//! Silt's desktop app: the local daemon's web UI in a window (ADR 0041).
//! Recording never depends on it. The daemon runs on its own, and closing
//! the window leaves it and its agents running. On Windows the window also
//! shows WSL distributions' daemons (ADR 0045).

// A console program would open a console window beside the app's own on
// Windows; debug builds keep it for their logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backends;
mod daemon;
#[cfg(target_os = "linux")]
mod microphone;

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use exn::Exn;
use tauri::webview::{NewWindowResponse, PageLoadEvent, PermissionKind, PermissionResponse};
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::backends::{Backend, Target};
use crate::daemon::{Answer, DesktopError};

const MAIN: &str = "main";

fn main() {
    // Shared by the window's handlers: where the daemon's UI is, once known,
    // and which backend and project the window should show.
    let ui: Arc<Mutex<Option<Url>>> = Arc::default();
    let known = Arc::new(known_backends());
    let asked = link_in(env::args(), &known);
    let opened_by_link = asked.is_some();
    let target = Arc::new(Mutex::new(asked.unwrap_or_default()));
    let (linked, linked_known) = (target.clone(), known.clone());
    tauri::Builder::default()
        // A second launch, a `silt://` link among them, shows the window
        // that's already open.
        .plugin(tauri_plugin_single_instance::init(move |app, argv, _| {
            if let Some(wanted) = link_in(argv, &linked_known) {
                *linked.lock().expect("lock") = wanted;
            }
            show(app);
        }))
        .plugin(tauri_plugin_deep_link::init())
        .setup(move |app| {
            // Opened by itself, the window shows where it last did, not this computer every time.
            if !opened_by_link
                && let Some(backend) =
                    app.path().app_data_dir().ok().and_then(|dir| last_backend(&dir.join(LAST_BACKEND), &known))
            {
                *target.lock().expect("lock") = Target { backend, project: None };
            }
            let dir = app.path().app_data_dir().ok();
            let silts = Arc::new(Silts::load(dir.as_ref().map(|dir| dir.join(WSL_SILTS))));
            let shown = target.lock().expect("lock").backend.clone();
            let (others, starting) = (known.clone(), silts.clone());
            std::thread::spawn(move || start_the_others(&others, &shown, &native_silt(), &starting));
            open_window(app.handle(), ui, target, known, silts, dir)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("the desktop app could not start");
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
/// window's own; one backend has nothing to choose.
fn backends_script(known: &[Backend]) -> String {
    let backends: Vec<serde_json::Value> = if known.len() > 1 {
        known.iter().map(|backend| serde_json::json!({ "id": backend.id(), "label": backend.label() })).collect()
    } else {
        Vec::new()
    };
    format!(
        "window.__siltBackends = {}; window.__siltDesktop = {};",
        serde_json::Value::Array(backends),
        serde_json::json!({ "commit": env!("SILT_BUILD_COMMIT") })
    )
}

/// Tells the page which backend it shows, once it has loaded.
fn current_script(target: &Target) -> String {
    format!(
        "window.__siltBackend = {id}; window.dispatchEvent(new CustomEvent('silt-backend', {{ detail: {id} }}));",
        id = serde_json::Value::String(target.backend.id())
    )
}

fn open_window(
    app: &AppHandle,
    ui: Arc<Mutex<Option<Url>>>,
    target: Arc<Mutex<Target>>,
    known: Arc<Vec<Backend>>,
    silts: Arc<Silts>,
    dir: Option<PathBuf>,
) -> tauri::Result<WebviewWindow> {
    let started = AtomicBool::new(false);
    let navigating = ui.clone();
    let loading = ui.clone();
    let listening = ui.clone();
    let (switching, shown) = (target.clone(), target.clone());
    let relaunching = app.clone();
    let window = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
        .title("Silt")
        .inner_size(1440.0, 900.0)
        .min_inner_size(390.0, 600.0)
        .initialization_script(backends_script(&known))
        .on_navigation(move |to| {
            // The page's way to finish an update that changed this app (ADR 0098).
            if backends::is_relaunch(to) {
                relaunching.request_restart();
                return false;
            }
            // The page's own way to switch backends: the window goes there itself.
            if let Some(wanted) = backends::target_of(to, &known) {
                *switching.lock().expect("lock") = wanted;
                return false;
            }
            let ui = navigating.lock().expect("lock");
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
                let ui = listening.lock().expect("lock");
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
            if payload.event() == PageLoadEvent::Finished {
                let _ = window.eval(current_script(&shown.lock().expect("lock")));
            }
            if payload.event() == PageLoadEvent::Finished && !started.swap(true, Ordering::SeqCst) {
                let (ui, target, silts, dir) = (loading.clone(), target.clone(), silts.clone(), dir.clone());
                std::thread::spawn(move || keep_ui(&window, &ui, &target, &silts, dir.as_deref()));
            }
        })
        .build()?;
    #[cfg(target_os = "linux")]
    microphone::enable(&window)?;
    Ok(window)
}

/// How often to look at the UI the window shows, and how long a look waits.
const CHECK_EVERY: Duration = Duration::from_secs(2);
const CHECK_WAIT: Duration = Duration::from_secs(3);

/// Finds or starts the daemon and loads its UI, then keeps it loaded: when
/// the daemon stops or starts again on a new address, it's found or started
/// again and the window follows it to the same page. When another backend
/// or project is asked for, the window goes there. Until the UI first
/// loads, the starting page says why it hasn't.
fn keep_ui(window: &WebviewWindow, ui: &Mutex<Option<Url>>, target: &Mutex<Target>, silts: &Silts, dir: Option<&Path>) {
    let program = native_silt();
    let mut showing: Option<Target> = None;
    let mut lost = true;
    let mut failures = 0;
    loop {
        let wanted = target.lock().expect("lock").clone();
        let known = ui.lock().expect("lock").clone();
        if !lost
            && showing.as_ref() == Some(&wanted)
            && let Some(known) = &known
        {
            match daemon::check(known, CHECK_WAIT) {
                Answer::Up => std::thread::sleep(CHECK_EVERY),
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
        // Another backend or project starts on its own page.
        let carried = if showing.as_ref() == Some(&wanted) { known.as_ref() } else { None };
        let opened = command_for(&wanted, &program, silts).and_then(daemon::open_web).and_then(|new| {
            let moved = daemon::same_page_on(Some(&shown), carried, &new);
            eprintln!("silt-desktop: showing {}{}", new.origin().ascii_serialization(), moved.path());
            *ui.lock().expect("lock") = Some(new);
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
                showing = Some(wanted);
                lost = false;
                failures = 0;
            }
            Err(error) => {
                if known.is_none() || showing.as_ref() != Some(&wanted) {
                    let message = serde_json::to_string(&explain(&error)).expect("a string");
                    if window.eval(format!("showError({message})")).is_err() {
                        return;
                    }
                }
                eprintln!("silt-desktop: {}", explain(&error));
                std::thread::sleep(daemon::backoff(failures));
                failures = failures.saturating_add(1);
            }
        }
    }
}

/// The `silt` this computer's backend runs.
fn native_silt() -> PathBuf {
    let exe = env::current_exe().ok();
    daemon::silt_program(env::var_os("SILT_BIN"), exe.as_deref(), |path| path.is_file())
}

/// The command that asks the target's `silt` for its UI.
fn command_for(target: &Target, program: &Path, silts: &Silts) -> Result<Command, Exn<DesktopError>> {
    let wsl_silt = match &target.backend {
        Backend::Native => String::new(),
        Backend::Wsl(distribution) => silts.find(distribution)?,
    };
    Ok(backends::open_web(target, program, &wsl_silt))
}

/// Starts the daemon of each backend but the one the window shows, which
/// the window starts itself, so every backend's watchers and sync run from
/// the app's launch rather than once it's picked. Each distribution starts
/// at once, as a distribution's first start can take seconds. A
/// distribution without `silt` is left alone.
fn start_the_others(known: &[Backend], shown: &Backend, program: &Path, silts: &Silts) {
    std::thread::scope(|scope| {
        for backend in known.iter().filter(|backend| *backend != shown) {
            scope.spawn(move || {
                let wsl_silt = match backend {
                    Backend::Native => String::new(),
                    Backend::Wsl(distribution) => match silts.find(distribution) {
                        Ok(found) => found,
                        Err(error) => return eprintln!("silt-desktop: not starting {}: {}", backend.label(), explain(&error)),
                    },
                };
                match daemon::run(backends::start_daemon(backend, program, &wsl_silt)) {
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
    /// `silt`, as the Windows CI job's does.
    #[test]
    fn the_native_and_a_distribution_backend_start() {
        let Some(distribution) = env::var("SILT_TEST_WSL").ok().filter(|name| !name.is_empty()) else {
            return;
        };
        let program = daemon::silt_program(env::var_os("SILT_BIN"), None, |_| false);
        let silts = Silts::load(None);
        for backend in [Backend::Native, Backend::Wsl(distribution.clone())] {
            let wsl_silt = match &backend {
                Backend::Native => String::new(),
                Backend::Wsl(distribution) => silts.find(distribution).unwrap_or_else(|error| panic!("{}", explain(&error))),
            };
            daemon::run(backends::start_daemon(&backend, &program, &wsl_silt))
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
}
