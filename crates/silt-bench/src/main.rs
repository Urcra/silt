//! Measures Silt against its performance budgets (ADR 0034).
//!
//! `silt-bench check` measures the enforced budgets and fails on any over
//! its limit; the gate runs it. `silt-bench report` measures every budget.
//! A cloud session's machine has its own limits (ADR 0037).
//! Build both binaries with `cargo build --profile perf -p silt-cli -p
//! silt-bench`, since debug timings say nothing about what people see.

mod budgets;
mod daemon;
mod fixtures;
mod measure;

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Instant;

use clap::{Parser, Subcommand};
use exn::{Exn, ResultExt};

use budgets::{BUDGETS, Budget, Env, Machine, Measured, Status, Verdict};
use fixtures::Fixtures;
use measure::Scenario;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct BenchError(pub String);

pub fn fail(message: impl Into<String>) -> Exn<BenchError> {
    Exn::new(BenchError(message.into()))
}

#[derive(Parser)]
#[command(about = "Measures Silt against its performance budgets (ADR 0034)")]
struct Cli {
    #[command(subcommand)]
    mode: Mode,
    /// The `silt` binary to measure; by default the one beside this one
    #[arg(long, global = true)]
    silt: Option<PathBuf>,
    /// Where fixtures are cached; by default `target/silt-bench`
    #[arg(long, global = true)]
    fixtures: Option<PathBuf>,
    /// Measure only these budgets
    #[arg(long, global = true)]
    only: Vec<String>,
    /// Leave each scenario's data dir and checkouts in /tmp, to look into
    #[arg(long, global = true)]
    keep: bool,
    /// Pass a budget over its limit by at most this many percent, with a warning
    #[arg(long, global = true, default_value_t = 0)]
    tolerance: u32,
}

#[derive(Subcommand, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Measure the enforced budgets, failing on any over its limit
    Check,
    /// Measure every budget and print them all
    Report,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("a runtime");
    match runtime.block_on(run(&cli)) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("silt-bench: {error:?}");
            ExitCode::FAILURE
        }
    }
}

/// Whether every budget measured is within its limit, or only reported.
async fn run(cli: &Cli) -> Result<bool, Exn<BenchError>> {
    let here = std::env::current_exe().or_raise(|| BenchError("cannot find this executable".into()))?;
    let bin = here.parent().expect("in a directory");
    let silt = cli.silt.clone().unwrap_or_else(|| bin.join("silt"));
    // Absolute, as the daemon is started from `/`.
    let Ok(silt) = silt_os::canonicalize(&silt) else {
        return Err(fail(format!(
            "no silt binary at {}; build it with `cargo build --profile perf -p silt-cli`",
            silt.display()
        )));
    };
    let fixture_dir = cli.fixtures.clone().unwrap_or_else(|| bin.parent().unwrap_or(bin).join("silt-bench"));
    let fixtures = Fixtures::in_dir(&fixture_dir);
    let (chosen, remote, actions) = (
        std::env::var("SILT_BENCH_MACHINE").ok(),
        std::env::var("CLAUDE_CODE_REMOTE").ok(),
        std::env::var("GITHUB_ACTIONS").ok(),
    );
    let limits = Machine::choose(Env { chosen: chosen.as_deref(), remote: remote.as_deref(), actions: actions.as_deref() })
        .map_err(fail)?;
    for name in &cli.only {
        if budgets::budget(name).is_none() {
            return Err(fail(format!("no budget is named {name}")));
        }
    }
    let selected: Vec<&Budget> = BUDGETS
        .iter()
        .filter(|budget| cli.only.is_empty() || cli.only.iter().any(|name| name == budget.name))
        .filter(|budget| cli.mode == Mode::Report || budget.status == Status::Enforced)
        .collect();
    let scenarios: Vec<Scenario> = Scenario::ALL
        .into_iter()
        .filter(|scenario| selected.iter().any(|budget| scenario.budgets().contains(&budget.name)))
        .collect();
    if cli.mode == Mode::Check && scenarios.is_empty() {
        println!("silt-bench: no budget is enforced yet");
        return Ok(true);
    }

    let wanted: Vec<&str> = selected.iter().map(|budget| budget.name).collect();
    let mut measured: HashMap<&str, Measured> = HashMap::new();
    for scenario in &scenarios {
        let start = Instant::now();
        measured.extend(scenario.run(&silt, &fixtures, &wanted, cli.keep).await?);
        eprintln!("silt-bench: {scenario:?} measured in {:.1?}", start.elapsed());
    }
    if cli.mode == Mode::Check {
        // ADR 0034: one more run before an enforced budget fails, for the
        // noise of a busy machine. Other work on the machine only ever adds
        // to a measurement, so the better of the two stands.
        for scenario in &scenarios {
            if scenario.budgets().iter().any(|name| over(&selected, limits, &measured, name)) {
                eprintln!("silt-bench: over budget; measuring {scenario:?} again");
                for (name, again) in scenario.run(&silt, &fixtures, &wanted, cli.keep).await? {
                    let better = measured.get(name).map_or(again, |first| budgets::better(*first, again));
                    measured.insert(name, better);
                }
            }
        }
    }

    let machine = machine(&fixture_dir);
    println!("silt-bench on {machine}, with {limits}");
    let shown: Vec<&Budget> =
        BUDGETS.iter().filter(|budget| selected.contains(budget) || measured.contains_key(budget.name)).collect();
    for budget in &shown {
        println!("{}", budgets::line(budget, limits, measured.get(budget.name).copied()));
    }
    if limits != Machine::Dev {
        off_target(&shown, &measured, actions.as_deref() == Some("true"));
    }
    if let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        let summary = budgets::summary(&shown, limits, &measured, &machine);
        if let Err(error) = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .and_then(|mut file| file.write_all(summary.as_bytes()))
        {
            eprintln!("silt-bench: the job summary wasn't written: {error}");
        }
    }
    if cli.mode == Mode::Report {
        return Ok(true);
    }
    let mut within = true;
    for budget in selected.iter().filter(|budget| budget.status == Status::Enforced) {
        if !over(&selected, limits, &measured, budget.name) {
            continue;
        }
        match measured.get(budget.name) {
            Some(measured) if budgets::verdict(budget.limit(limits).raised(cli.tolerance), *measured) == Verdict::Within => {
                eprintln!("{}", budgets::tolerated(budget, limits, *measured, cli.tolerance, &machine));
            }
            Some(measured) => {
                within = false;
                eprintln!("{}", budgets::failure(budget, limits, *measured, &machine));
            }
            None => {
                within = false;
                eprintln!("over budget: {} was never measured", budget.name);
            }
        }
    }
    Ok(within)
}

/// The dev limits stay the targets on a machine held to its own (ADR 0037),
/// so a budget over one is noted, on a runner as a warning on the job.
fn off_target(shown: &[&Budget], measured: &HashMap<&str, Measured>, actions: bool) {
    for budget in shown {
        let Some(line) = measured.get(budget.name).and_then(|measured| budgets::off_target(budget, *measured)) else {
            continue;
        };
        if actions {
            println!("::warning title=off target::{line}");
        } else {
            eprintln!("{line}");
        }
    }
}

fn over(selected: &[&Budget], limits: Machine, measured: &HashMap<&str, Measured>, name: &str) -> bool {
    let Some(budget) = selected.iter().find(|budget| budget.name == name && budget.status == Status::Enforced) else {
        return false;
    };
    measured.get(name).is_none_or(|measured| budgets::verdict(budget.limit(limits), *measured) != Verdict::Within)
}

/// The machine's cores and the filesystem the fixtures are on, which
/// budgets depend on.
fn machine(dir: &Path) -> String {
    let cores = std::thread::available_parallelism().map_or(0, std::num::NonZero::get);
    let filesystem = Command::new("stat")
        .args(["-f", "-c", "%T"])
        .arg(dir)
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "an unknown filesystem".to_owned());
    format!("{cores} cores, {filesystem}")
}
