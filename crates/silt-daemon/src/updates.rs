//! Updating this machine's Silt from the web UI (ADR 0100, ADR 0129): whether
//! a newer build is out, checked at start and every few hours, and, for a dev
//! build, the detached `silt update run` that installs it, whose state this
//! reads back.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use silt_update::check::{self, Availability, OffReason};
use silt_update::run;
use silt_update::{BuildInfo, Channel, Commit, UpdateView};
use tracing::warn;

use crate::core::{CoreError, now_ms};

/// How often a check runs by itself.
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
const RELEASE_WAIT: Duration = Duration::from_secs(20);
/// The lines of output the page shows.
const OUTPUT_LINES: usize = 200;

#[derive(Default)]
struct Checked {
    availability: Option<Availability>,
    checked_ms: Option<i64>,
    at: Option<Instant>,
    checking: bool,
}

pub(crate) struct Updates {
    data_dir: PathBuf,
    build: BuildInfo,
    client: reqwest::Client,
    checked: Mutex<Checked>,
    /// Whether the desktop app built from one commit differs from another's.
    differs: Mutex<HashMap<(Commit, Commit), bool>>,
}

impl Updates {
    pub(crate) fn new(data_dir: &Path) -> Self {
        let client = release_client();
        Self {
            data_dir: data_dir.to_owned(),
            build: BuildInfo::current(),
            client,
            checked: Mutex::default(),
            differs: Mutex::default(),
        }
    }

    /// Starts a check when none ran for a while, unless this machine is
    /// offline or `SILT_UPDATE_CHECK` is `never`, which leave checks to
    /// Check now.
    pub(crate) fn check_if_due(self: &Arc<Self>) {
        if !crate::network::allowed(&self.data_dir).update_checks {
            return;
        }
        let due = {
            let checked = self.checked.lock().expect("lock");
            !checked.checking && checked.at.is_none_or(|at| at.elapsed() >= CHECK_EVERY)
        };
        if due {
            self.start_check();
        }
    }

    /// Starts a check now, unless one runs.
    pub(crate) fn start_check(self: &Arc<Self>) {
        {
            let mut checked = self.checked.lock().expect("lock");
            if checked.checking {
                return;
            }
            checked.checking = true;
        }
        let here = self.clone();
        tokio::spawn(async move {
            let availability = here.check().await;
            if let Availability::Failed { message } = &availability {
                warn!(%message, "could not check for an update");
            }
            let mut checked = here.checked.lock().expect("lock");
            checked.availability = Some(availability);
            checked.checked_ms = Some(now_ms());
            checked.at = Some(Instant::now());
            checked.checking = false;
        });
    }

    async fn check(&self) -> Availability {
        if self.build.debug {
            return Availability::Off { reason: OffReason::DebugBuild };
        }
        match self.build.channel {
            Channel::Dev => {
                let build = self.build.clone();
                tokio::task::spawn_blocking(move || check::dev(&build, true))
                    .await
                    .unwrap_or_else(|error| Availability::Failed { message: error.to_string() })
            }
            Channel::Nightly | Channel::Stable => released(&self.client, &self.build).await,
        }
    }

    /// What Settings › Updates shows, for the desktop app built from
    /// `desktop` when the page is in one.
    pub(crate) async fn view(self: &Arc<Self>, desktop: Option<Commit>) -> UpdateView {
        let (availability, checked_ms, checking) = {
            let checked = self.checked.lock().expect("lock");
            (checked.availability.clone(), checked.checked_ms, checked.checking)
        };
        let data_dir = self.data_dir.clone();
        let here = self.clone();
        let target = match &availability {
            Some(Availability::Behind { target, .. }) => Some(target.clone()),
            _ => self.build.commit.clone(),
        };
        let (run, output, relaunch) = tokio::task::spawn_blocking(move || {
            let state = run::read_state(&data_dir);
            let run = state.as_ref().map(|state| state.view(now_ms(), !state.in_flight() || run::runner_alive(state)));
            let output = std::fs::read_to_string(run::output_path(&data_dir))
                .map(|text| run::tail(&text, OUTPUT_LINES))
                .unwrap_or_default();
            let relaunch = match (desktop, target) {
                (Some(desktop), Some(target)) => here.desktop_differs(desktop, target),
                _ => false,
            };
            (run, output, relaunch)
        })
        .await
        .unwrap_or_default();
        UpdateView {
            build: self.build.view(),
            availability,
            checked_ms,
            checking,
            run,
            output,
            output_path: run::output_path(&self.data_dir).display().to_string(),
            relaunch,
        }
    }

    fn desktop_differs(&self, desktop: Commit, target: Commit) -> bool {
        let Some(source) = &self.build.source else {
            return false;
        };
        let key = (desktop, target);
        if let Some(known) = self.differs.lock().expect("lock").get(&key) {
            return *known;
        }
        let differs = check::desktop_differs(source, &key.0, &key.1);
        self.differs.lock().expect("lock").insert(key, differs);
        differs
    }

    /// Whether an update is running, waiting for agents among them.
    pub(crate) fn waiting(&self) -> bool {
        run::read_state(&self.data_dir).is_some_and(|state| matches!(state.outcome, silt_update::Outcome::Waiting { .. }))
    }

    /// Starts `silt update run` detached, so it outlives this daemon's
    /// restart part-way and at its end.
    pub(crate) fn start(&self, exe: Option<&Path>) -> Result<(), CoreError> {
        if let Err(reason) = check::updatable(&self.build) {
            return Err(CoreError::Invalid(format!("no update: {}", reason.explain())));
        }
        match self.build.channel {
            Channel::Dev => {}
            Channel::Nightly => {
                return Err(CoreError::Invalid("a nightly build updates from the nightly release's downloads".to_owned()));
            }
            Channel::Stable => return Err(CoreError::Invalid("a stable build updates with its installer".to_owned())),
        }
        if run::read_state(&self.data_dir).is_some_and(|state| state.in_flight() && run::runner_alive(&state)) {
            return Err(CoreError::Invalid("an update is already running".to_owned()));
        }
        let exe = exe.ok_or_else(|| CoreError::Invalid("the daemon doesn't know its own program".to_owned()))?;
        silt_os::create_private_dir(&run::dir(&self.data_dir))?;
        let log = std::fs::OpenOptions::new().create(true).append(true).open(run::dir(&self.data_dir).join("runner.log"))?;
        let log_err = log.try_clone()?;
        silt_os::spawn_detached(
            Command::new(exe).args(["update", "run"]).current_dir("/").stdin(Stdio::null()).stdout(log).stderr(log_err),
        )?;
        Ok(())
    }
}

/// A client for asking GitHub of releases.
pub fn release_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(RELEASE_WAIT)
        .user_agent(concat!("silt/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

/// Whether a release past a nightly or stable build is out: the `nightly`
/// release for a nightly build, the latest release for a stable one.
pub async fn released(client: &reqwest::Client, build: &BuildInfo) -> Availability {
    let api = std::env::var("SILT_UPDATE_API").ok().filter(|api| !api.is_empty()).unwrap_or_else(|| check::API.to_owned());
    let url = match (build.channel, &build.commit) {
        (Channel::Nightly, Some(built)) => check::nightly_url(&api, built),
        (Channel::Nightly, None) => return Availability::Off { reason: OffReason::NoClone },
        _ => check::releases_url(&api),
    };
    let asked = client.get(url).header("Accept", "application/vnd.github+json").send().await;
    let answer = match asked.and_then(reqwest::Response::error_for_status) {
        Ok(answer) => answer.json::<serde_json::Value>().await,
        Err(error) => Err(error),
    };
    match (answer, build.channel) {
        (Ok(answer), Channel::Nightly) => check::nightly(&answer),
        (Ok(answer), _) => check::release(build, &answer),
        (Err(error), _) => Availability::Failed { message: format!("could not reach the releases: {error}") },
    }
}
