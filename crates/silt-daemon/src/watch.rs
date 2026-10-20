use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::{PollWatcher, RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, NoCache, new_debouncer_opt};

const DEBOUNCE: Duration = Duration::from_millis(100);
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Watches a checkout and reports paths that may have changed. Event kinds
/// are not trusted: every reported path is re-read and compared.
#[expect(dead_code, reason = "the debouncer is only held so the watch keeps running")]
pub enum Watcher {
    Native(Debouncer<RecommendedWatcher, NoCache>),
    Poll(Debouncer<PollWatcher, NoCache>),
}

/// Filesystems where inotify delivers nothing or can't be relied on, such as
/// WSL's `/mnt/c` (9p), FUSE, SMB and NFS (docs/research/probes.md).
fn needs_polling(root: &Path) -> bool {
    const V9FS: i64 = 0x0102_1997;
    const FUSE: i64 = 0x6573_5546;
    const CIFS: i64 = 0xFF53_4D42;
    const SMB2: i64 = 0xFE53_4D42;
    const NFS: i64 = 0x6969;
    rustix::fs::statfs(root).is_ok_and(|stats| {
        #[allow(
            clippy::unnecessary_cast,
            reason = "`f_type` is i64 here but a different integer type on other platforms, so #[expect] would not hold everywhere"
        )]
        let kind = stats.f_type as i64;
        [V9FS, FUSE, CIFS, SMB2, NFS].contains(&kind)
    })
}

/// Starts watching `root`; `on_change` gets batches of absolute paths, or the
/// root itself when events were lost and everything must be rescanned.
pub fn watch(root: &Path, on_change: impl Fn(Vec<PathBuf>) + Send + 'static) -> notify::Result<Watcher> {
    let rescan_root = root.to_owned();
    let handler = move |result: DebounceEventResult| match result {
        Ok(events) => {
            let mut paths: Vec<PathBuf> = events
                .iter()
                .flat_map(|event| {
                    if event.need_rescan() {
                        vec![rescan_root.clone()]
                    } else {
                        event.paths.clone()
                    }
                })
                .collect();
            paths.sort();
            paths.dedup();
            if !paths.is_empty() {
                on_change(paths);
            }
        }
        Err(_) => on_change(vec![rescan_root.clone()]),
    };
    // Linked paths (`.agents/linked`) are symlinks to shared trees such as
    // `node_modules`, which are ignored and too big to watch.
    let config = notify::Config::default().with_follow_symlinks(false);
    if needs_polling(root) {
        let config = config.with_poll_interval(POLL_INTERVAL).with_compare_contents(true);
        let mut debouncer = new_debouncer_opt::<_, PollWatcher, NoCache>(DEBOUNCE, None, handler, NoCache, config)?;
        debouncer.watch(root, RecursiveMode::Recursive)?;
        Ok(Watcher::Poll(debouncer))
    } else {
        // inotify pairs renames itself; a file-id cache would walk the
        // linked trees anyway.
        let mut debouncer = new_debouncer_opt::<_, RecommendedWatcher, NoCache>(DEBOUNCE, None, handler, NoCache, config)?;
        debouncer.watch(root, RecursiveMode::Recursive)?;
        Ok(Watcher::Native(debouncer))
    }
}
