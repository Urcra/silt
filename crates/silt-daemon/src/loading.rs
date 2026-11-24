//! Events that name a family the daemon doesn't hold wait while the loader
//! loads it, and the core goes on with others (ADR 0115).

use std::collections::{BTreeSet, HashMap};

use silt_types::FamilyId;
use tokio::sync::oneshot;
use tracing::warn;

use crate::core::{Core, CoreError, Event};
use crate::load::Loaded;
use crate::protocol::Request;

/// The events waiting for each family being loaded, in the order they came,
/// and the requests waiting for the stale digests to be taken.
#[derive(Default)]
pub(crate) struct Waiting(HashMap<FamilyId, Vec<Event>>, Vec<Event>);

impl Core {
    /// The event, if it can be handled now; else it waits for the families
    /// it names to load.
    pub(crate) fn unless_loading(&mut self, event: Event) -> Option<Event> {
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
            // A page of a history kept as it is now needs no family loaded.
            Event::Request(Request::HistoryRead { thread, cut: true, .. }, _)
                if self.find_thread(thread).is_ok_and(|thread| self.history_kept_now(thread)) =>
            {
                return Some(event);
            }
            Event::Request(request, _) => self.families_named(request),
            Event::Changed { checkout, .. } => self.checkouts.get(checkout).map(|held| held.family).into_iter().collect(),
            _ => return Some(event),
        };
        let Some(family) = named.into_iter().find(|family| self.waiting.0.contains_key(family) || self.unheld(*family)) else {
            return Some(event);
        };
        self.load_later(family);
        self.waiting.0.entry(family).or_default().push(event);
        None
    }

    /// Lets the requests that waited for the stale digests go on.
    pub(crate) fn digests_taken(&mut self) {
        for event in std::mem::take(&mut self.waiting.1) {
            let _ = self.events.send(event);
        }
    }

    /// Whether the family is known here but not held, and can be loaded.
    fn unheld(&self, family: FamilyId) -> bool {
        self.families.knows(&family) && !self.families.is_loaded(&family) && self.families.loader().is_some()
    }

    fn load_later(&mut self, family: FamilyId) {
        if self.waiting.0.contains_key(&family) {
            return;
        }
        let Some(loader) = self.families.loader() else {
            return;
        };
        let answer: oneshot::Receiver<Result<Loaded, CoreError>> =
            loader.load_later(family, self.families.root(&family).map(ToOwned::to_owned));
        let events = self.events.clone();
        tokio::spawn(async move {
            let result = answer.await.unwrap_or_else(|_| Err(CoreError::Invalid("the loader stopped".into())));
            let _ = events.send(Event::Loaded { family, result: Box::new(result) });
        });
    }

    /// Holds what the loader loaded of the family, unless something loaded
    /// it meanwhile, and lets what waited for it go on.
    pub(crate) fn loaded(&mut self, family: FamilyId, result: Result<Loaded, CoreError>) {
        match result {
            Ok(loaded) if !self.families.is_loaded(&family) => {
                let root = self.families.root(&family).map(ToOwned::to_owned);
                self.families.hold(family, loaded, root);
            }
            Ok(_) => {}
            Err(error) => warn!(%error, %family, "could not load the family; what needs it loads it again"),
        }
        for event in self.waiting.0.remove(&family).unwrap_or_default() {
            let _ = self.events.send(event);
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
