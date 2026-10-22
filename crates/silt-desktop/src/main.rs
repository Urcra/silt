//! Silt's desktop app: the local daemon's web UI in a window (ADR 0040).
//! Recording never depends on it. The daemon runs on its own, and closing
//! the window leaves it and its agents running.

mod daemon;

use std::env;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use exn::Exn;
use tauri::webview::{NewWindowResponse, PageLoadEvent};
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::daemon::DesktopError;

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
                tauri::async_runtime::spawn_blocking(move || show_ui(&window, &ui));
            }
        })
        .build()
}

/// Finds or starts the daemon and loads its UI, or says why it can't.
fn show_ui(window: &WebviewWindow, ui: &Mutex<Option<Url>>) {
    let exe = env::current_exe().ok();
    let program = daemon::silt_program(env::var_os("SILT_BIN"), exe.as_deref(), |path| path.is_file());
    let shown = daemon::open_web(&program).and_then(|url| {
        *ui.lock().expect("lock") = Some(url.clone());
        window.navigate(url).map_err(|error| Exn::from(error).raise(DesktopError::Run("the window".into())))
    });
    if let Err(error) = shown {
        let message = serde_json::to_string(&explain(&error)).expect("a string");
        if let Err(error) = window.eval(format!("showError({message})")) {
            eprintln!("silt-desktop: {error}");
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
