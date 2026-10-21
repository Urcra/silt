mod daemon;
mod hook;
mod rescan;

use std::path::Path;

use exn::{Exn, ResultExt};

use crate::BenchError;
use crate::budgets::{CHECKOUT_RESCAN, DAEMON_IDLE, DAEMON_STARTUP, Measured, RECORD_HOOK, RECORD_IMPORT, STORE_SIZE};
use crate::fixtures::Fixtures;

/// A set of budgets measured together, on the same setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    Hook,
    Rescan,
    Daemon,
}

impl Scenario {
    pub const ALL: [Self; 3] = [Self::Hook, Self::Rescan, Self::Daemon];

    pub fn budgets(self) -> &'static [&'static str] {
        match self {
            Self::Hook => &[RECORD_HOOK],
            Self::Rescan => &[CHECKOUT_RESCAN],
            Self::Daemon => &[RECORD_IMPORT, STORE_SIZE, DAEMON_STARTUP, DAEMON_IDLE],
        }
    }

    pub async fn run(self, silt: &Path, fixtures: &Fixtures) -> Result<Vec<(&'static str, Measured)>, Exn<BenchError>> {
        // In /tmp, so the daemon's socket path stays short enough to sit in
        // its data dir.
        let root = tempfile::Builder::new()
            .prefix("silt-bench")
            .tempdir_in("/tmp")
            .or_raise(|| BenchError("cannot make a scratch dir".into()))?;
        match self {
            Self::Hook => Ok(vec![(RECORD_HOOK, hook::measure(root.path()).await?)]),
            Self::Rescan => {
                let repo = fixtures.big_repo().or_raise(|| BenchError("cannot write the 50k-file repository".into()))?;
                Ok(vec![(CHECKOUT_RESCAN, rescan::measure(root.path(), &repo).await?)])
            }
            Self::Daemon => daemon::measure(silt, root.path(), fixtures).await,
        }
    }
}
