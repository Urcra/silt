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
const TICK: Duration = DEBOUNCE;
const POLL_INTERVAL: Duration = Duration::from_millis(500);

type OnChange = Box<dyn Fn(Vec<PathBuf>) + Send>;

/// The checkouts watched, each by its root, for the handler to route the
/// paths it hears about. Checkouts of several threads may share a root
/// (ADR 0089), so each route has its own number.
type Routes = Arc<Mutex<Vec<Route>>>;

struct Route {
    id: u64,
    root: PathBuf,
    mode: WatchMode,
    on_change: OnChange,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WatchMode {
    Native,
    Polled,
    Rescan,
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
    mode: WatchMode,
}

impl Watched {
    pub(crate) fn needs_rescan(&self) -> bool {
        self.mode == WatchMode::Rescan
    }
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
        let (already, mode) = {
            let mut routes = lock(&self.routes);
            let already = routes.iter().find(|route| route.root == root).map(|route| route.mode);
            let mode = already.unwrap_or_else(|| if needs_polling(root) { WatchMode::Polled } else { WatchMode::Native });
            routes.push(Route { id, root: root.to_owned(), mode, on_change: Box::new(on_change) });
            (already.is_some(), mode)
        };
        let mut watched = Watched { watchers: Arc::clone(self), id, root: root.to_owned(), mode };
        if already {
            return Ok(watched);
        }
        let config = notify::Config::default().with_follow_symlinks(false);
        let debouncing = |config| {
            notify_debouncer_full::Config::default().with_timeout(DEBOUNCE).with_tick_rate(TICK).with_notify_config(config)
        };
        let result = if mode == WatchMode::Polled {
            let config = config.with_poll_interval(POLL_INTERVAL).with_compare_contents(true);
            started(&mut lock(&self.polled), || new_debouncer_opt(debouncing(config), handler(&self.routes), NoCache))
                .and_then(|debouncer| debouncer.watch(root, RecursiveMode::Recursive))
        } else {
            // inotify pairs renames itself; a file-id cache would walk the
            // linked trees anyway.
            started(&mut lock(&self.native), || new_debouncer_opt(debouncing(config), handler(&self.routes), NoCache))
                .and_then(|debouncer| debouncer.watch(root, RecursiveMode::Recursive))
        };
        if result.as_ref().is_err_and(|error| matches!(error.kind, notify::ErrorKind::MaxFilesWatch)) {
            self.rescan_instead(&mut watched);
            tracing::warn!("file watch limit reached for {}; checking its tracked files periodically", root.display());
            return Ok(watched);
        }
        // Dropped on failure, which takes the route back out.
        result.map(|()| watched)
    }

    fn rescan_instead(&self, watched: &mut Watched) {
        // A recursive watch may have installed thousands of watches before
        // failing. Release them before the next checkout starts its watch.
        if let Some(native) = lock(&self.native).as_mut() {
            let _ = native.unwatch(&watched.root);
        }
        watched.mode = WatchMode::Rescan;
        for route in lock(&self.routes).iter_mut().filter(|route| route.root == watched.root) {
            route.mode = WatchMode::Rescan;
        }
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
        match self.mode {
            WatchMode::Polled => {
                let _ = lock(&self.watchers.polled).as_mut().map(|debouncer| debouncer.unwatch(&self.root));
            }
            WatchMode::Native => {
                let _ = lock(&self.watchers.native).as_mut().map(|debouncer| debouncer.unwatch(&self.root));
            }
            WatchMode::Rescan => {}
        }
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
    fn rescanning_releases_partial_watches_and_shared_checkouts_keep_the_fallback() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("src")).expect("mkdir");
        let watchers = Watchers::new();
        let mut watched = watchers.watch(dir.path(), |_| {}).expect("watches");
        watchers.rescan_instead(&mut watched);
        assert!(watched.needs_rescan());
        assert!(lock(&watchers.native).as_ref().expect("started").watched_paths().expect("paths").is_empty());

        let beside = watchers.watch(dir.path(), |_| {}).expect("shares the fallback");
        assert!(beside.needs_rescan());
        drop(watched);
        assert!(beside.needs_rescan());
        drop(beside);
        let native = watchers.watch(dir.path(), |_| {}).expect("watches again after all routes leave");
        assert!(!native.needs_rescan());
    }

    #[tokio::test]
    async fn periodic_capture_records_edits_and_skips_ignored_output_and_inactive_clones() {
        use crate::core::{Core, Event};
        use silt_store::{BlobStore, Store};
        use silt_types::ActorId;

        let home = tempfile::tempdir().expect("home");
        let dir = tempfile::tempdir().expect("checkout");
        assert!(std::process::Command::new("git").args(["init", "-q"]).current_dir(dir.path()).status().expect("git").success());
        let root = silt_os::canonicalize(dir.path()).expect("canonical");
        std::fs::write(root.join(".gitignore"), "/out/\n").expect("ignore output");
        std::fs::write(root.join("edited.txt"), "before").expect("write");
        std::fs::write(root.join("removed.txt"), "before").expect("write");
        let store = Store::open(home.path()).await.expect("store");
        let mut core = Core::visiting(store, ActorId(1), home.path()).await.expect("core");
        core.start_session(&root, "claude", None, false, false).await.expect("session");
        core.pending.clear();
        let id = *core.checkouts.keys().next().expect("checkout");
        let watchers = Watchers::new();
        let mut watched = watchers.watch(&root, |_| {}).expect("watches");
        watchers.rescan_instead(&mut watched);
        core.checkouts.get_mut(&id).expect("checkout")._watched = Some(watched);
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        core.events = sender;

        core.tick().await;
        assert!(receiver.try_recv().is_err(), "an inactive personal clone stays left alone");
        core.checkouts.get_mut(&id).expect("checkout").record.managed = true;
        std::fs::write(root.join("edited.txt"), "after editing").expect("edit");
        std::fs::write(root.join("added.txt"), "added").expect("add");
        std::fs::remove_file(root.join("removed.txt")).expect("remove");
        std::fs::create_dir(root.join("out")).expect("output");
        std::fs::write(root.join("out/ignored.txt"), "build output").expect("ignored output");

        core.tick().await;
        let Event::Changed { checkout, paths, noticed, settled } = receiver.try_recv().expect("periodic change") else {
            panic!("expected a checkout rescan");
        };
        core.changed(checkout, paths, noticed, settled).await;
        let entries = core.checkouts[&id].bridge.entries();
        assert_eq!(entries["edited.txt"].hash, BlobStore::hash(b"after editing"));
        assert_eq!(entries["added.txt"].hash, BlobStore::hash(b"added"));
        assert!(!entries.contains_key("removed.txt"));
        assert!(!entries.contains_key("out/ignored.txt"));
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
        // FSEvents can still report the write from before the watch began.
        while receiver.recv_timeout(DEBOUNCE * 2).is_ok() {}
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
