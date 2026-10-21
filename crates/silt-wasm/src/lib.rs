//! A replica of one thread family in the browser (ADR 0014). The web UI
//! loads the family's deltas into it, edits files locally as new deltas,
//! and merges everyone else's as they arrive, so co-editing never waits on
//! the network.

use std::collections::{HashMap, VecDeque};
use std::str::FromStr;

use silt_checkout::text_edits;
use silt_log::{Author, Delta, Family, FileView, LogError, Op};
use silt_types::{ActorId, Frontier, GitOid, ThreadId};
use wasm_bindgen::prelude::*;

/// The replica's logic, free of JavaScript types so it runs in tests.
pub struct Core {
    family: Family,
    bases: HashMap<GitOid, String>,
    actor: ActorId,
    thread: ThreadId,
    /// Deltas received but not applied yet, waiting for a git base text.
    pending: VecDeque<Delta>,
    received: usize,
    /// What the editor shows of each file and the version it is of. Edits
    /// are made against it, so changes that arrived since are merged, not
    /// undone.
    shown: HashMap<String, (Frontier, String)>,
}

impl Core {
    pub fn new(thread: &str, actor: &str) -> Result<Self, String> {
        Ok(Self {
            family: Family::new(),
            bases: HashMap::new(),
            actor: ActorId::from_str(actor).map_err(|error| error.to_string())?,
            thread: ThreadId::from_str(thread).map_err(|error| error.to_string())?,
            pending: VecDeque::new(),
            received: 0,
            shown: HashMap::new(),
        })
    }

    /// Takes the daemon's deltas from position `after` on. Ones already
    /// received, as when two syncs overlap, are skipped; a batch that would
    /// leave a gap is refused.
    pub fn receive(&mut self, after: usize, deltas: Vec<Delta>) -> Result<Option<GitOid>, String> {
        if after > self.received {
            return Err(format!("deltas from {after} on can't follow the {} received", self.received));
        }
        let fresh: Vec<Delta> = deltas.into_iter().skip(self.received - after).collect();
        self.received += fresh.len();
        self.pending.extend(fresh);
        self.apply_pending()
    }

    pub fn add_base(&mut self, oid: GitOid, text: String) -> Result<Option<GitOid>, String> {
        self.bases.insert(oid, text);
        self.apply_pending()
    }

    /// A delta that can't apply is dropped, as the daemon does (ADR 0016);
    /// one waiting for a git base stays first until the text arrives.
    fn apply_pending(&mut self) -> Result<Option<GitOid>, String> {
        while let Some(delta) = self.pending.front().cloned() {
            match self.family.apply(delta, &mut self.bases) {
                Err(LogError::MissingBase(oid)) => return Ok(Some(oid)),
                Ok(_) | Err(_) => {
                    self.pending.pop_front();
                }
            }
        }
        Ok(None)
    }

    pub fn paths(&self) -> Vec<String> {
        self.family.thread_view(self.thread).paths().keys().cloned().collect()
    }

    pub fn needed_base(&self, path: &str) -> Option<GitOid> {
        let inode = *self.family.thread_view(self.thread).paths().get(path)?;
        match self.family.file(inode)? {
            FileView::GitText(oid) if !self.bases.contains_key(&oid) => Some(oid),
            _ => None,
        }
    }

    pub fn text(&self, path: &str) -> Option<String> {
        let view = self.family.thread_view(self.thread);
        let inode = *view.paths().get(path)?;
        match view.file(inode)? {
            FileView::Text(text) => Some(text),
            FileView::GitText(oid) => self.bases.get(&oid).cloned(),
            FileView::Blob(_) => None,
        }
    }

    pub fn version(&self) -> String {
        self.family.thread_head(self.thread).ids().iter().map(ToString::to_string).collect::<Vec<_>>().join(",")
    }

    /// The file's current text, now shown in the editor.
    pub fn show(&mut self, path: &str) -> Option<String> {
        let text = self.text(path)?;
        self.shown.insert(path.to_owned(), (self.family.thread_head(self.thread), text.clone()));
        Some(text)
    }

    /// The editor turned what it showed of `path` into `text`: makes those
    /// edits, as this replica's actor, at the version it showed, and
    /// returns them.
    pub fn edit(&mut self, path: &str, text: &str, time_ms: i64) -> Result<Vec<Delta>, String> {
        let inode = *self
            .family
            .thread_view(self.thread)
            .paths()
            .get(path)
            .ok_or_else(|| format!("{path} is not a file of this thread"))?;
        let (mut parents, old) = match self.shown.get(path) {
            Some(shown) => shown.clone(),
            None => (self.family.thread_head(self.thread), self.text(path).ok_or_else(|| format!("{path} has no text to edit"))?),
        };
        self.family.text_doc_or_base(inode, &mut self.bases).map_err(|error| error.to_string())?;
        let mut made = Vec::new();
        for op in text_edits(&old, text) {
            let delta =
                self.family.delta(self.actor, self.thread, Author::User, time_ms, parents.clone(), Op::Text { inode, op });
            self.family.apply(delta.clone(), &mut self.bases).map_err(|error| error.to_string())?;
            parents = Frontier::single(delta.span.last());
            made.push(delta);
        }
        self.shown.insert(path.to_owned(), (parents, text.to_owned()));
        Ok(made)
    }
}

/// The replica as the web UI sees it; deltas travel postcard-encoded.
#[wasm_bindgen]
pub struct Replica(Core);

fn js(message: impl std::fmt::Display) -> JsError {
    JsError::new(&message.to_string())
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn error(message: &str);
}

/// A panic traps the module, and the replica that was in use stays borrowed
/// for good; its message goes to the console, as the trap itself says
/// nothing.
#[wasm_bindgen(start)]
fn start() {
    std::panic::set_hook(Box::new(|info| error(&format!("silt replica: {info}"))));
}

#[wasm_bindgen]
impl Replica {
    /// A replica writing as `actor` (32 hex digits, fresh for each tab)
    /// into `thread`.
    #[wasm_bindgen(constructor)]
    pub fn new(thread: &str, actor: &str) -> Result<Replica, JsError> {
        Core::new(thread, actor).map(Self).map_err(js)
    }

    /// How many of the daemon's deltas were received; ask for those after.
    pub fn received(&self) -> usize {
        self.0.received
    }

    /// Takes deltas from the daemon, starting at position `after` of its
    /// log. Returns the git blob whose text is needed before the rest can
    /// apply, if any.
    pub fn receive(&mut self, after: usize, bytes: &[u8]) -> Result<Option<String>, JsError> {
        let deltas: Vec<Delta> = postcard::from_bytes(bytes).map_err(js)?;
        Ok(self.0.receive(after, deltas).map_err(js)?.map(|oid| oid.to_string()))
    }

    /// A git blob's text, as asked for by `receive` or `needed_base`.
    pub fn add_base(&mut self, oid: &str, text: String) -> Result<Option<String>, JsError> {
        let oid = GitOid::from_str(oid).map_err(js)?;
        Ok(self.0.add_base(oid, text).map_err(js)?.map(|oid| oid.to_string()))
    }

    /// The thread's files.
    pub fn paths(&self) -> Vec<String> {
        self.0.paths()
    }

    /// The git blob a file's text still comes from, before anyone edited it.
    pub fn needed_base(&self, path: &str) -> Option<String> {
        self.0.needed_base(path).map(|oid| oid.to_string())
    }

    /// The file's text in the thread as it is now.
    pub fn text(&self, path: &str) -> Option<String> {
        self.0.text(path)
    }

    /// The file's text as it is now, to show in the editor; later edits are
    /// made against it.
    pub fn show(&mut self, path: &str) -> Option<String> {
        self.0.show(path)
    }

    /// The thread's head, to tell whether anything changed.
    pub fn version(&self) -> String {
        self.0.version()
    }

    /// The editor's text of `path` is now `text`; returns the deltas that
    /// make it so, to send to the daemon.
    pub fn edit(&mut self, path: &str, text: &str, time_ms: f64) -> Result<Vec<u8>, JsError> {
        #[expect(clippy::cast_possible_truncation, reason = "milliseconds since the epoch fit in an i64")]
        let made = self.0.edit(path, text, time_ms as i64).map_err(js)?;
        postcard::to_allocvec(&made).map_err(js)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use silt_crdt::{InodeKind, Parent};
    use silt_log::Content;

    use super::*;

    const THREAD: &str = "0000000000000000000000000000000a";

    /// The daemon's side: a file `main.rs` that came from git, unedited.
    fn daemon_log() -> (Vec<Delta>, GitOid) {
        let oid = GitOid::from_str("1111111111111111111111111111111111111111").expect("oid");
        let mut family = Family::new();
        let thread = ThreadId::from_str(THREAD).expect("thread");
        let delta = family.delta(
            ActorId(1),
            thread,
            Author::Import,
            0,
            family.thread_head(thread),
            Op::Create {
                kind: InodeKind::File,
                parent: Parent::Root,
                name: "main.rs".into(),
                content: Content::GitText(oid),
                executable: false,
            },
        );
        family.apply(delta.clone(), &mut HashMap::new()).expect("applies");
        (vec![delta], oid)
    }

    #[test]
    fn two_browsers_edit_one_file_and_converge() {
        let (log, oid) = daemon_log();
        let mut alice = Core::new(THREAD, "000000000000000000000000000000a1").expect("replica");
        let mut bob = Core::new(THREAD, "000000000000000000000000000000b2").expect("replica");
        for replica in [&mut alice, &mut bob] {
            assert_eq!(replica.receive(0, log.clone()), Ok(None));
            assert_eq!(replica.paths(), vec!["main.rs".to_owned()]);
            assert_eq!(replica.needed_base("main.rs"), Some(oid));
            assert_eq!(replica.add_base(oid, "fn main() {}\n".into()), Ok(None));
        }
        let from_alice = alice.edit("main.rs", "// alice\nfn main() {}\n", 1).expect("edits");
        let from_bob = bob.edit("main.rs", "fn main() {}\nfn bob() {}\n", 1).expect("edits");
        assert_eq!(bob.receive(1, from_alice), Ok(None));
        assert_eq!(alice.receive(1, from_bob), Ok(None));
        let merged = "// alice\nfn main() {}\nfn bob() {}\n";
        assert_eq!(alice.text("main.rs").as_deref(), Some(merged));
        assert_eq!(bob.text("main.rs").as_deref(), Some(merged));
        assert_eq!(alice.version(), bob.version());
    }

    #[test]
    fn deltas_wait_for_their_base_text() {
        let (log, oid) = daemon_log();
        let mut writer = Core::new(THREAD, "000000000000000000000000000000a1").expect("replica");
        writer.receive(0, log.clone()).expect("receives");
        writer.add_base(oid, "x\n".into()).expect("base");
        let edits = writer.edit("main.rs", "y\n", 1).expect("edits");
        let mut reader = Core::new(THREAD, "000000000000000000000000000000b2").expect("replica");
        reader.receive(0, log).expect("receives");
        assert_eq!(reader.receive(1, edits), Ok(Some(oid)), "the edit needs the file's git text first");
        assert_eq!(reader.add_base(oid, "x\n".into()), Ok(None));
        assert_eq!(reader.text("main.rs").as_deref(), Some("y\n"));
    }

    #[test]
    fn an_edit_keeps_changes_that_arrived_after_the_editor_showed_the_file() {
        let (log, oid) = daemon_log();
        let mut other = Core::new(THREAD, "000000000000000000000000000000b2").expect("replica");
        other.receive(0, log.clone()).expect("receives");
        other.add_base(oid, "x\n".into()).expect("base");
        let remote = other.edit("main.rs", "remote\nx\n", 1).expect("edits");

        let mut here = Core::new(THREAD, "000000000000000000000000000000a1").expect("replica");
        here.receive(0, log).expect("receives");
        here.add_base(oid, "x\n".into()).expect("base");
        assert_eq!(here.show("main.rs").as_deref(), Some("x\n"));
        here.receive(1, remote).expect("someone else's edit arrives");
        here.edit("main.rs", "x\nlocal\n", 2).expect("the editor, still showing x, types a line");
        assert_eq!(here.text("main.rs").as_deref(), Some("remote\nx\nlocal\n"));
        assert_eq!(here.show("main.rs").as_deref(), Some("remote\nx\nlocal\n"));
    }

    #[test]
    fn overlapping_syncs_count_each_delta_once() {
        let (log, oid) = daemon_log();
        let mut writer = Core::new(THREAD, "000000000000000000000000000000a1").expect("replica");
        writer.receive(0, log.clone()).expect("receives");
        writer.add_base(oid, "x\n".into()).expect("base");
        let edits = writer.edit("main.rs", "y\n", 1).expect("edits");
        let daemon: Vec<Delta> = log.into_iter().chain(edits).collect();

        let mut reader = Core::new(THREAD, "000000000000000000000000000000b2").expect("replica");
        reader.add_base(oid, "x\n".into()).expect("base");
        reader.receive(0, daemon[..1].to_vec()).expect("first sync");
        reader.receive(0, daemon.clone()).expect("a second sync that started at the same point");
        assert_eq!(reader.received, daemon.len());
        assert_eq!(reader.text("main.rs").as_deref(), Some("y\n"));
        assert!(reader.receive(daemon.len() + 1, Vec::new()).is_err(), "a gap is refused");
    }
}
