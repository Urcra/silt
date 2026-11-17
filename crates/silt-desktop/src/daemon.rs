//! Finding the local daemon's web UI through the `silt` CLI, which attaches
//! to the daemon or starts it (ADR 0041).

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use exn::{Exn, ResultExt};
use tauri::Url;

#[derive(Debug, thiserror::Error)]
pub enum DesktopError {
    #[error("Silt's command line isn't installed; install `silt`, or set SILT_BIN to it")]
    NoSilt,
    #[error("Silt isn't installed in the WSL distribution {0}; install it there, then pick it again")]
    NoSiltIn(String),
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
        .map(|dir| dir.join(if cfg!(windows) { "silt.exe" } else { "silt" }))
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

/// Runs `command`, which attaches to a daemon, starting it if none runs,
/// and returns its web UI's address. The daemon runs in its own process
/// group, so it outlives this app.
pub fn open_web(command: Command) -> Result<Url, Exn<DesktopError>> {
    let shown = command.get_program().to_string_lossy().into_owned();
    let stdout = silt_output(command)?;
    web_url(&stdout).or_raise(|| DesktopError::Run(shown))
}

/// Runs `command`, a `silt` command such as `daemon start`, for its effect.
pub fn run(command: Command) -> Result<(), Exn<DesktopError>> {
    silt_output(command).map(drop)
}

/// What a `silt` command printed, or the reason it gave for failing.
fn silt_output(mut command: Command) -> Result<String, Exn<DesktopError>> {
    let shown = command.get_program().to_string_lossy().into_owned();
    hide_console(&mut command);
    let output = command.stdin(Stdio::null()).output().map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => Exn::new(DesktopError::NoSilt),
        _ => Exn::from(error).raise(DesktopError::Run(shown)),
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr.lines().map(str::trim).rfind(|line| !line.is_empty()).unwrap_or("it failed");
        return Err(Exn::new(DesktopError::Silt(reason.strip_prefix("silt: ").unwrap_or(reason).to_owned())));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// A window app's console programs would each open a console window.
fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::os::windows::process::CommandExt::creation_flags(command, CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// Runs `command` quietly and returns what it printed, if it succeeded.
pub fn output_of(mut command: Command) -> Option<Vec<u8>> {
    hide_console(&mut command);
    let output = command.stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    output.status.success().then_some(output.stdout)
}

/// What a look at the UI found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// It answers, or is too busy to: either way, it runs.
    Up,
    /// Nothing listens there any more.
    Gone,
    /// It refuses the window's token: a daemon started again on a fixed
    /// `SILT_WEB_PORT`, with a new token.
    Stale,
}

/// A request that only the web layer answers when the token is wrong, so a
/// refused token shows at once however busy the daemon is.
pub fn status_request(ui: &Url) -> Option<String> {
    let token = ui.fragment()?.strip_prefix("token=")?;
    let port = ui.port()?;
    Some(format!(
        "GET /api/status HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
    ))
}

/// The answer from the start of an HTTP response.
pub fn answer_of(response: &[u8]) -> Answer {
    let line = response.split(|byte| *byte == b'\n').next().unwrap_or_default();
    match line.split(|byte| *byte == b' ').nth(1) {
        Some(b"401") => Answer::Stale,
        Some(_) => Answer::Up,
        None => Answer::Gone,
    }
}

/// Looks at the UI at `ui`. A daemon that takes longer than `wait` to answer
/// is busy, not gone.
pub fn check(ui: &Url, wait: Duration) -> Answer {
    let (Some(request), Some(port)) = (status_request(ui), ui.port()) else {
        return Answer::Gone;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), wait) else {
        return Answer::Gone;
    };
    if stream.set_read_timeout(Some(wait)).is_err() || stream.write_all(request.as_bytes()).is_err() {
        return Answer::Gone;
    }
    let mut start = [0; 16];
    let mut read = 0;
    while read < start.len() {
        match stream.read(&mut start[read..]) {
            Ok(0) => break,
            Ok(more) => read += more,
            // A signal, such as a child's exit, isn't the daemon's answer.
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => return Answer::Up,
            Err(_) => break,
        }
    }
    answer_of(&start[..read])
}

/// How long to wait before running `silt open` again after `failures`
/// failed tries in a row.
pub fn backoff(failures: u32) -> Duration {
    const FIRST: Duration = Duration::from_millis(500);
    const MOST: Duration = Duration::from_secs(10);
    FIRST.saturating_mul(2u32.saturating_pow(failures)).min(MOST)
}

/// Where the window goes on a new address for the UI: the page it was on,
/// if it was on the UI.
pub fn same_page_on(shown: Option<&Url>, old: Option<&Url>, new: &Url) -> Url {
    let mut moved = new.clone();
    if let (Some(shown), Some(old)) = (shown, old)
        && shown.origin() == old.origin()
    {
        moved.set_path(shown.path());
        moved.set_query(shown.query());
    }
    moved
}

/// Whether going from `shown` to `moved` only changes the fragment, which a
/// browser takes as a move within the page: the page must then be reloaded
/// to read its new token.
pub fn only_the_fragment_moves(shown: &Url, moved: &Url) -> bool {
    shown[..url::Position::AfterQuery] == moved[..url::Position::AfterQuery]
}

/// Whether the window may go to `to`: the daemon's UI, this app's own
/// starting page, or a frame the UI writes itself. Anything else, such as a
/// link in a transcript, belongs in the browser.
pub fn stays_in_app(ui: Option<&Url>, to: &Url) -> bool {
    let own = to.scheme() == "tauri" || (to.scheme() == "http" && to.host_str() == Some("tauri.localhost"));
    // WebKitGTK asks about every frame's navigation, not only the window's,
    // and a canvas's boards, a deck's slides and a design system's preview
    // are `srcdoc` frames. A window can't be sent to `about:srcdoc` itself.
    let written = to.as_str() == "about:srcdoc";
    own || written || ui.is_some_and(|ui| ui.origin() == to.origin())
}

/// Whether the page at `page` may hear the microphone: only the daemon's own
/// UI, which the window can stray from only by a navigation it refuses.
pub fn may_listen(ui: Option<&Url>, page: &Url) -> bool {
    ui.is_some_and(|ui| ui.origin() == page.origin())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_daemons_own_ui_may_listen() {
        let ui = Url::parse("http://127.0.0.1:4321/#token=abc").expect("a URL");
        let page = |url: &str| Url::parse(url).expect("a URL");
        assert!(may_listen(Some(&ui), &page("http://127.0.0.1:4321/t/1")));
        assert!(!may_listen(None, &page("http://127.0.0.1:4321/")));
        for other in ["http://127.0.0.1:9/", "http://example.com:4321/", "tauri://localhost/index.html"] {
            assert!(!may_listen(Some(&ui), &page(other)), "{other}");
        }
    }

    #[test]
    fn silt_bin_wins_then_the_one_beside_the_app_then_the_path() {
        let app = Path::new("/opt/silt/silt-desktop");
        let everywhere = |_: &Path| true;
        let nowhere = |_: &Path| false;
        assert_eq!(silt_program(Some("/x/silt".into()), Some(app), everywhere), PathBuf::from("/x/silt"));
        let beside = if cfg!(windows) { "/opt/silt/silt.exe" } else { "/opt/silt/silt" };
        assert_eq!(silt_program(Some("".into()), Some(app), everywhere), PathBuf::from(beside));
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
        assert!(inside("about:srcdoc"));
        assert!(!inside("about:blank"));
        assert!(!inside("http://127.0.0.1:9999/"));
        assert!(!inside("http://localhost:4321/"));
        assert!(!inside("https://github.com/"));
        assert!(!stays_in_app(None, &Url::parse("http://127.0.0.1:4321/").expect("url")));
    }

    #[test]
    fn the_status_request_carries_the_token_to_the_ui_s_own_host() {
        let ui = Url::parse("http://127.0.0.1:4321/#token=abc").expect("url");
        let request = status_request(&ui).expect("a request");
        assert!(request.starts_with("GET /api/status HTTP/1.1\r\n"));
        assert!(request.contains("\r\nHost: 127.0.0.1:4321\r\n"));
        assert!(request.contains("\r\nAuthorization: Bearer abc\r\n"));
        assert!(request.ends_with("\r\n\r\n"));
        assert_eq!(status_request(&Url::parse("http://127.0.0.1:4321/").expect("url")), None);
    }

    #[test]
    fn a_refused_token_is_stale_and_no_answer_is_gone() {
        assert_eq!(answer_of(b"HTTP/1.1 200 OK\r\n"), Answer::Up);
        assert_eq!(answer_of(b"HTTP/1.1 502 Bad"), Answer::Up);
        assert_eq!(answer_of(b"HTTP/1.1 401 Unauthorized\r\n"), Answer::Stale);
        assert_eq!(answer_of(b""), Answer::Gone);
    }

    #[test]
    fn retries_back_off_to_ten_seconds() {
        let delays: Vec<_> = (0..7).map(|failures| backoff(failures).as_millis()).collect();
        assert_eq!(delays, [500, 1000, 2000, 4000, 8000, 10_000, 10_000]);
        assert_eq!(backoff(u32::MAX), Duration::from_secs(10));
    }

    #[test]
    fn a_new_address_keeps_the_page_the_window_was_on() {
        let url = |text: &str| Url::parse(text).expect("url");
        let old = url("http://127.0.0.1:4321/#token=old");
        let new = url("http://127.0.0.1:5555/#token=new");
        let shown = url("http://127.0.0.1:4321/t/abc?tab=changes");
        assert_eq!(same_page_on(Some(&shown), Some(&old), &new).as_str(), "http://127.0.0.1:5555/t/abc?tab=changes#token=new");
        let starting = url("tauri://localhost/index.html");
        assert_eq!(same_page_on(Some(&starting), Some(&old), &new), new);
        assert_eq!(same_page_on(Some(&shown), None, &new), new);
    }

    #[test]
    fn a_new_token_on_the_same_page_needs_a_reload() {
        let url = |text: &str| Url::parse(text).expect("url");
        let shown = url("http://127.0.0.1:4321/t/abc");
        assert!(only_the_fragment_moves(&shown, &url("http://127.0.0.1:4321/t/abc#token=new")));
        assert!(!only_the_fragment_moves(&shown, &url("http://127.0.0.1:5555/t/abc#token=new")));
        assert!(!only_the_fragment_moves(&shown, &url("http://127.0.0.1:4321/t/abc?tab=x#token=new")));
    }

    /// A UI on a free port that answers each request with `reply`, or
    /// never, and its address.
    fn serve(reply: Option<&'static str>) -> Url {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                match reply {
                    Some(reply) => {
                        let _ = stream.write_all(reply.as_bytes());
                    }
                    None => std::thread::sleep(Duration::from_secs(5)),
                }
            }
        });
        Url::parse(&format!("http://127.0.0.1:{port}/#token=abc")).expect("url")
    }

    #[test]
    fn the_check_tells_a_running_ui_from_a_restarted_or_stopped_one() {
        let wait = Duration::from_millis(300);
        assert_eq!(check(&serve(Some("HTTP/1.1 200 OK\r\n\r\n")), wait), Answer::Up);
        assert_eq!(check(&serve(Some("HTTP/1.1 401 Unauthorized\r\n\r\n")), wait), Answer::Stale);
        assert_eq!(check(&serve(None), wait), Answer::Up, "busy");
        let free = std::net::TcpListener::bind("127.0.0.1:0").expect("bind").local_addr().expect("addr").port();
        assert_eq!(check(&Url::parse(&format!("http://127.0.0.1:{free}/#token=abc")).expect("url"), wait), Answer::Gone);
    }

    #[test]
    fn a_missing_silt_says_how_to_get_one() {
        let error = open_web(Command::new("/nonexistent/silt")).expect_err("no silt there");
        assert!(matches!(error.frame().error().downcast_ref::<DesktopError>(), Some(DesktopError::NoSilt)));
    }
}
