mod daemon;
mod hook;
mod pages;
mod rescan;
mod sync;

use std::path::Path;

use exn::{Exn, ResultExt};

use crate::BenchError;
use crate::budgets::{
    APP_HISTORY, APP_THREADS, CHECKOUT_RESCAN, DAEMON_IDLE, DAEMON_READY, DAEMON_STARTUP, DOC_REPLAY, IMPORT_SESSION, Measured,
    PAGE_FOLD, RECORD_HOOK, RECORD_HOOK_DAEMONLESS, RECORD_IMPORT, SHEET_EVAL, STORE_SIZE, SYNC_PULL, SYNC_PUSH,
};
use crate::fixtures::Fixtures;

/// A set of budgets measured together, on the same setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    Hook,
    Rescan,
    Daemon,
    Sync,
    Pages,
}

impl Scenario {
    pub const ALL: [Self; 5] = [Self::Hook, Self::Rescan, Self::Daemon, Self::Sync, Self::Pages];

    pub fn budgets(self) -> &'static [&'static str] {
        match self {
            Self::Hook => &[RECORD_HOOK, RECORD_HOOK_DAEMONLESS],
            Self::Rescan => &[CHECKOUT_RESCAN],
            Self::Daemon => &[RECORD_IMPORT, STORE_SIZE, IMPORT_SESSION, DAEMON_STARTUP, DAEMON_READY, DAEMON_IDLE],
            Self::Sync => &[SYNC_PUSH, SYNC_PULL, APP_THREADS, APP_HISTORY],
            Self::Pages => &[PAGE_FOLD, DOC_REPLAY, SHEET_EVAL],
        }
    }

    /// Measures this scenario's budgets: at least those `wanted` names.
    pub async fn run(
        self,
        silt: &Path,
        fixtures: &Fixtures,
        wanted: &[&str],
        keep: bool,
    ) -> Result<Vec<(&'static str, Measured)>, Exn<BenchError>> {
        // In /tmp, so the daemon's socket path stays short enough to sit in
        // its data dir.
        let root = tempfile::Builder::new()
            .prefix("silt-bench")
            .tempdir_in("/tmp")
            .or_raise(|| BenchError("cannot make a scratch dir".into()))?;
        let measured = match self {
            Self::Hook => hook::measure(silt, root.path()).await,
            Self::Rescan => {
                let repo = fixtures.big_repo().or_raise(|| BenchError("cannot write the 50k-file repository".into()))?;
                rescan::measure(root.path(), &repo).await.map(|time| vec![(CHECKOUT_RESCAN, time)])
            }
            Self::Daemon => daemon::measure(silt, root.path(), fixtures, wanted).await,
            Self::Sync => sync::measure(silt, root.path(), fixtures).await,
            Self::Pages => Ok(pages::measure()),
        };
        if keep {
            eprintln!("silt-bench: kept {self:?} in {}", root.keep().display());
        }
        measured
    }
}
