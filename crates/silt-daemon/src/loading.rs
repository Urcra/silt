//! Events that name a family the daemon doesn't hold wait while the loader
//! loads it, and the core goes on with others (ADR 0116). So do requests for
//! a history the daemon doesn't keep, while it's read whole off the core
//! (ADR 0127).

use std::collections::{BTreeSet, HashMap};

use silt_types::{FamilyId, ThreadId};
use tokio::sync::oneshot;
use tracing::warn;

use crate::core::{Core, CoreError, Event};
use crate::families::Fetched;
use crate::history::WholeRead;
use crate::load::Loaded;
use crate::protocol::Request;

/// The events waiting for each family being loaded, in the order they came,
/// the requests waiting for the stale digests to be taken, and those
/// waiting for each history being read whole.
#[derive(Default)]
pub(crate) struct Waiting(HashMap<FamilyId, Vec<Event>>, Vec<Event>, HashMap<ThreadId, Vec<Event>>);

impl Core {
    /// The event, if it can be handled now; else it waits for the families
    /// it names to load.
    pub(crate) fn unless_loading(&mut self, event: Event) -> Option<Event> {
        if let Some((thread, cut)) = self.history_asked(&event) {
            return self.unless_reading(thread, cut, event);
        }
        let named = match &event {
            // A hook's family is held while its session is; and hooks must
            // be taken in the order their agent sent them.
            Event::Request(Request::Hook { .. } | Request::Scoped { .. } | Request::Shutdown | Request::Ping, _) => {
                return Some(event);
            }
            // Views across families would load every family without a digest
            // and hold them all; they wait for the digests instead.
            Event::Request(..) if !self.stale_digests.is_empty() => {
                self.waiting.1.push(event);
                return None;
            }
            Event::Request(request, _) => self.families_named(request),
            Event::Changed { checkout, .. } => self.checkouts.get(checkout).map(|held| held.family).into_iter().collect(),
            _ => return Some(event),
        };
        let Some(family) = named.into_iter().find(|family| self.waiting.0.contains_key(family) || self.unheld(*family)) else {
            return Some(event);
        };
        self.load_later(family, Vec::new());
        self.waiting.0.entry(family).or_default().push(event);
        None
    }

    /// The thread whose history the request reads, and whether it asks for
    /// it cut, as a page of the web UI carries it. Not while the digests are
    /// taken, which every request waits for.
    fn history_asked(&self, event: &Event) -> Option<(ThreadId, bool)> {
        if !self.stale_digests.is_empty() {
            return None;
        }
        let (thread, cut) = match event {
            Event::Request(Request::HistoryRead { thread, cut, .. }, _) => (thread, *cut),
            Event::Request(Request::Turns { thread }, _) => (thread, true),
            _ => return None,
        };
        self.find_thread(thread).ok().map(|thread| (thread, cut))
    }

    /// The request for `thread`'s history, if it can be answered now. A page
    /// of a history kept as it is now needs no family loaded. A family not
    /// held is loaded with the history read in it; a history not kept is
    /// read on a blocking thread. Either way the core goes on meanwhile: a
    /// long thread read whole would hold up every other event for a second
    /// or more, the web UI switching threads among them.
    fn unless_reading(&mut self, thread: ThreadId, cut: bool, event: Event) -> Option<Event> {
        if cut && self.history_kept_now(thread) {
            return Some(event);
        }
        let family = self.threads[&thread].family;
        if self.waiting.0.contains_key(&family) || self.unheld(family) {
            self.load_later(family, vec![thread]);
            self.waiting.0.entry(family).or_default().push(event);
            return None;
        }
        if !self.waiting.2.contains_key(&thread) && !self.history_unkept(thread) {
            return Some(event);
        }
        self.read_later(thread);
        self.waiting.2.entry(thread).or_default().push(event);
        None
    }

    fn read_later(&self, thread: ThreadId) {
        if self.waiting.2.contains_key(&thread) {
            return;
        }
        let family = self.threads[&thread].family;
        let events = self.events.clone();
        // A log let go of (ADR 0116) would be read back here, on the core,
        // before the history could be cloned from it: the loader reads both
        // in a copy of the family instead. A log is let go of only while the
        // store holds just its deltas, so the copy's is the same.
        if let Some(loader) = self.families.loader()
            && self.families.loaded(&family).is_some_and(|held| !held.log_decoded())
        {
            let answer = loader.load_later(family, self.families.root(&family).map(ToOwned::to_owned), vec![thread]);
            tokio::spawn(async move {
                let whole = answer.await.ok().and_then(|mut fetched| fetched.histories.pop()).map(Box::new);
                let _ = events.send(Event::HistoryRead { thread, whole, copy: true });
            });
            return;
        }
        let to_read = self.history_to_read(thread);
        tokio::spawn(async move {
            let whole = tokio::task::spawn_blocking(move || to_read.read()).await.ok().map(Box::new);
            let _ = events.send(Event::HistoryRead { thread, whole, copy: false });
        });
    }

    /// A history read whole off the core is kept, and the requests waiting
    /// for it go on. One read in a copy of a family that changed since is
    /// dropped, as the copy may not have been the same, and one that failed
    /// is read here instead, so the requests needn't wait for another.
    pub(crate) fn history_read(&mut self, thread: ThreadId, whole: Option<WholeRead>, copy: bool) {
        let length = self.threads.get(&thread).map(|record| self.local_log_len(record.family));
        match whole {
            Some(whole) if copy && Some(whole.version()) != length => {}
            Some(whole) => self.take_whole(whole),
            None => warn!(%thread, "could not read the history off the core; it's read here"),
        }
        let waiting = self.waiting.2.remove(&thread).unwrap_or_default();
        if self.threads.contains_key(&thread) && self.history_unkept(thread) {
            self.history_entries(thread);
        }
        for event in waiting {
            self.send_on(event);
        }
    }

    /// Sends on an event that waited, a page's view ahead of the rest again.
    fn send_on(&self, event: Event) {
        let to = match &event {
            Event::Request(request, _) if request.is_view() => &self.views.sender,
            _ => &self.events,
        };
        let _ = to.send(event);
    }

    /// Answers the views of pages waiting now, between the steps of long
    /// work such as recording what an agent wrote (ADR 0127). Each step
    /// leaves the family as it would be between events.
    pub(crate) async fn answer_views(&mut self) {
        while let Ok(event) = self.views.receiver.try_recv() {
            let Some(event) = self.unless_loading(event) else {
                continue;
            };
            if let Event::Request(request, reply) = event {
                Box::pin(self.request(request, reply)).await;
            }
        }
    }

    /// Lets the requests that waited for the stale digests go on.
    pub(crate) fn digests_taken(&mut self) {
        for event in std::mem::take(&mut self.waiting.1) {
            self.send_on(event);
        }
    }

    /// Whether the family is known here but not held, and can be loaded.
    fn unheld(&self, family: FamilyId) -> bool {
        self.families.knows(&family) && !self.families.is_loaded(&family) && self.families.loader().is_some()
    }

    fn load_later(&mut self, family: FamilyId, threads: Vec<ThreadId>) {
        if self.waiting.0.contains_key(&family) {
            return;
        }
        let Some(loader) = self.families.loader() else {
            return;
        };
        let root = self.families.root(&family).map(ToOwned::to_owned);
        let answer: oneshot::Receiver<Fetched> = loader.load_later(family, root, threads);
        let events = self.events.clone();
        tokio::spawn(async move {
            let loaded = answer.await.unwrap_or_else(|_| Fetched {
                result: Err(CoreError::Invalid("the loader stopped".into())),
                size: None,
                histories: Vec::new(),
            });
            let _ = events.send(Event::Loaded {
                family,
                result: Box::new(loaded.result),
                size: loaded.size,
                histories: loaded.histories,
            });
        });
    }

    /// Holds what the loader loaded of the family, unless something loaded
    /// it meanwhile, and lets what waited for it go on.
    pub(crate) fn loaded(
        &mut self,
        family: FamilyId,
        result: Result<Loaded, CoreError>,
        size: Option<u64>,
        histories: Vec<WholeRead>,
    ) {
        match result {
            Ok(loaded) if !self.families.is_loaded(&family) => {
                let root = self.families.root(&family).map(ToOwned::to_owned);
                self.families.hold(family, loaded, root);
                if let Some(size) = size {
                    self.families.set_size(&family, size);
                }
                // Read in the very family now held, so as it is now.
                for whole in histories {
                    self.take_whole(whole);
                }
            }
            Ok(_) => {}
            Err(error) => warn!(%error, %family, "could not load the family; what needs it loads it again"),
        }
        for event in self.waiting.0.remove(&family).unwrap_or_default() {
            self.send_on(event);
        }
    }

    /// The families a request names: those of the threads its strings name
    /// whole, or as `find_thread` takes them, and of the merge requests.
    fn families_named(&self, request: &Request) -> BTreeSet<FamilyId> {
        let value = serde_json::to_value(request).unwrap_or_default();
        let mut strings = Vec::new();
        strings_in(&value, &mut strings);
        let mut named = BTreeSet::new();
        for text in strings.iter().filter(|text| (8..=64).contains(&text.len())) {
            if let Ok(thread) = self.find_thread(text) {
                named.insert(self.threads[&thread].family);
            } else if let Ok((family, _)) = self.find_mr(text) {
                named.insert(family);
            }
        }
        named
    }
}

fn strings_in<'a>(value: &'a serde_json::Value, found: &mut Vec<&'a str>) {
    match value {
        serde_json::Value::String(text) => found.push(text),
        serde_json::Value::Array(values) => values.iter().for_each(|value| strings_in(value, found)),
        serde_json::Value::Object(fields) => fields.values().for_each(|value| strings_in(value, found)),
        _ => {}
    }
}
