//! Files from a file manager, pasted or dropped on the page (ADR 0140). A
//! file manager copies and drags files as a `text/uri-list`. WebKitGTK gives
//! the page that type with its `file:` addresses taken out and no `File`s, so
//! the page asks the window for them with a `silt://files/pasted` or
//! `silt://files/dropped` link. The window answers only just after its own
//! paste keys or a drop on it, neither of which a page can make. It takes the
//! files from the clipboard or the drop itself, and serves each once at an
//! unguessable address on its own scheme. The page never names a path.

use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::Url;
use tauri::http::{Response, StatusCode, header};

/// The scheme the window serves the files on.
pub const SCHEME: &str = "silt-files";

/// How soon after the paste keys or the drop the page's ask is answered.
const ASKED_WITHIN: Duration = Duration::from_secs(3);
/// How long an offered file stays to be fetched.
const SERVED_FOR: Duration = Duration::from_secs(60);
/// The most one paste or drop brings, as the page holds all of it in
/// memory; a larger file is attached with +, which reads it from disk.
pub const HANDED_BYTES: u64 = 256 << 20;

/// How the files came.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Way {
    Pasted,
    Dropped,
}

impl Way {
    fn verb(self) -> &'static str {
        match self {
            Self::Pasted => "paste",
            Self::Dropped => "drop",
        }
    }
}

/// The page's ask in a `silt://files/pasted?request=3` link, with the
/// number its answer carries back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ask {
    pub way: Way,
    pub request: u32,
}

pub fn ask_of(link: &Url) -> Option<Ask> {
    if link.scheme() != "silt" || link.host_str() != Some("files") {
        return None;
    }
    let way = match link.path() {
        "/pasted" => Way::Pasted,
        "/dropped" => Way::Dropped,
        _ => return None,
    };
    let request = link.query_pairs().find(|(name, _)| name == "request").and_then(|(_, value)| value.parse().ok())?;
    Some(Ask { way, request })
}

#[cfg_attr(all(not(target_os = "linux"), not(test)), expect(dead_code, reason = "only GTK's side watches keys and drops"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    V,
    Insert,
    Other,
}

/// Whether a key press pastes: Ctrl+V, Ctrl+Shift+V, or Shift+Insert.
#[cfg_attr(all(not(target_os = "linux"), not(test)), expect(dead_code, reason = "only GTK's side watches keys and drops"))]
pub fn is_paste(key: Key, control: bool, shift: bool, alt: bool) -> bool {
    match key {
        Key::V => control && !alt,
        Key::Insert => shift && !control && !alt,
        Key::Other => false,
    }
}

/// A file offered to the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offered {
    pub token: String,
    pub name: String,
    pub size: u64,
    pub mime: String,
}

/// What a paste or a drop brings: the files offered, and why any weren't.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Handed {
    pub files: Vec<Offered>,
    pub refused: Vec<String>,
}

impl Handed {
    pub fn refusing(why: impl Into<String>) -> Self {
        Self { files: Vec::new(), refused: vec![why.into()] }
    }

    /// The script that hands the answer to the page's ask `request`.
    pub fn script(&self, request: u32) -> String {
        let files: Vec<serde_json::Value> = self
            .files
            .iter()
            .map(|file| {
                serde_json::json!({
                    "url": format!("{SCHEME}://localhost/{}", file.token),
                    "name": file.name,
                    "size": file.size,
                    "mime": file.mime,
                })
            })
            .collect();
        let refused = (!self.refused.is_empty()).then(|| self.refused.join("; "));
        let detail = serde_json::json!({ "request": request, "files": files, "refused": refused });
        format!("window.dispatchEvent(new CustomEvent('silt-local-files', {{ detail: {detail} }}));")
    }
}

struct Served {
    path: PathBuf,
    mime: String,
    until: Instant,
}

/// The paste keys last pressed, the files last dragged over the window and
/// dropped on it, and the files offered since.
#[derive(Default)]
pub struct LocalFiles {
    pressed: Mutex<Option<Instant>>,
    #[cfg_attr(all(not(target_os = "linux"), not(test)), expect(dead_code, reason = "only GTK's side watches keys and drops"))]
    dragged: Mutex<Vec<String>>,
    dropped: Mutex<Option<(Instant, Vec<String>)>>,
    served: Mutex<HashMap<String, Served>>,
}

impl LocalFiles {
    #[cfg_attr(all(not(target_os = "linux"), not(test)), expect(dead_code, reason = "only GTK's side watches keys and drops"))]
    pub fn pressed(&self, now: Instant) {
        *self.pressed.lock().expect("lock") = Some(now);
    }

    /// The URIs of what's dragged over the window, as GTK hands them to the
    /// web view before a drop.
    #[cfg_attr(all(not(target_os = "linux"), not(test)), expect(dead_code, reason = "only GTK's side watches keys and drops"))]
    pub fn dragged(&self, uris: Vec<String>) {
        *self.dragged.lock().expect("lock") = uris;
    }

    #[cfg_attr(all(not(target_os = "linux"), not(test)), expect(dead_code, reason = "only GTK's side watches keys and drops"))]
    pub fn dropped(&self, now: Instant) {
        let uris = std::mem::take(&mut *self.dragged.lock().expect("lock"));
        *self.dropped.lock().expect("lock") = Some((now, uris));
    }

    /// Whether an ask comes just after the paste keys. Each press answers
    /// one ask, so a page can't read the clipboard again later.
    pub fn may_answer_paste(&self, now: Instant) -> bool {
        let pressed = self.pressed.lock().expect("lock").take();
        pressed.is_some_and(|at| now.saturating_duration_since(at) <= ASKED_WITHIN)
    }

    /// The URIs of the drop an ask comes just after, once.
    pub fn take_drop(&self, now: Instant) -> Option<Vec<String>> {
        let (at, uris) = self.dropped.lock().expect("lock").take()?;
        (now.saturating_duration_since(at) <= ASKED_WITHIN).then_some(uris)
    }

    /// Offers the regular files among `uris`, each at a fresh `token`, up
    /// to [`HANDED_BYTES`] in all.
    pub fn offer(&self, way: Way, uris: &[String], now: Instant, mut token: impl FnMut() -> String) -> Handed {
        let mut served = self.served.lock().expect("lock");
        served.retain(|_, file| file.until > now);
        let mut handed = Handed::default();
        let mut total = 0;
        for path in uris.iter().filter_map(|uri| file_path(uri)) {
            let name = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
            let size = match readable(&path) {
                Ok(size) => size,
                Err(why) => {
                    handed.refused.push(format!("{name} {why}"));
                    continue;
                }
            };
            if total + size > HANDED_BYTES {
                handed.refused.push(format!("{name} is too large to {} here: attach it with +", way.verb()));
                continue;
            }
            total += size;
            let mime = mime_guess::from_path(&path).first_raw().unwrap_or_default().to_owned();
            let token = token();
            served.insert(token.clone(), Served { path, mime: mime.clone(), until: now + SERVED_FOR });
            handed.files.push(Offered { token, name, size, mime });
        }
        if handed.files.is_empty() && handed.refused.is_empty() {
            handed.refused.push(match way {
                Way::Pasted => "The clipboard has no files to attach".to_owned(),
                Way::Dropped => "Nothing dropped was a file to attach".to_owned(),
            });
        }
        handed
    }

    /// The file offered at `token`, once.
    pub fn take(&self, token: &str, now: Instant) -> Option<(PathBuf, String)> {
        let served = self.served.lock().expect("lock").remove(token)?;
        (served.until > now).then_some((served.path, served.mime))
    }

    /// The response to a request for `uri` on [`SCHEME`]. It reads the
    /// file, so it's for a thread of its own.
    pub fn respond(&self, uri: &str, now: Instant) -> Response<Vec<u8>> {
        let token = Url::parse(uri).ok().map(|uri| uri.path().trim_start_matches('/').to_owned()).unwrap_or_default();
        let Some((path, mime)) = self.take(&token, now) else {
            return response(StatusCode::NOT_FOUND, "", Vec::new());
        };
        match fs::read(&path) {
            Ok(bytes) => response(StatusCode::OK, &mime, bytes),
            Err(error) => response(StatusCode::INTERNAL_SERVER_ERROR, "text/plain", error.to_string().into_bytes()),
        }
    }
}

fn response(status: StatusCode, mime: &str, body: Vec<u8>) -> Response<Vec<u8>> {
    let mut response = Response::builder()
        .status(status)
        // The page is the daemon's, on another origin; the token is what
        // keeps the file to it.
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::CACHE_CONTROL, "no-store");
    if !mime.is_empty() {
        response = response.header(header::CONTENT_TYPE, mime);
    }
    response.body(body).expect("a valid response")
}

/// The local path a URI names, if it's a `file:` one.
fn file_path(uri: &str) -> Option<PathBuf> {
    let uri = Url::parse(uri.trim()).ok()?;
    if uri.scheme() != "file" {
        return None;
    }
    uri.to_file_path().ok()
}

/// The size of a regular file this app can read, or why it can't be handed over.
fn readable(path: &Path) -> Result<u64, String> {
    let metadata = fs::metadata(path).map_err(|error| format!("can't be read: {error}"))?;
    if metadata.is_dir() {
        return Err("is a folder: attach the files in it".to_owned());
    }
    if !metadata.is_file() {
        return Err("isn't a file".to_owned());
    }
    File::open(path).map_err(|error| format!("can't be read: {error}"))?;
    Ok(metadata.len())
}

/// The paste keys, drops and the clipboard, through GTK.
#[cfg(target_os = "linux")]
pub mod native {
    use std::sync::Arc;
    use std::time::Instant;

    use gtk::prelude::*;
    use gtk::{gdk, glib};
    use tauri::{AppHandle, WebviewWindow};

    use super::{Key, LocalFiles, is_paste};

    /// Notes each paste key the window's web view gets and each drop on
    /// it, before WebKit handles them, which goes on as before.
    pub fn watch(window: &WebviewWindow, files: Arc<LocalFiles>) -> tauri::Result<()> {
        window.with_webview(move |webview| {
            let view = webview.inner();
            let pasting = files.clone();
            view.connect_key_press_event(move |_, event| {
                let keyval = event.keyval().to_lower();
                let key = if keyval == gdk::keys::constants::v {
                    Key::V
                } else if keyval == gdk::keys::constants::Insert {
                    Key::Insert
                } else {
                    Key::Other
                };
                let state = event.state();
                let (control, shift, alt) = (
                    state.contains(gdk::ModifierType::CONTROL_MASK),
                    state.contains(gdk::ModifierType::SHIFT_MASK),
                    state.contains(gdk::ModifierType::MOD1_MASK),
                );
                if is_paste(key, control, shift, alt) {
                    pasting.pressed(Instant::now());
                }
                glib::Propagation::Proceed
            });
            // WebKit asks for what's dragged as it enters the view, so the
            // URIs arrive before the drop.
            let dragging = files.clone();
            view.connect_drag_data_received(move |_, _, _, _, data, _, _| {
                let uris = data.uris();
                if !uris.is_empty() {
                    dragging.dragged(uris.iter().map(ToString::to_string).collect());
                }
            });
            view.connect_drag_drop(move |_, _, _, _, _| {
                files.dropped(Instant::now());
                // Not handled here: WebKit's own handler takes the drop.
                false
            });
        })
    }

    /// Hands `then` the clipboard's URIs, read on GTK's thread.
    pub fn clipboard(app: &AppHandle, then: impl FnOnce(Vec<String>) + Send + 'static) {
        let _ = app.run_on_main_thread(move || {
            gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD)
                .request_uris(move |_, uris| then(uris.iter().map(ToString::to_string).collect()));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(text: &str) -> Option<Ask> {
        ask_of(&Url::parse(text).expect("a link"))
    }

    #[test]
    fn only_the_windows_links_ask_for_files() {
        assert_eq!(link("silt://files/pasted?request=3&key=k"), Some(Ask { way: Way::Pasted, request: 3 }));
        assert_eq!(link("silt://files/dropped?request=4&key=k"), Some(Ask { way: Way::Dropped, request: 4 }));
        assert_eq!(link("silt://files/pasted?key=k"), None);
        assert_eq!(link("silt://files/other?request=3"), None);
        assert_eq!(link("silt://zoom/?request=3"), None);
        assert_eq!(link("https://files/pasted?request=3"), None);
    }

    #[test]
    fn ctrl_v_ctrl_shift_v_and_shift_insert_paste() {
        assert!(is_paste(Key::V, true, false, false));
        assert!(is_paste(Key::V, true, true, false));
        assert!(is_paste(Key::Insert, false, true, false));
        assert!(!is_paste(Key::V, false, false, false));
        assert!(!is_paste(Key::V, true, false, true));
        assert!(!is_paste(Key::Insert, false, false, false));
        assert!(!is_paste(Key::Insert, true, true, false));
        assert!(!is_paste(Key::Other, true, false, false));
    }

    #[test]
    fn a_paste_is_answered_once_and_only_just_after_the_keys() {
        let files = LocalFiles::default();
        let now = Instant::now();
        assert!(!files.may_answer_paste(now), "no keys pressed");
        files.pressed(now);
        assert!(files.may_answer_paste(now + Duration::from_millis(100)));
        assert!(!files.may_answer_paste(now + Duration::from_millis(200)), "one press, one answer");
        files.pressed(now);
        assert!(!files.may_answer_paste(now + ASKED_WITHIN + Duration::from_millis(1)), "too late");
    }

    #[test]
    fn a_drop_hands_over_what_was_dragged_once_and_only_just_after() {
        let files = LocalFiles::default();
        let now = Instant::now();
        assert_eq!(files.take_drop(now), None, "nothing dropped");
        files.dragged(vec!["file:///home/me/a.xls".into()]);
        files.dropped(now);
        assert_eq!(files.take_drop(now + Duration::from_millis(100)), Some(vec!["file:///home/me/a.xls".into()]));
        assert_eq!(files.take_drop(now + Duration::from_millis(200)), None, "one drop, one answer");
        files.dropped(now);
        assert_eq!(files.take_drop(now), Some(Vec::new()), "a drop hands over only what was dragged for it");
        files.dragged(vec!["file:///home/me/b.xls".into()]);
        files.dropped(now);
        assert_eq!(files.take_drop(now + ASKED_WITHIN + Duration::from_millis(1)), None, "too late");
    }

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("silt-desktop-files-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("a temporary dir");
            Self(dir)
        }

        fn uri(&self, name: &str) -> String {
            Url::from_file_path(self.0.join(name)).expect("an absolute path").to_string()
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn tokens() -> impl FnMut() -> String {
        let mut next = 0;
        move || {
            next += 1;
            format!("t{next}")
        }
    }

    #[test]
    fn regular_files_are_offered_and_the_rest_say_why_not() {
        let dir = Dir::new("offer");
        fs::write(dir.0.join("budget.xls"), b"cells").expect("a file");
        fs::write(dir.0.join("photo.png"), b"pixels").expect("a file");
        fs::create_dir(dir.0.join("folder")).expect("a folder");
        let uris = [
            dir.uri("budget.xls"),
            "https://example.com/not-a-file".to_owned(),
            dir.uri("folder"),
            dir.uri("photo.png"),
            dir.uri("gone.txt"),
        ];
        let handed = LocalFiles::default().offer(Way::Pasted, &uris, Instant::now(), tokens());
        let offered: Vec<_> =
            handed.files.iter().map(|file| (file.token.as_str(), file.name.as_str(), file.size, file.mime.as_str())).collect();
        assert_eq!(offered, [("t1", "budget.xls", 5, "application/vnd.ms-excel"), ("t2", "photo.png", 6, "image/png")]);
        assert_eq!(handed.refused.len(), 2, "{:?}", handed.refused);
        assert!(handed.refused[0].starts_with("folder is a folder"), "{:?}", handed.refused);
        assert!(handed.refused[1].starts_with("gone.txt can't be read"), "{:?}", handed.refused);
    }

    #[test]
    fn a_paste_or_drop_brings_up_to_its_limit() {
        let dir = Dir::new("large");
        let large = File::create(dir.0.join("video.mp4")).expect("a file");
        large.set_len(HANDED_BYTES - 2).expect("a sparse file");
        fs::write(dir.0.join("notes.txt"), b"three").expect("a file");
        fs::write(dir.0.join("a.txt"), b"a").expect("a file");
        let uris = [dir.uri("video.mp4"), dir.uri("notes.txt"), dir.uri("a.txt")];
        let handed = LocalFiles::default().offer(Way::Dropped, &uris, Instant::now(), tokens());
        let names: Vec<_> = handed.files.iter().map(|file| file.name.as_str()).collect();
        assert_eq!(names, ["video.mp4", "a.txt"]);
        assert_eq!(handed.refused, ["notes.txt is too large to drop here: attach it with +"]);
    }

    #[test]
    fn no_files_says_so() {
        let link = ["https://example.com/".to_owned()];
        let pasted = LocalFiles::default().offer(Way::Pasted, &link, Instant::now(), tokens());
        assert_eq!(pasted, Handed::refusing("The clipboard has no files to attach"));
        let dropped = LocalFiles::default().offer(Way::Dropped, &[], Instant::now(), tokens());
        assert_eq!(dropped, Handed::refusing("Nothing dropped was a file to attach"));
    }

    #[test]
    fn an_offered_file_is_served_once_until_it_expires() {
        let dir = Dir::new("serve");
        fs::write(dir.0.join("budget.xls"), b"cells").expect("a file");
        fs::write(dir.0.join("later.txt"), b"later").expect("a file");
        let files = LocalFiles::default();
        let now = Instant::now();
        files.offer(Way::Pasted, &[dir.uri("budget.xls"), dir.uri("later.txt")], now, tokens());

        let served = files.respond(&format!("{SCHEME}://localhost/t1"), now);
        assert_eq!(served.status(), StatusCode::OK);
        assert_eq!(served.body(), b"cells");
        assert_eq!(served.headers()[header::CONTENT_TYPE], "application/vnd.ms-excel");
        assert_eq!(served.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        assert_eq!(files.respond(&format!("{SCHEME}://localhost/t1"), now).status(), StatusCode::NOT_FOUND);
        assert_eq!(files.respond(&format!("{SCHEME}://localhost/t9"), now).status(), StatusCode::NOT_FOUND);
        let late = now + SERVED_FOR + Duration::from_secs(1);
        assert_eq!(files.respond(&format!("{SCHEME}://localhost/t2"), late).status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn the_answer_tells_the_page_where_each_file_is() {
        let handed = Handed {
            files: vec![Offered {
                token: "abc".into(),
                name: "a \"b\".xls".into(),
                size: 5,
                mime: "application/vnd.ms-excel".into(),
            }],
            refused: vec!["one".into(), "two".into()],
        };
        let script = handed.script(7);
        assert!(script.starts_with("window.dispatchEvent(new CustomEvent('silt-local-files', { detail: "), "{script}");
        let detail = script.split("detail: ").nth(1).and_then(|rest| rest.strip_suffix(" }));")).expect("the detail");
        let detail: serde_json::Value = serde_json::from_str(detail).expect("JSON");
        assert_eq!(
            detail,
            serde_json::json!({
                "request": 7,
                "files": [{ "url": "silt-files://localhost/abc", "name": "a \"b\".xls", "size": 5, "mime": "application/vnd.ms-excel" }],
                "refused": "one; two",
            })
        );
        assert!(Handed::default().script(1).contains("\"refused\":null"));
    }
}
