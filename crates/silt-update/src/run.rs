//! An update's run: its steps, as named from the recipe lines `just`
//! echoes, how far along it is, and the state the runner writes for the
//! daemon to show (ADR 0098).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::info::Commit;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Fetch,
    WebDependencies,
    WebBuild,
    Cli,
    Desktop,
    Voice,
    Windows,
    Restart,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Fetch => "Fast-forward main",
            Self::WebDependencies => "Web dependencies",
            Self::WebBuild => "Build the web UI",
            Self::Cli => "Build silt",
            Self::Desktop => "Build the desktop app",
            Self::Voice => "Speech helper",
            Self::Windows => "Build for Windows",
            Self::Restart => "Restart the daemon",
        }
    }

    /// The step a recipe line `just` echoes starts, as the justfile writes
    /// them.
    pub fn of_line(line: &str) -> Option<Self> {
        let line = line.trim();
        let installs = |krate: &str| line.starts_with("cargo install") && line.contains(krate);
        if line.starts_with("pnpm -C web install") {
            Some(Self::WebDependencies)
        } else if line.starts_with("pnpm -C web build") {
            Some(Self::WebBuild)
        } else if installs("crates/silt-cli") {
            Some(Self::Cli)
        } else if installs("crates/silt-desktop") {
            Some(Self::Desktop)
        } else if installs("crates/silt-voice") {
            Some(Self::Voice)
        } else if line.starts_with("scripts/install-windows.sh") {
            Some(Self::Windows)
        } else {
            None
        }
    }

    /// About how long it takes, before a run has said.
    fn guess_ms(self) -> i64 {
        match self {
            Self::Fetch | Self::Restart => 3_000,
            Self::WebDependencies => 15_000,
            Self::WebBuild => 40_000,
            Self::Cli => 240_000,
            Self::Desktop => 120_000,
            Self::Voice => 30_000,
            Self::Windows => 360_000,
        }
    }

    fn compiles(self) -> bool {
        matches!(self, Self::Cli | Self::Desktop | Self::Voice | Self::Windows)
    }
}

/// The steps an update takes: the justfile's `update` recipe installs the
/// CLI, the desktop app where one is installed, and the speech helper;
/// from WSL, and for Windows, it builds for Windows too.
pub fn expected(desktop: bool, windows: bool) -> Vec<Phase> {
    let mut phases = vec![Phase::Fetch, Phase::WebDependencies, Phase::WebBuild, Phase::Cli];
    if desktop && !windows {
        phases.push(Phase::Desktop);
    }
    phases.push(Phase::Voice);
    if windows {
        phases.push(Phase::Windows);
    }
    phases.push(Phase::Restart);
    phases
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub phase: Phase,
    pub started_ms: i64,
    #[serde(default)]
    pub ended_ms: Option<i64>,
    /// The crates cargo compiled in it.
    #[serde(default)]
    pub compiled: u32,
}

/// How long a step took the last time it finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Estimate {
    pub phase: Phase,
    pub took_ms: i64,
    pub compiled: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "RunOutcome"))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    Running,
    /// Installed, with the daemon's restart waiting for agents mid-turn.
    Waiting {
        agents: u32,
    },
    Done,
    Failed {
        message: String,
    },
}

/// What the runner writes to `state.json` as it goes. A daemon reads what a
/// runner of another build wrote, so fields are only ever added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunState {
    pub pid: u32,
    pub started_ms: i64,
    #[serde(default)]
    pub ended_ms: Option<i64>,
    #[serde(default)]
    pub from: Option<Commit>,
    #[serde(default)]
    pub to: Option<Commit>,
    #[serde(default)]
    pub expected: Vec<Phase>,
    #[serde(default)]
    pub steps: Vec<Step>,
    pub outcome: Outcome,
    #[serde(default)]
    pub estimates: Vec<Estimate>,
}

impl RunState {
    /// A run starting now, which learns how long each step takes from the
    /// run before it.
    pub fn new(pid: u32, now: i64, from: Option<Commit>, expected: Vec<Phase>, before: Option<&RunState>) -> Self {
        let mut estimates: Vec<Estimate> = before.map(|before| before.estimates.clone()).unwrap_or_default();
        for step in before.iter().flat_map(|before| &before.steps) {
            if let Some(ended) = step.ended_ms {
                let estimate = Estimate { phase: step.phase, took_ms: ended - step.started_ms, compiled: step.compiled };
                estimates.retain(|kept| kept.phase != step.phase);
                estimates.push(estimate);
            }
        }
        Self {
            pid,
            started_ms: now,
            ended_ms: None,
            from,
            to: None,
            expected,
            steps: Vec::new(),
            outcome: Outcome::Running,
            estimates,
        }
    }

    pub fn current(&self) -> Option<Phase> {
        self.steps.last().filter(|step| step.ended_ms.is_none()).map(|step| step.phase)
    }

    /// Ends the step under way and starts `phase`, unless it's under way.
    pub fn begin(&mut self, phase: Phase, now: i64) {
        if self.current() == Some(phase) {
            return;
        }
        self.end_step(now);
        self.steps.push(Step { phase, started_ms: now, ended_ms: None, compiled: 0 });
    }

    pub fn end_step(&mut self, now: i64) {
        if let Some(step) = self.steps.last_mut().filter(|step| step.ended_ms.is_none()) {
            step.ended_ms = Some(now);
        }
    }

    /// Takes in a line the update printed; says whether it moved anything.
    pub fn line(&mut self, line: &str, now: i64) -> bool {
        if let Some(phase) = Phase::of_line(line) {
            let moved = self.current() != Some(phase);
            self.begin(phase, now);
            return moved;
        }
        if line.trim_start().starts_with("Compiling ")
            && let Some(step) = self.steps.last_mut().filter(|step| step.ended_ms.is_none() && step.phase.compiles())
        {
            step.compiled += 1;
            return true;
        }
        false
    }

    pub fn finish(&mut self, outcome: Outcome, now: i64) {
        if outcome == Outcome::Done {
            self.end_step(now);
        }
        self.ended_ms = Some(now);
        self.outcome = outcome;
    }

    pub fn in_flight(&self) -> bool {
        matches!(self.outcome, Outcome::Running | Outcome::Waiting { .. })
    }

    fn estimate(&self, phase: Phase) -> Option<&Estimate> {
        self.estimates.iter().find(|estimate| estimate.phase == phase)
    }

    /// The run as the web UI shows it at `now`. One whose runner is gone
    /// before it finished failed.
    pub fn view(&self, now: i64, runner_alive: bool) -> RunView {
        let outcome = if self.in_flight() && !runner_alive {
            Outcome::Failed { message: "the update stopped before it finished".to_owned() }
        } else {
            self.outcome.clone()
        };
        let finished = !matches!(outcome, Outcome::Running | Outcome::Waiting { .. });
        let current = self.current();
        let mut steps: Vec<StepView> = self
            .steps
            .iter()
            .map(|step| {
                let under_way = step.ended_ms.is_none() && Some(step.phase) == current;
                let status = match (&outcome, under_way) {
                    (_, false) => StepStatus::Done,
                    (Outcome::Waiting { .. }, true) => StepStatus::Waiting,
                    (Outcome::Failed { .. }, true) => StepStatus::Failed,
                    (Outcome::Done, true) => StepStatus::Done,
                    (Outcome::Running, true) => StepStatus::Active,
                };
                let took_ms =
                    step.ended_ms.map(|ended| ended - step.started_ms).or_else(|| under_way.then(|| now - step.started_ms));
                let estimate = self.estimate(step.phase);
                StepView {
                    phase: step.phase,
                    label: step.phase.label().to_owned(),
                    status,
                    took_ms,
                    compiled: step.compiled,
                    expected_compiled: estimate.map(|estimate| estimate.compiled).filter(|compiled| *compiled > 0),
                }
            })
            .collect();
        for phase in &self.expected {
            if !self.steps.iter().any(|step| step.phase == *phase) {
                let status = if finished { StepStatus::Skipped } else { StepStatus::Todo };
                steps.push(StepView {
                    phase: *phase,
                    label: phase.label().to_owned(),
                    status,
                    took_ms: None,
                    compiled: 0,
                    expected_compiled: self.estimate(*phase).map(|estimate| estimate.compiled).filter(|compiled| *compiled > 0),
                });
            }
        }

        let weight = |phase: Phase| self.estimate(phase).map_or(phase.guess_ms(), |estimate| estimate.took_ms.max(1_000));
        let mut total = 0f64;
        let mut done = 0f64;
        for step in &steps {
            if step.status == StepStatus::Skipped {
                continue;
            }
            let full = weight(step.phase) as f64;
            total += full;
            done += match step.status {
                StepStatus::Done => full,
                StepStatus::Active | StepStatus::Waiting | StepStatus::Failed => {
                    let by_crates = step.expected_compiled.map(|expected| f64::from(step.compiled) / f64::from(expected));
                    let by_time = step.took_ms.map_or(0.0, |took| took as f64 / full);
                    full * by_crates.unwrap_or(by_time).min(0.95)
                }
                StepStatus::Todo | StepStatus::Skipped => 0.0,
            };
        }
        let percent =
            if outcome == Outcome::Done || total == 0.0 { 100 } else { ((done / total) * 100.0).round().clamp(0.0, 99.0) as u8 };
        let ended = self.ended_ms.filter(|_| finished);
        RunView {
            outcome,
            from: self.from.clone(),
            to: self.to.clone(),
            started_ms: self.started_ms,
            elapsed_ms: ended.unwrap_or(now) - self.started_ms,
            expected_ms: (!self.estimates.is_empty()).then_some(total as i64),
            percent,
            steps,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Done,
    Active,
    Waiting,
    Failed,
    Todo,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StepView {
    pub phase: Phase,
    pub label: String,
    pub status: StepStatus,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub took_ms: Option<i64>,
    pub compiled: u32,
    /// What it compiled last time, which a bar fills toward.
    pub expected_compiled: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RunView {
    pub outcome: Outcome,
    pub from: Option<Commit>,
    pub to: Option<Commit>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub started_ms: i64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub elapsed_ms: i64,
    /// How long the whole update should take, once a run has said.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub expected_ms: Option<i64>,
    pub percent: u8,
    pub steps: Vec<StepView>,
}

/// Where an update keeps its state and output, in Silt's data dir.
pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("update")
}

pub fn state_path(data_dir: &Path) -> PathBuf {
    dir(data_dir).join("state.json")
}

pub fn output_path(data_dir: &Path) -> PathBuf {
    dir(data_dir).join("output.log")
}

/// The last run's state, if one ran.
pub fn read_state(data_dir: &Path) -> Option<RunState> {
    serde_json::from_slice(&fs::read(state_path(data_dir)).ok()?).ok()
}

/// Writes `state` whole, so a reader never sees half of it.
pub fn write_state(data_dir: &Path, state: &RunState) -> io::Result<()> {
    let dir = dir(data_dir);
    silt_os::create_private_dir(&dir)?;
    let partial = dir.join("state.json.partial");
    silt_os::write_private(&partial, &serde_json::to_vec_pretty(state).map_err(io::Error::other)?)?;
    fs::rename(&partial, state_path(data_dir))
}

/// `line` without the escapes that color it on a terminal.
pub fn plain(line: &str) -> String {
    let mut plain = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            continue;
        }
        if c != '\r' {
            plain.push(c);
        }
    }
    plain
}

/// Whether the runner that wrote `state` still runs: its process is there,
/// and started before the run did, so it isn't another that took its pid.
pub fn runner_alive(state: &RunState) -> bool {
    let Ok(processes) = silt_os::processes() else {
        return true;
    };
    let started_by = u64::try_from(state.started_ms / 1_000).unwrap_or(0) + 2;
    processes.iter().any(|process| process.pid == state.pid && process.started <= started_by)
}

/// The last `count` lines of an update's output.
pub fn tail(text: &str, count: usize) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(count)..].iter().map(|line| (*line).to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const INSTALL: &str = "pnpm -C web install --frozen-lockfile
Lockfile is up to date, resolution step is skipped
pnpm -C web build
✓ built in 31.84s
cargo install --locked --path crates/silt-cli
   Compiling silt-types v0.0.1 (/src/silt/crates/silt-types)
   Compiling silt-crdt v0.0.1 (/src/silt/crates/silt-crdt)
  Installed package `silt-cli v0.0.1` (executable `silt`)
\"/home/me/.cargo/bin/silt\" log > /dev/null
cargo install --locked --path crates/silt-desktop --features tauri/custom-protocol
   Compiling tauri v2.12.5
cargo install --locked --path crates/silt-voice";

    fn ran(state: &mut RunState, output: &str, from: i64) {
        for (at, line) in output.lines().enumerate() {
            state.line(line, from + at as i64 * 1_000);
        }
    }

    #[test]
    fn the_steps_come_from_just_s_recipe_lines() {
        let mut state = RunState::new(1, 0, None, expected(true, false), None);
        state.begin(Phase::Fetch, 0);
        ran(&mut state, INSTALL, 1_000);
        let phases: Vec<_> = state.steps.iter().map(|step| step.phase).collect();
        assert_eq!(phases, [Phase::Fetch, Phase::WebDependencies, Phase::WebBuild, Phase::Cli, Phase::Desktop, Phase::Voice]);
        let compiled: Vec<_> = state.steps.iter().map(|step| step.compiled).collect();
        assert_eq!(compiled, [0, 0, 0, 2, 1, 0]);
        assert_eq!(state.current(), Some(Phase::Voice));
    }

    #[test]
    fn a_run_shows_its_steps_done_under_way_and_to_come() {
        let mut state = RunState::new(1, 0, None, expected(true, false), None);
        state.begin(Phase::Fetch, 0);
        state.line("pnpm -C web install --frozen-lockfile", 2_000);
        let view = state.view(5_000, true);
        let statuses: Vec<_> = view.steps.iter().map(|step| (step.phase, step.status)).collect();
        assert_eq!(statuses[0], (Phase::Fetch, StepStatus::Done));
        assert_eq!(statuses[1], (Phase::WebDependencies, StepStatus::Active));
        assert!(statuses[2..].iter().all(|(_, status)| *status == StepStatus::Todo));
        assert_eq!(statuses.last(), Some(&(Phase::Restart, StepStatus::Todo)));
        assert!(view.percent > 0 && view.percent < 10, "{}", view.percent);
        assert_eq!(view.expected_ms, None);
    }

    #[test]
    fn a_runner_gone_mid_way_failed_and_steps_never_reached_were_skipped() {
        let mut state = RunState::new(1, 0, None, expected(false, false), None);
        state.begin(Phase::Fetch, 0);
        let view = state.view(1_000, false);
        assert!(matches!(view.outcome, Outcome::Failed { .. }));
        assert_eq!(view.steps[0].status, StepStatus::Failed);
        assert!(view.steps[1..].iter().all(|step| step.status == StepStatus::Skipped));
    }

    #[test]
    fn waiting_for_agents_shows_on_the_restart() {
        let mut state = RunState::new(1, 0, None, vec![Phase::Fetch, Phase::Restart], None);
        state.begin(Phase::Fetch, 0);
        state.begin(Phase::Restart, 1_000);
        state.outcome = Outcome::Waiting { agents: 2 };
        let view = state.view(2_000, true);
        assert_eq!(view.steps[1].status, StepStatus::Waiting);
        state.finish(Outcome::Done, 3_000);
        let view = state.view(9_000, true);
        assert_eq!(view.percent, 100);
        assert_eq!(view.elapsed_ms, 3_000);
        assert!(view.steps.iter().all(|step| step.status == StepStatus::Done));
    }

    #[test]
    fn the_next_run_estimates_from_the_last() {
        let mut first = RunState::new(1, 0, None, expected(false, false), None);
        first.begin(Phase::Fetch, 0);
        ran(&mut first, INSTALL, 1_000);
        first.finish(Outcome::Done, 20_000);
        let mut second = RunState::new(2, 100_000, None, expected(false, false), Some(&first));
        let cli = second.estimates.iter().find(|estimate| estimate.phase == Phase::Cli).expect("cli estimate");
        assert_eq!(cli.compiled, 2);
        second.begin(Phase::Cli, 100_000);
        second.line("   Compiling silt-types v0.0.1", 101_000);
        let view = second.view(101_000, true);
        let step = view.steps.iter().find(|step| step.phase == Phase::Cli).expect("cli step");
        assert_eq!((step.compiled, step.expected_compiled), (1, Some(2)));
        assert!(view.expected_ms.is_some());
    }

    #[test]
    fn the_state_survives_a_round_trip_and_tolerates_what_it_lacks() {
        let dir = tempfile::tempdir().expect("dir");
        let mut state = RunState::new(7, 1, Commit::parse("a1b2c3d"), expected(true, false), None);
        state.begin(Phase::Fetch, 1);
        write_state(dir.path(), &state).expect("write");
        assert_eq!(read_state(dir.path()), Some(state));
        let older: RunState =
            serde_json::from_str(r#"{"pid":1,"started_ms":0,"outcome":{"kind":"done"},"later":true}"#).expect("older");
        assert_eq!(older.outcome, Outcome::Done);
    }

    #[test]
    fn colors_are_taken_out() {
        assert_eq!(plain("\u{1b}[1;32m   Compiling\u{1b}[0m silt v0.0.1\r"), "   Compiling silt v0.0.1");
        assert_eq!(Phase::of_line(&plain("\u{1b}[1mpnpm -C web build\u{1b}[0m")), Some(Phase::WebBuild));
    }

    #[test]
    fn this_process_is_a_live_runner() {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("time").as_millis();
        let state = RunState::new(std::process::id(), i64::try_from(now).expect("ms"), None, Vec::new(), None);
        assert!(runner_alive(&state));
        assert!(!runner_alive(&RunState { started_ms: 0, ..state }));
    }

    #[test]
    fn the_tail_is_the_last_lines() {
        assert_eq!(tail("a\nb\nc\n", 2), ["b", "c"]);
        assert_eq!(tail("a", 5), ["a"]);
    }
}
