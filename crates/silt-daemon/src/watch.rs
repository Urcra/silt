//! Watches the daemon's checkouts and reports paths that may have changed.
//! Event kinds are not trusted: every reported path is re-read and compared.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use notify::event::{AccessKind, AccessMode};
use notify::{EventKind, PollWatcher, RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, DebouncedEvent, Debouncer, NoCache, new_debouncer_opt};

const DEBOUNCE: Duration = Duration::from_millis(100);
/// How often the debouncer looks for events that have settled; notify's
/// default is a quarter of `DEBOUNCE`.
const TICK: Option<Duration> = Some(DEBOUNCE);
const POLL_INTERVAL: Duration = Duration::from_millis(500);

type OnChange = Box<dyn Fn(Vec<PathBuf>) + Send>;

/// The checkouts watched, each by its root, for the handler to route the
/// paths it hears about. Checkouts of several threads may share a root
/// (ADR 0086), so each route has its own number.
type Routes = Arc<Mutex<Vec<Route>>>;

struct Route {
    id: u64,
    root: PathBuf,
    on_change: OnChange,
}

/// One watch for all of the daemon's checkouts. A debouncer each kept a
/// thread waking ten times a second for every checkout, which was most of
/// what an idle daemon spent.
pub struct Watchers {
    routes: Routes,
    next: std::sync::atomic::AtomicU64,
    native: Mutex<Option<Debouncer<RecommendedWatcher, NoCache>>>,
    polled: Mutex<Option<Debouncer<PollWatcher, NoCache>>>,
}

/// One checkout being watched, until this is dropped.
pub struct Watched {
    watchers: Arc<Watchers>,
    id: u64,
    root: PathBuf,
    polled: bool,
}

impl Watchers {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { routes: Arc::default(), next: Default::default(), native: Mutex::default(), polled: Mutex::default() })
    }

    /// Starts watching `root`; `on_change` gets batches of absolute paths,
    /// or the root itself when events were lost and everything must be
    /// rescanned.
    pub fn watch(self: &Arc<Self>, root: &Path, on_change: impl Fn(Vec<PathBuf>) + Send + 'static) -> notify::Result<Watched> {
        let id = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let already = {
            let mut routes = lock(&self.routes);
            let already = routes.iter().any(|route| route.root == root);
            routes.push(Route { id, root: root.to_owned(), on_change: Box::new(on_change) });
            already
        };
        let polled = needs_polling(root);
        let watched = Watched { watchers: Arc::clone(self), id, root: root.to_owned(), polled };
        if already {
            return Ok(watched);
        }
        let config = notify::Config::default().with_follow_symlinks(false);
        let result = if polled {
            let config = config.with_poll_interval(POLL_INTERVAL).with_compare_contents(true);
            started(&mut lock(&self.polled), || new_debouncer_opt(DEBOUNCE, TICK, handler(&self.routes), NoCache, config))
                .and_then(|debouncer| debouncer.watch(root, RecursiveMode::Recursive))
        } else {
            // inotify pairs renames itself; a file-id cache would walk the
            // linked trees anyway.
            started(&mut lock(&self.native), || new_debouncer_opt(DEBOUNCE, TICK, handler(&self.routes), NoCache, config))
                .and_then(|debouncer| debouncer.watch(root, RecursiveMode::Recursive))
        };
        // Dropped on failure, which takes the route back out.
        result.map(|()| watched)
    }
}

impl Drop for Watched {
    fn drop(&mut self) {
        let shared = {
            let mut routes = lock(&self.watchers.routes);
            routes.retain(|route| route.id != self.id);
            routes.iter().any(|route| route.root == self.root)
        };
        if shared {
            return;
        }
        let _ = if self.polled {
            lock(&self.watchers.polled).as_mut().map(|debouncer| debouncer.unwatch(&self.root))
        } else {
            lock(&self.watchers.native).as_mut().map(|debouncer| debouncer.unwatch(&self.root))
        };
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The debouncer in `slot`, started with `start` the first time.
fn started<W: notify::Watcher>(
    slot: &mut Option<Debouncer<W, NoCache>>,
    start: impl FnOnce() -> notify::Result<Debouncer<W, NoCache>>,
) -> notify::Result<&mut Debouncer<W, NoCache>> {
    if slot.is_none() {
        *slot = Some(start()?);
    }
    Ok(slot.as_mut().expect("started above"))
}

fn handler(routes: &Routes) -> impl FnMut(DebounceEventResult) + Send + 'static {
    let routes = Arc::clone(routes);
    move |result| {
        let routes = lock(&routes);
        let roots: Vec<&Path> = routes.iter().map(|route| route.root.as_path()).collect();
        for (index, paths) in route(&roots, result.as_deref().ok()) {
            (routes[index].on_change)(paths);
        }
    }
}

/// The paths that may have changed, by the index of the root they're under,
/// the deepest when roots nest, and to every root of that path. Lost
/// events, or none at all (`None`), rescan every root.
fn route(roots: &[&Path], events: Option<&[DebouncedEvent]>) -> Vec<(usize, Vec<PathBuf>)> {
    let everything = || roots.iter().enumerate().map(|(index, root)| (index, vec![root.to_path_buf()])).collect();
    let Some(events) = events else {
        return everything();
    };
    if events.iter().any(|event| event.need_rescan()) {
        return everything();
    }
    let mut routed: Vec<Vec<PathBuf>> = vec![Vec::new(); roots.len()];
    for path in events.iter().filter(|event| may_change(&event.kind)).flat_map(|event| &event.paths) {
        let deepest = roots.iter().filter(|root| path.starts_with(root)).max_by_key(|root| root.as_os_str().len());
        for (index, _) in roots.iter().enumerate().filter(|(_, root)| Some(*root) == deepest) {
            routed[index].push(path.clone());
        }
    }
    routed
        .into_iter()
        .enumerate()
        .filter(|(_, paths)| !paths.is_empty())
        .map(|(index, mut paths)| {
            paths.sort();
            paths.dedup();
            (index, paths)
        })
        .collect()
}

/// Filesystems where events can't be relied on (docs/research/probes.md).
fn needs_polling(root: &Path) -> bool {
    silt_os::needs_polling(root)
}

/// Whether an event can mean the path changed. Opening and reading don't,
/// and must be skipped: rescanning reads every file, which would otherwise
/// report every file again, forever.
fn may_change(kind: &EventKind) -> bool {
    match kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        EventKind::Access(_) => false,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use notify::Event;
    use notify::event::{CreateKind, ModifyKind};

    use super::*;

    #[test]
    fn reading_is_not_changing() {
        assert!(!may_change(&EventKind::Access(AccessKind::Open(AccessMode::Any))));
        assert!(!may_change(&EventKind::Access(AccessKind::Close(AccessMode::Read))));
        assert!(may_change(&EventKind::Access(AccessKind::Close(AccessMode::Write))));
        assert!(may_change(&EventKind::Create(CreateKind::File)));
        assert!(may_change(&EventKind::Modify(ModifyKind::Any)));
    }

    fn event(kind: EventKind, path: &str) -> DebouncedEvent {
        DebouncedEvent::new(Event::new(kind).add_path(PathBuf::from(path)), Instant::now())
    }

    #[test]
    fn a_path_goes_to_the_deepest_checkout_it_is_in() {
        let roots = [Path::new("/repo"), Path::new("/repo/.silt/checkouts/a"), Path::new("/other")];
        let changed = EventKind::Modify(ModifyKind::Any);
        let events = [
            event(changed, "/repo/src/main.rs"),
            event(changed, "/repo/.silt/checkouts/a/lib.rs"),
            event(changed, "/repo/src/main.rs"),
            event(EventKind::Access(AccessKind::Open(AccessMode::Any)), "/other/read.rs"),
            event(changed, "/elsewhere/x"),
        ];
        assert_eq!(
            route(&roots, Some(&events)),
            [(0, vec![PathBuf::from("/repo/src/main.rs")]), (1, vec![PathBuf::from("/repo/.silt/checkouts/a/lib.rs")])]
        );
    }

    #[test]
    fn a_path_goes_to_every_checkout_sharing_its_root() {
        let roots = [Path::new("/repo"), Path::new("/repo"), Path::new("/repo/.silt/checkouts/a")];
        let events = [event(EventKind::Modify(ModifyKind::Any), "/repo/src/main.rs")];
        let main = vec![PathBuf::from("/repo/src/main.rs")];
        assert_eq!(route(&roots, Some(&events)), [(0, main.clone()), (1, main)]);
    }

    #[test]
    fn lost_events_rescan_every_checkout() {
        let roots = [Path::new("/repo"), Path::new("/other")];
        let everything = [(0, vec![PathBuf::from("/repo")]), (1, vec![PathBuf::from("/other")])];
        assert_eq!(route(&roots, None), everything);
        let mut lost = event(EventKind::Other, "/repo");
        lost.event = lost.event.clone().set_flag(notify::event::Flag::Rescan);
        assert_eq!(route(&roots, Some(&[lost])), everything);
    }

    #[test]
    fn rescanning_a_checkout_reports_nothing() {
        let (dir, other) = (tempfile::tempdir().expect("tempdir"), tempfile::tempdir().expect("tempdir"));
        std::fs::write(dir.path().join("a.txt"), "a").expect("write");
        let watchers = Watchers::new();
        let (sender, receiver) = std::sync::mpsc::channel();
        let _watched = watchers
            .watch(dir.path(), move |paths| {
                let _ = sender.send(paths);
            })
            .expect("watches");
        let (other_sender, other_receiver) = std::sync::mpsc::channel();
        let watched_other = watchers
            .watch(other.path(), move |paths| {
                let _ = other_sender.send(paths);
            })
            .expect("watches");
        std::thread::sleep(Duration::from_millis(200));
        for entry in std::fs::read_dir(dir.path()).expect("lists") {
            std::fs::read(entry.expect("entry").path()).expect("reads");
        }
        assert!(receiver.recv_timeout(DEBOUNCE * 5).is_err(), "reading files is not a change");
        std::fs::write(dir.path().join("a.txt"), "b").expect("write");
        assert!(receiver.recv_timeout(Duration::from_secs(5)).is_ok(), "writing is");
        assert!(other_receiver.try_recv().is_err(), "the other checkout heard nothing");
        drop(watched_other);
        std::fs::write(other.path().join("b.txt"), "b").expect("write");
        assert!(other_receiver.recv_timeout(DEBOUNCE * 5).is_err(), "a checkout no longer watched hears nothing");

        let (beside, beside_receiver) = std::sync::mpsc::channel();
        let watched_beside = watchers
            .watch(dir.path(), move |paths| {
                let _ = beside.send(paths);
            })
            .expect("watches the same root again");
        std::fs::write(dir.path().join("a.txt"), "c").expect("write");
        assert!(receiver.recv_timeout(Duration::from_secs(5)).is_ok(), "the first checkout hears it");
        assert!(beside_receiver.recv_timeout(Duration::from_secs(5)).is_ok(), "and so does the one beside it");
        drop(watched_beside);
        while receiver.try_recv().is_ok() {}
        std::fs::write(dir.path().join("a.txt"), "d").expect("write");
        assert!(receiver.recv_timeout(Duration::from_secs(5)).is_ok(), "the root stays watched for the first");
    }
}
