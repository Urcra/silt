//! Which build this is, whether a newer one is out, and the update that
//! installs it (ADR 0100, ADR 0129): a dev build pulls its clone's `main` and
//! runs `just update` there; a nightly one learns of newer nightlies, and a
//! stable one of newer releases.

pub mod check;
pub mod info;
pub mod run;
pub mod shell;

use serde::{Deserialize, Serialize};

pub use check::{Availability, NewCommit, OffReason};
pub use info::{BuildInfo, BuildView, Channel, Commit, Source, VERSION};
pub use run::{Outcome, Phase, RunState, RunView, StepStatus, StepView};

/// Everything Settings › Updates shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UpdateView {
    pub build: BuildView,
    /// The last check's answer, until the first check ends.
    pub availability: Option<Availability>,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub checked_ms: Option<i64>,
    pub checking: bool,
    /// The update running or last run.
    pub run: Option<RunView>,
    /// The run's last lines of output.
    pub output: Vec<String>,
    /// Where all of it is.
    pub output_path: String,
    /// Whether the desktop app showing this needs a relaunch to run what
    /// the update brings, or has brought.
    pub relaunch: bool,
}
