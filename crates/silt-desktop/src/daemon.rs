//! Finding the local daemon's web UI through the `silt` CLI, which attaches
//! to the daemon or starts it (ADR 0041).

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use exn::{Exn, ResultExt};
use tauri::Url;

#[derive(Debug, thiserror::Error)]
pub enum DesktopError {
    #[error("Silt's command line isn't installed; install `silt`, or set SILT_BIN to it")]
    NoSilt,
    #[error("could not run {0}")]
    Run(String),
    #[error("{0}")]
    Silt(String),
    #[error("`silt open` answered with something other than the web UI's address: {0:?}")]
    NotTheUi(String),
}

/// The `silt` to run: `$SILT_BIN`, else one installed beside this app, else
/// whichever the `PATH` finds.
pub fn silt_program(silt_bin: Option<OsString>, app_exe: Option<&Path>, is_file: impl Fn(&Path) -> bool) -> PathBuf {
    if let Some(bin) = silt_bin.filter(|bin| !bin.is_empty()) {
        return PathBuf::from(bin);
    }
    app_exe
        .and_then(Path::parent)
        .map(|dir| dir.join("silt"))
        .filter(|beside| is_file(beside))
        .unwrap_or_else(|| PathBuf::from("silt"))
}

/// The web UI's address from what `silt open --no-browser` printed. It
/// carries the daemon's lasting token, which is fine here: it reached this
/// app through a pipe, not a command line others can read.
pub fn web_url(stdout: &str) -> Result<Url, DesktopError> {
    let line = stdout.lines().map(str::trim).rfind(|line| !line.is_empty()).unwrap_or_default();
    let not_the_ui = || DesktopError::NotTheUi(line.to_owned());
    let url = Url::parse(line).map_err(|_| not_the_ui())?;
    let local = url.scheme() == "http" && url.host_str() == Some("127.0.0.1") && url.port().is_some();
    let token = url.fragment().is_some_and(|fragment| fragment.starts_with("token="));
    if local && token { Ok(url) } else { Err(not_the_ui()) }
}

/// Attaches to the daemon, starting it if none runs, and returns its web
/// UI's address. The daemon runs in its own process group, so it outlives
/// this app.
pub fn open_web(program: &Path) -> Result<Url, Exn<DesktopError>> {
    let shown = program.display().to_string();
    let output = Command::new(program)
        .args(["open", "--no-browser"])
        // Not a clone of any repository, whatever directory the app
        // started in.
        .current_dir("/")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => Exn::new(DesktopError::NoSilt),
            _ => Exn::from(error).raise(DesktopError::Run(shown.clone())),
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr.lines().map(str::trim).rfind(|line| !line.is_empty()).unwrap_or("it failed");
        return Err(Exn::new(DesktopError::Silt(reason.strip_prefix("silt: ").unwrap_or(reason).to_owned())));
    }
    web_url(&String::from_utf8_lossy(&output.stdout)).or_raise(|| DesktopError::Run(shown))
}

/// Whether the window may go to `to`: the daemon's UI, or this app's own
/// starting page. Anything else, such as a link in a transcript, belongs in
/// the browser.
pub fn stays_in_app(ui: Option<&Url>, to: &Url) -> bool {
    let own = to.scheme() == "tauri" || (to.scheme() == "http" && to.host_str() == Some("tauri.localhost"));
    own || ui.is_some_and(|ui| ui.origin() == to.origin())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silt_bin_wins_then_the_one_beside_the_app_then_the_path() {
        let app = Path::new("/opt/silt/silt-desktop");
        let everywhere = |_: &Path| true;
        let nowhere = |_: &Path| false;
        assert_eq!(silt_program(Some("/x/silt".into()), Some(app), everywhere), PathBuf::from("/x/silt"));
        assert_eq!(silt_program(Some("".into()), Some(app), everywhere), PathBuf::from("/opt/silt/silt"));
        assert_eq!(silt_program(None, Some(app), nowhere), PathBuf::from("silt"));
        assert_eq!(silt_program(None, None, everywhere), PathBuf::from("silt"));
    }

    #[test]
    fn only_the_local_ui_with_its_token_is_taken() {
        let url = web_url("http://127.0.0.1:4321/#token=abc\n").expect("the UI");
        assert_eq!(url.port(), Some(4321));
        assert!(web_url("note\nhttp://127.0.0.1:4321/#token=abc\n\n").is_ok());
        for wrong in ["", "http://example.com:4321/#token=abc", "http://127.0.0.1:4321/#code=abc", "https://127.0.0.1:1/#token=a"]
        {
            assert!(matches!(web_url(wrong), Err(DesktopError::NotTheUi(_))), "{wrong}");
        }
    }

    #[test]
    fn the_window_stays_on_the_ui_and_sends_the_rest_out() {
        let ui = Url::parse("http://127.0.0.1:4321/#token=abc").expect("url");
        let inside = |to: &str| stays_in_app(Some(&ui), &Url::parse(to).expect("url"));
        assert!(inside("http://127.0.0.1:4321/t/abc#turn-2"));
        assert!(inside("http://tauri.localhost/index.html"));
        assert!(inside("tauri://localhost/index.html"));
        assert!(!inside("http://127.0.0.1:9999/"));
        assert!(!inside("http://localhost:4321/"));
        assert!(!inside("https://github.com/"));
        assert!(!stays_in_app(None, &Url::parse("http://127.0.0.1:4321/").expect("url")));
    }

    #[test]
    fn a_missing_silt_says_how_to_get_one() {
        let error = open_web(Path::new("/nonexistent/silt")).expect_err("no silt there");
        assert!(matches!(error.frame().error().downcast_ref::<DesktopError>(), Some(DesktopError::NoSilt)));
    }
}
