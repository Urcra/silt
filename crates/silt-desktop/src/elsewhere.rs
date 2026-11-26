//! Every connected machine's threads, for the page's All machines (ADR
//! 0132): asking each daemon for its listing over the address the app holds,
//! how the person chose to list and mark them, and the links the page sends
//! about them.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use tauri::Url;

use crate::backends::Backend;

/// The file in the app's data dir that keeps how threads are listed and marked.
pub const VIEW: &str = "view.json";

/// What a machine's daemon is asked for, at most, before it counts as lost.
pub const ASK_WAIT: Duration = Duration::from_secs(5);

/// A listing bigger than this isn't a daemon's.
const MOST_BYTES: usize = 64 * 1024 * 1024;

/// Whether the sidebar lists every connected machine's threads, or the shown one's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum List {
    #[default]
    All,
    One,
}

/// How a thread on another machine is told apart in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mark {
    #[default]
    Chip,
    ColourChip,
    Line,
    Stripe,
    Initial,
    Nothing,
}

const MARKS: [(Mark, &str); 6] = [
    (Mark::Chip, "chip"),
    (Mark::ColourChip, "colour-chip"),
    (Mark::Line, "line"),
    (Mark::Stripe, "stripe"),
    (Mark::Initial, "initial"),
    (Mark::Nothing, "nothing"),
];

impl Mark {
    fn parse(text: &str) -> Option<Self> {
        MARKS.iter().find(|(_, name)| *name == text).map(|(mark, _)| *mark)
    }

    fn name(self) -> &'static str {
        MARKS.iter().find(|(mark, _)| *mark == self).map_or("chip", |(_, name)| name)
    }
}

/// The colours a machine can have, in the order they're given out.
pub const COLOURS: [&str; 6] = ["teal", "plum", "blue", "rose", "sand", "ink"];

/// How the person chose to list and mark threads, which every machine's page follows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct View {
    pub list: List,
    pub mark: Mark,
    /// Mark This computer's threads too, not only other machines'.
    pub mark_here: bool,
    /// The colour picked for a backend, by its id.
    pub colours: BTreeMap<String, String>,
}

impl View {
    /// The view kept in `text`, with whatever doesn't pass the checks left at its default.
    pub fn parse(text: &str) -> Self {
        let Ok(serde_json::Value::Object(kept)) = serde_json::from_str(text) else {
            return Self::default();
        };
        let list = match kept.get("list").and_then(serde_json::Value::as_str) {
            Some("one") => List::One,
            _ => List::All,
        };
        let mark = kept.get("mark").and_then(serde_json::Value::as_str).and_then(Mark::parse).unwrap_or_default();
        let mark_here = kept.get("mark_here").and_then(serde_json::Value::as_bool).unwrap_or(false);
        let colours = kept
            .get("colours")
            .and_then(serde_json::Value::as_object)
            .map(|colours| {
                colours
                    .iter()
                    .filter_map(|(id, colour)| {
                        let colour = colour.as_str().filter(|colour| COLOURS.contains(colour))?;
                        Backend::parse(id).map(|_| (id.clone(), colour.to_owned()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self { list, mark, mark_here, colours }
    }

    pub fn remember(&self) -> String {
        serde_json::to_string_pretty(&serde_json::json!({
            "list": if self.list == List::All { "all" } else { "one" },
            "mark": self.mark.name(),
            "mark_here": self.mark_here,
            "colours": self.colours,
        }))
        .expect("values serialize")
    }

    /// What the page is told of it.
    pub fn for_page(&self) -> serde_json::Value {
        serde_json::json!({
            "list": if self.list == List::All { "all" } else { "one" },
            "mark": self.mark.name(),
            "markHere": self.mark_here,
        })
    }

    /// The colour of the backend `id`, the `at`th the app knows: the one
    /// picked for it, else the next in turn, This computer's being ink.
    pub fn colour(&self, id: &str, at: usize) -> &str {
        if let Some(picked) = self.colours.get(id) {
            return picked;
        }
        if id == Backend::Native.id() {
            return "ink";
        }
        COLOURS[at.saturating_sub(1) % COLOURS.len()]
    }

    /// Makes the change `edit` asks for.
    pub fn apply(&mut self, edit: ViewEdit) {
        match edit {
            ViewEdit::List(list) => self.list = list,
            ViewEdit::Mark { mark, here } => {
                self.mark = mark;
                self.mark_here = here;
            }
            ViewEdit::Colour { backend, colour } => {
                self.colours.insert(backend, colour);
            }
        }
    }
}

/// What a `silt://view/…` link asks to change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewEdit {
    List(List),
    Mark { mark: Mark, here: bool },
    Colour { backend: String, colour: String },
}

fn field(link: &Url, key: &str) -> Option<String> {
    link.query_pairs().find(|(name, _)| name == key).map(|(_, value)| value.into_owned())
}

/// The change a `silt://view/…` link asks for, if it's one and its fields pass the checks.
pub fn view_edit_of(link: &Url) -> Option<ViewEdit> {
    if link.scheme() != "silt" || link.host_str() != Some("view") {
        return None;
    }
    match link.path() {
        "" | "/" => match field(link, "list")?.as_str() {
            "all" => Some(ViewEdit::List(List::All)),
            "one" => Some(ViewEdit::List(List::One)),
            _ => None,
        },
        "/mark" => {
            Some(ViewEdit::Mark { mark: Mark::parse(&field(link, "mark")?)?, here: field(link, "here").as_deref() == Some("1") })
        }
        "/colour" => {
            let backend = field(link, "backend").filter(|id| Backend::parse(id).is_some())?;
            let colour = field(link, "colour").filter(|colour| COLOURS.contains(&colour.as_str()))?;
            Some(ViewEdit::Colour { backend, colour })
        }
        _ => None,
    }
}

/// Whether `text` is a thread's id, and nothing that could become a path.
pub fn is_thread_id(text: &str) -> bool {
    (12..=64).contains(&text.len()) && text.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

/// A computer's name as its threads are marked with it: up to its domain,
/// and nothing that isn't plainly a name.
pub fn short_host(said: &str) -> Option<String> {
    let name = said.trim().split('.').next()?;
    let plain =
        !name.is_empty() && name.len() <= 64 && name.chars().all(|char| char.is_alphanumeric() || char == '-' || char == '_');
    plain.then(|| name.to_owned())
}

/// A change to a thread on another machine: pin, snooze, settle, or its read mark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateChange {
    pub backend: Backend,
    pub thread: String,
    /// The change as that daemon takes it, with nothing else in it.
    pub change: serde_json::Value,
}

/// The change a `silt://threads/state?backend=…&thread=…&change=…` link asks
/// for, on a backend the app knows. The change keeps only a `StateChange`'s
/// fields, each of its kind.
pub fn state_change_of(link: &Url, known: &[Backend]) -> Option<StateChange> {
    if link.scheme() != "silt" || link.host_str() != Some("threads") || link.path() != "/state" {
        return None;
    }
    let backend = Backend::parse(&field(link, "backend")?).filter(|backend| known.contains(backend))?;
    let thread = field(link, "thread").filter(|thread| is_thread_id(thread))?;
    let asked: serde_json::Value = serde_json::from_str(&field(link, "change")?).ok()?;
    let bool_or_null = |key: &str| match asked.get(key) {
        None | Some(serde_json::Value::Null) => Some(serde_json::Value::Null),
        Some(serde_json::Value::Bool(set)) => Some((*set).into()),
        Some(_) => None,
    };
    let snooze = match asked.get("snooze") {
        None | Some(serde_json::Value::Null) => serde_json::Value::Null,
        Some(snooze) => {
            let until = match snooze.get("until") {
                None | Some(serde_json::Value::Null) => serde_json::Value::Null,
                Some(until) => until.as_u64()?.into(),
            };
            let wake = snooze.get("wake_on_activity").and_then(serde_json::Value::as_bool)?;
            serde_json::json!({ "until": until, "wake_on_activity": wake })
        }
    };
    let reviewed_turn = match asked.get("reviewed_turn") {
        None | Some(serde_json::Value::Null) => serde_json::Value::Null,
        Some(turn) => turn.as_u64()?.into(),
    };
    let seen = match asked.get("seen") {
        None => false,
        Some(seen) => seen.as_bool()?,
    };
    let change = serde_json::json!({
        "pinned": bool_or_null("pinned")?,
        "snooze": snooze,
        "settled": bool_or_null("settled")?,
        "seen": seen,
        "reviewed_turn": reviewed_turn,
    });
    Some(StateChange { backend, thread, change })
}

/// The request for `path` at the daemon whose UI is `ui`, with its token,
/// to its own host, as the daemon only answers that.
pub fn request(ui: &Url, method: &str, path: &str, body: Option<&str>) -> Option<String> {
    let token = ui.fragment()?.strip_prefix("token=")?;
    let port = ui.port()?;
    let mut text = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nAccept: application/json\r\nConnection: close\r\n"
    );
    if let Some(body) = body {
        text.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", body.len()));
    }
    text.push_str("\r\n");
    if let Some(body) = body {
        text.push_str(body);
    }
    Some(text)
}

/// The status and body of a whole HTTP response, chunked or not.
pub fn response(bytes: &[u8]) -> Option<(u16, Vec<u8>)> {
    let end = bytes.windows(4).position(|window| window == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&bytes[..end]).ok()?;
    let rest = &bytes[end + 4..];
    let mut lines = head.split("\r\n");
    let status = lines.next()?.split(' ').nth(1)?.parse().ok()?;
    let header = |name: &str| {
        head.split("\r\n").skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim().eq_ignore_ascii_case(name).then(|| value.trim().to_owned())
        })
    };
    if header("transfer-encoding").is_some_and(|coding| coding.eq_ignore_ascii_case("chunked")) {
        return Some((status, unchunked(rest)?));
    }
    match header("content-length") {
        Some(length) => {
            let length: usize = length.parse().ok()?;
            rest.get(..length).map(|body| (status, body.to_vec()))
        }
        None => Some((status, rest.to_vec())),
    }
}

fn unchunked(mut rest: &[u8]) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let line_end = rest.windows(2).position(|window| window == b"\r\n")?;
        let size = std::str::from_utf8(&rest[..line_end]).ok()?;
        let size = usize::from_str_radix(size.split(';').next()?.trim(), 16).ok()?;
        rest = &rest[line_end + 2..];
        if size == 0 {
            return Some(body);
        }
        body.extend_from_slice(rest.get(..size)?);
        rest = rest.get(size + 2..)?;
    }
}

/// Sends `request` to the daemon on `port` and reads its whole answer.
fn exchange(port: u16, request: &str, wait: Duration) -> io::Result<Vec<u8>> {
    let mut stream = TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), wait)?;
    stream.set_read_timeout(Some(wait))?;
    stream.set_write_timeout(Some(wait))?;
    stream.write_all(request.as_bytes())?;
    let mut answer = Vec::new();
    stream.take(MOST_BYTES as u64).read_to_end(&mut answer)?;
    Ok(answer)
}

/// Asks the daemon at `ui` for its threads, as its own page lists them.
pub fn threads_of(ui: &Url) -> Option<serde_json::Value> {
    let asked = request(ui, "GET", "/api/threads", None)?;
    let (status, body) = response(&exchange(ui.port()?, &asked, ASK_WAIT).ok()?)?;
    let threads: serde_json::Value = serde_json::from_slice(&body).ok()?;
    (status == 200 && threads.is_array()).then_some(threads)
}

/// Makes `change` on the daemon at `ui`, as its own page would.
pub fn change_state(ui: &Url, change: &StateChange) -> bool {
    let path = format!("/api/threads/{}/state", change.thread);
    let Some(asked) = request(ui, "PATCH", &path, Some(&change.change.to_string())) else {
        return false;
    };
    let answer = ui.port().and_then(|port| exchange(port, &asked, ASK_WAIT).ok());
    answer.as_deref().and_then(response).is_some_and(|(status, _)| (200..300).contains(&status))
}

/// How a backend's threads stand for the page.
#[derive(Debug, Clone, PartialEq)]
pub enum Listing {
    Loading,
    Ready(serde_json::Value),
    /// It answered before and doesn't now.
    Lost,
}

/// The script that tells the page every listing, by backend id.
pub fn script(listings: &BTreeMap<String, Listing>) -> String {
    let told: Vec<serde_json::Value> = listings
        .iter()
        .map(|(id, listing)| match listing {
            Listing::Loading => serde_json::json!({ "id": id, "state": "loading", "threads": [] }),
            Listing::Ready(threads) => serde_json::json!({ "id": id, "state": "ready", "threads": threads }),
            Listing::Lost => serde_json::json!({ "id": id, "state": "lost", "threads": [] }),
        })
        .collect();
    format!("window.__siltThreads = {}; window.dispatchEvent(new CustomEvent('silt-threads'));", serde_json::Value::Array(told))
}

/// The script that tells the page how threads are listed and marked.
pub fn view_script(view: &View) -> String {
    format!(
        "window.__siltDesktop = Object.assign(window.__siltDesktop || {{}}, {{ view: {} }}); window.dispatchEvent(new CustomEvent('silt-view'));",
        view.for_page()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(text: &str) -> Url {
        Url::parse(text).expect("a URL")
    }

    #[test]
    fn a_view_round_trips_and_what_isnt_one_is_dropped() {
        let mut view = View::default();
        assert_eq!(view.list, List::All, "every connected machine, unless the person picks one");
        view.apply(ViewEdit::List(List::One));
        view.apply(ViewEdit::Mark { mark: Mark::Stripe, here: true });
        view.apply(ViewEdit::Colour { backend: "ssh:nuc".into(), colour: "rose".into() });
        assert_eq!(View::parse(&view.remember()), view);
        let kept = r#"{"list":"sideways","mark":"glitter","colours":{"ssh:nuc":"plaid","nope":"teal","wsl:Ubuntu":"sand"}}"#;
        let read = View::parse(kept);
        assert_eq!((read.list, read.mark), (List::All, Mark::Chip));
        assert_eq!(read.colours.into_iter().collect::<Vec<_>>(), [("wsl:Ubuntu".to_owned(), "sand".to_owned())]);
        assert_eq!(View::parse("not json"), View::default());
    }

    #[test]
    fn colours_go_out_in_turn_and_a_pick_wins() {
        let mut view = View::default();
        assert_eq!(view.colour("native", 0), "ink");
        assert_eq!(view.colour("ssh:nuc", 1), "teal");
        assert_eq!(view.colour("wsl:Ubuntu", 2), "plum");
        assert_eq!(view.colour("ssh:far", 7), "teal", "they come round again");
        view.colours.insert("ssh:nuc".into(), "sand".into());
        assert_eq!(view.colour("ssh:nuc", 1), "sand");
    }

    #[test]
    fn view_links_are_read_only_with_their_fields_in_order() {
        assert_eq!(view_edit_of(&link("silt://view/?list=one")), Some(ViewEdit::List(List::One)));
        assert_eq!(
            view_edit_of(&link("silt://view/mark?mark=colour-chip&here=1")),
            Some(ViewEdit::Mark { mark: Mark::ColourChip, here: true })
        );
        assert_eq!(
            view_edit_of(&link("silt://view/colour?backend=ssh:nuc&colour=blue")),
            Some(ViewEdit::Colour { backend: "ssh:nuc".into(), colour: "blue".into() })
        );
        for wrong in [
            "silt://view/?list=some",
            "silt://view/mark?mark=sparkle",
            "silt://view/colour?backend=ssh:nuc&colour=red",
            "silt://view/colour?backend=elsewhere&colour=blue",
            "silt://machines/?list=all",
            "silt://view/other?list=all",
        ] {
            assert_eq!(view_edit_of(&link(wrong)), None, "{wrong}");
        }
    }

    #[test]
    fn only_a_thread_id_names_a_thread() {
        assert!(is_thread_id("a81cee7abcb7"));
        assert!(is_thread_id(&"0f".repeat(32)));
        for wrong in ["", "a81cee7", "A81CEE7ABCB7", "../../etc/pa", "a81cee7abcb7/x", &"0".repeat(65)] {
            assert!(!is_thread_id(wrong), "{wrong}");
        }
    }

    #[test]
    fn a_computer_is_named_up_to_its_domain() {
        assert_eq!(short_host("framework\n").as_deref(), Some("framework"));
        assert_eq!(short_host("MacBook-Pro.local").as_deref(), Some("MacBook-Pro"));
        assert_eq!(short_host(""), None);
        assert_eq!(short_host("<script>"), None);
    }

    #[test]
    fn a_state_change_keeps_only_its_own_fields() {
        let known = [Backend::Native, Backend::Ssh("nuc".into())];
        let asked = |change: &str| {
            let mut url = link("silt://threads/state");
            url.query_pairs_mut()
                .append_pair("backend", "ssh:nuc")
                .append_pair("thread", "a81cee7abcb7")
                .append_pair("change", change);
            state_change_of(&url, &known)
        };
        let pinned = asked(r#"{"pinned":true,"settled":null,"seen":false,"extra":"dropped"}"#).expect("a change");
        assert_eq!(pinned.backend, Backend::Ssh("nuc".into()));
        assert_eq!(
            pinned.change,
            serde_json::json!({ "pinned": true, "snooze": null, "settled": null, "seen": false, "reviewed_turn": null })
        );
        let snoozed = asked(r#"{"snooze":{"until":1700000000000,"wake_on_activity":true}}"#).expect("a change");
        assert_eq!(snoozed.change["snooze"], serde_json::json!({ "until": 1_700_000_000_000_u64, "wake_on_activity": true }));
        for wrong in [r#"{"pinned":"yes"}"#, r#"{"snooze":{"until":"soon","wake_on_activity":true}}"#, r#"{"seen":1}"#, "nope"] {
            assert_eq!(asked(wrong), None, "{wrong}");
        }
        let elsewhere = link("silt://threads/state?backend=ssh:other&thread=a81cee7abcb7&change=%7B%7D");
        assert_eq!(state_change_of(&elsewhere, &known), None, "a machine the app doesn't know");
        let path = link("silt://threads/state?backend=native&thread=..%2Fstatus&change=%7B%7D");
        assert_eq!(state_change_of(&path, &known), None);
    }

    #[test]
    fn requests_carry_the_token_to_the_daemons_own_host() {
        let ui = link("http://127.0.0.1:4321/#token=abc");
        let get = request(&ui, "GET", "/api/threads", None).expect("a request");
        assert!(get.starts_with("GET /api/threads HTTP/1.1\r\nHost: 127.0.0.1:4321\r\nAuthorization: Bearer abc\r\n"));
        assert!(get.ends_with("\r\n\r\n"));
        let patch = request(&ui, "PATCH", "/api/threads/a/state", Some("{}")).expect("a request");
        assert!(patch.contains("\r\nContent-Length: 2\r\n") && patch.ends_with("\r\n\r\n{}"));
        assert_eq!(request(&link("http://127.0.0.1:4321/"), "GET", "/", None), None);
    }

    #[test]
    fn responses_are_read_whole_chunked_or_not() {
        assert_eq!(response(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n[]"), Some((200, b"[]".to_vec())));
        assert_eq!(
            response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n[1,\r\n2\r\n2]\r\n0\r\n\r\n"),
            Some((200, b"[1,2]".to_vec()))
        );
        assert_eq!(response(b"HTTP/1.1 401 Unauthorized\r\n\r\n"), Some((401, Vec::new())));
        assert_eq!(response(b"HTTP/1.1 200 OK\r\ncontent-length: 9\r\n\r\n[]"), None, "cut short");
        assert_eq!(response(b"garbage"), None);
    }

    /// A daemon on a free port that answers once with `reply`, and its UI's address.
    fn serve(reply: &'static str) -> Url {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        std::thread::spawn(move || {
            if let Some(mut stream) = listener.incoming().flatten().next() {
                let mut asked = [0; 1024];
                let _ = stream.read(&mut asked);
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        link(&format!("http://127.0.0.1:{port}/#token=abc"))
    }

    #[test]
    fn a_daemon_s_threads_are_its_listing_and_nothing_else() {
        let listed = serve("HTTP/1.1 200 OK\r\ncontent-length: 14\r\n\r\n[{\"id\":\"abc\"}]");
        assert_eq!(threads_of(&listed), Some(serde_json::json!([{ "id": "abc" }])));
        assert_eq!(threads_of(&serve("HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\n\r\n")), None);
        assert_eq!(threads_of(&serve("HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}")), None, "not a list");
        let free = std::net::TcpListener::bind("127.0.0.1:0").expect("bind").local_addr().expect("addr").port();
        assert_eq!(threads_of(&link(&format!("http://127.0.0.1:{free}/#token=abc"))), None);
    }

    #[test]
    fn the_page_hears_each_listing_by_its_backend() {
        let mut listings = BTreeMap::new();
        listings.insert("ssh:nuc".to_owned(), Listing::Ready(serde_json::json!([{ "id": "abc" }])));
        listings.insert("wsl:Ubuntu".to_owned(), Listing::Lost);
        let told = script(&listings);
        assert!(told.starts_with(
            r#"window.__siltThreads = [{"id":"ssh:nuc","state":"ready","threads":[{"id":"abc"}]},{"id":"wsl:Ubuntu","state":"lost","threads":[]}];"#
        ));
        assert!(view_script(&View::default()).contains(r#"view: {"list":"all","mark":"chip","markHere":false}"#));
    }
}
