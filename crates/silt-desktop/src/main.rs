//! Silt's desktop app: the local daemon's web UI in a window (ADR 0041).
//! Recording never depends on it. The daemon runs on its own, and closing
//! the window leaves it and its agents running.

mod daemon;

use std::env;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use exn::Exn;
use tauri::webview::{NewWindowResponse, PageLoadEvent};
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::daemon::{Answer, DesktopError};

const MAIN: &str = "main";

fn main() {
    // Shared by the window's handlers: where the daemon's UI is, once known.
    let ui: Arc<Mutex<Option<Url>>> = Arc::default();
    tauri::Builder::default()
        // A second launch shows the window that's already open.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| show(app)))
        .setup(move |app| {
            open_window(app.handle(), ui)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("the desktop app could not start");
}

fn open_window(app: &AppHandle, ui: Arc<Mutex<Option<Url>>>) -> tauri::Result<WebviewWindow> {
    let started = AtomicBool::new(false);
    let navigating = ui.clone();
    let loading = ui.clone();
    WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
        .title("Silt")
        .inner_size(1440.0, 900.0)
        .min_inner_size(390.0, 600.0)
        .on_navigation(move |to| {
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
        // Once the starting page has loaded, so it can show what went wrong.
        .on_page_load(move |window, payload| {
            if payload.event() == PageLoadEvent::Finished && !started.swap(true, Ordering::SeqCst) {
                let ui = loading.clone();
                std::thread::spawn(move || keep_ui(&window, &ui));
            }
        })
        .build()
}

/// How often to look at the UI the window shows, and how long a look waits.
const CHECK_EVERY: Duration = Duration::from_secs(2);
const CHECK_WAIT: Duration = Duration::from_secs(3);

/// Finds or starts the daemon and loads its UI, then keeps it loaded: when
/// the daemon stops or starts again on a new address, it's found or started
/// again and the window follows it to the same page. Until the UI first
/// loads, the starting page says why it hasn't.
fn keep_ui(window: &WebviewWindow, ui: &Mutex<Option<Url>>) {
    let exe = env::current_exe().ok();
    let program = daemon::silt_program(env::var_os("SILT_BIN"), exe.as_deref(), |path| path.is_file());
    let mut lost = true;
    let mut failures = 0;
    loop {
        let known = ui.lock().expect("lock").clone();
        if !lost && let Some(known) = &known {
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
        let opened = daemon::open_web(&program).and_then(|new| {
            let moved = daemon::same_page_on(Some(&shown), known.as_ref(), &new);
            eprintln!("silt-desktop: showing {}{}", new.origin().ascii_serialization(), moved.path());
            *ui.lock().expect("lock") = Some(new);
            let reload = daemon::only_the_fragment_moves(&shown, &moved);
            window
                .navigate(moved)
                .and_then(|()| if reload { window.reload() } else { Ok(()) })
                .map_err(|error| Exn::from(error).raise(DesktopError::Run("the window".into())))
        });
        match opened {
            Ok(()) => {
                lost = false;
                failures = 0;
            }
            Err(error) => {
                if known.is_none() {
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
        let _ = Command::new("xdg-open").arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
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
