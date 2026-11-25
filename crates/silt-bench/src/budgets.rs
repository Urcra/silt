//! The budgets of ADR 0034, and how a measurement compares with one.

use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Limit {
    Time(Duration),
    /// Bytes stored for each byte of transcript recorded.
    Ratio(f64),
    /// CPU time over wall time, where 1.0 is one core kept busy.
    CoreShare(f64),
    /// Checked somewhere else, or not set yet.
    Described(&'static str),
}

impl Limit {
    /// This limit raised by `percent` of itself.
    pub fn raised(self, percent: u32) -> Self {
        let factor = 1.0 + f64::from(percent) / 100.0;
        match self {
            Self::Time(limit) => Self::Time(limit.mul_f64(factor)),
            Self::Ratio(limit) => Self::Ratio(limit * factor),
            Self::CoreShare(limit) => Self::CoreShare(limit * factor),
            Self::Described(text) => Self::Described(text),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Measured {
    Time(Duration),
    Ratio(f64),
    CoreShare(f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Enforced,
    /// Not met yet; the task named makes it pass and enforces it.
    #[expect(dead_code, reason = "every budget is met for now; a new one starts pending (ADR 0034)")]
    Pending(&'static str),
    /// Measured outside `silt-bench`.
    Elsewhere(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Budget {
    pub name: &'static str,
    pub measures: &'static str,
    /// On the reference machine, the development box (ADR 0034).
    pub dev: Limit,
    /// On a cloud session's machine, which is several times slower (ADR 0037).
    pub cloud: Limit,
    /// On a GitHub Actions runner (ADR 0126).
    pub ci: Limit,
    pub status: Status,
}

impl Budget {
    pub fn limit(&self, machine: Machine) -> Limit {
        match machine {
            Machine::Dev => self.dev,
            Machine::Cloud => self.cloud,
            Machine::Ci => self.ci,
        }
    }
}

/// Which limits apply: the machine the budgets were set on, a cloud
/// session's (ADR 0037), or a GitHub Actions runner's (ADR 0126).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Machine {
    Dev,
    Cloud,
    Ci,
}

/// The variables a machine is chosen by.
#[derive(Debug, Clone, Copy, Default)]
pub struct Env<'a> {
    /// `SILT_BENCH_MACHINE`
    pub chosen: Option<&'a str>,
    /// `CLAUDE_CODE_REMOTE`, which cloud sessions set to `true`
    pub remote: Option<&'a str>,
    /// `GITHUB_ACTIONS`, which runners set to `true`
    pub actions: Option<&'a str>,
}

impl Machine {
    /// From `SILT_BENCH_MACHINE` when it's set, and otherwise from where
    /// this runs.
    pub fn choose(env: Env<'_>) -> Result<Self, String> {
        match env.chosen {
            Some("dev") => Ok(Self::Dev),
            Some("cloud") => Ok(Self::Cloud),
            Some("ci") => Ok(Self::Ci),
            Some(other) => Err(format!("SILT_BENCH_MACHINE is {other:?}; it takes dev, cloud or ci")),
            None if env.actions == Some("true") => Ok(Self::Ci),
            None if env.remote == Some("true") => Ok(Self::Cloud),
            None => Ok(Self::Dev),
        }
    }
}

impl fmt::Display for Machine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dev => f.write_str("dev limits"),
            Self::Cloud => f.write_str("cloud limits"),
            Self::Ci => f.write_str("CI limits"),
        }
    }
}

pub const RECORD_IMPORT: &str = "record.import";
pub const IMPORT_SESSION: &str = "import.session";
pub const RECORD_HOOK: &str = "record.hook";
pub const RECORD_HOOK_DAEMONLESS: &str = "record.hook_daemonless";
pub const STORE_SIZE: &str = "store.size";
pub const DAEMON_STARTUP: &str = "daemon.startup";
pub const DAEMON_READY: &str = "daemon.ready";
pub const CHECKOUT_RESCAN: &str = "checkout.rescan";
pub const DAEMON_IDLE: &str = "daemon.idle";
pub const SYNC_PUSH: &str = "sync.push";
pub const SYNC_PULL: &str = "sync.pull";
pub const APP_THREADS: &str = "app.threads";
pub const APP_HISTORY: &str = "app.history";
pub const PAGE_FOLD: &str = "page.fold";
pub const DOC_REPLAY: &str = "doc.replay";
pub const SHEET_EVAL: &str = "sheet.eval";

pub const BUDGETS: &[Budget] = &[
    Budget {
        name: RECORD_IMPORT,
        measures: "tracking a 100 MB transcript, the daemon already running",
        dev: Limit::Time(Duration::from_secs(1)),
        cloud: Limit::Time(Duration::from_secs(4)),
        ci: Limit::Time(Duration::from_secs(2)),
        status: Status::Enforced,
    },
    Budget {
        name: IMPORT_SESSION,
        measures: "`silt import` of a 100 MB transcript, the daemon already running",
        dev: Limit::Time(Duration::from_secs(1)),
        cloud: Limit::Time(Duration::from_secs(4)),
        ci: Limit::Time(Duration::from_secs(2)),
        status: Status::Enforced,
    },
    Budget {
        name: RECORD_HOOK,
        measures: "`silt hook` forwarded to the running daemon, p95 of the slowest event, at 100k deltas and 20k files",
        dev: Limit::Time(Duration::from_millis(20)),
        cloud: Limit::Time(Duration::from_millis(70)),
        ci: Limit::Time(Duration::from_millis(40)),
        status: Status::Enforced,
    },
    Budget {
        name: RECORD_HOOK_DAEMONLESS,
        measures: "the hook that finds no daemon and captures in process before starting one, p95 of the slowest event, at 100k deltas and 20k files",
        dev: Limit::Time(Duration::from_millis(50)),
        cloud: Limit::Time(Duration::from_millis(230)),
        ci: Limit::Time(Duration::from_millis(200)),
        status: Status::Enforced,
    },
    Budget {
        name: STORE_SIZE,
        measures: "the data dir over the transcript bytes it records",
        dev: Limit::Ratio(1.5),
        cloud: Limit::Ratio(1.5),
        ci: Limit::Ratio(1.5),
        status: Status::Enforced,
    },
    Budget {
        name: DAEMON_STARTUP,
        measures: "spawning the daemon to its first answer, holding every fixture",
        dev: Limit::Time(Duration::from_millis(100)),
        cloud: Limit::Time(Duration::from_millis(100)),
        ci: Limit::Time(Duration::from_millis(100)),
        status: Status::Enforced,
    },
    Budget {
        name: DAEMON_READY,
        measures: "spawning the daemon to its first thread listing, holding every fixture (100 MB of transcript), at 100 MB/s",
        dev: Limit::Time(Duration::from_secs(1)),
        cloud: Limit::Time(Duration::from_millis(1600)),
        ci: Limit::Time(Duration::from_secs(1)),
        status: Status::Enforced,
    },
    Budget {
        name: CHECKOUT_RESCAN,
        measures: "the capture step after a Bash call, comparing a 50,000-file checkout",
        dev: Limit::Time(Duration::from_millis(200)),
        cloud: Limit::Time(Duration::from_millis(850)),
        ci: Limit::Time(Duration::from_millis(450)),
        status: Status::Enforced,
    },
    Budget {
        name: DAEMON_IDLE,
        measures: "CPU of a daemon with nothing to do, over 10 s",
        dev: Limit::CoreShare(0.001),
        cloud: Limit::CoreShare(0.002),
        ci: Limit::CoreShare(0.002),
        status: Status::Enforced,
    },
    Budget {
        name: "web.thread",
        measures: "a 2,000-message thread in the browser",
        dev: Limit::Described("shown < 100 ms, heap < 200 MB"),
        cloud: Limit::Described("shown < 210 ms, heap < 200 MB"),
        ci: Limit::Described("shown < 210 ms, heap < 200 MB"),
        status: Status::Elsewhere("enforced by web/e2e/perf.spec.ts on the perf build"),
    },
    Budget {
        name: "web.scroll",
        measures: "long tasks over 100 wheel notches up that thread (ADR 0101)",
        dev: Limit::Described("< 500 ms blocked"),
        cloud: Limit::Described("< 1 s blocked"),
        ci: Limit::Described("< 1 s blocked"),
        status: Status::Elsewhere("enforced by web/e2e/perf.spec.ts on the perf build"),
    },
    Budget {
        name: "web.change",
        measures: "history read again after one change to that thread, open (ADR 0101)",
        dev: Limit::Described("< 64 KB"),
        cloud: Limit::Described("< 64 KB"),
        ci: Limit::Described("< 64 KB"),
        status: Status::Elsewhere("enforced by web/e2e/perf.spec.ts on any build"),
    },
    Budget {
        name: SYNC_PUSH,
        measures: "`silt sync --once` pushing a family holding 100 MB of transcript to a server, at 100 MB/s",
        dev: Limit::Time(Duration::from_secs(1)),
        cloud: Limit::Time(Duration::from_millis(2600)),
        ci: Limit::Time(Duration::from_millis(2200)),
        status: Status::Enforced,
    },
    Budget {
        name: SYNC_PULL,
        measures: "`silt resume` pulling that family onto another machine and restoring its session, at 100 MB/s",
        dev: Limit::Time(Duration::from_secs(1)),
        cloud: Limit::Time(Duration::from_secs(7)),
        ci: Limit::Time(Duration::from_millis(2600)),
        status: Status::Enforced,
    },
    Budget {
        name: APP_THREADS,
        measures: "the server's hosted app listing threads, holding that family",
        dev: Limit::Time(Duration::from_millis(100)),
        cloud: Limit::Time(Duration::from_millis(200)),
        ci: Limit::Time(Duration::from_millis(200)),
        status: Status::Enforced,
    },
    Budget {
        name: APP_HISTORY,
        measures: "the hosted app's first page of that family's 100 MB session, the first time it's opened",
        dev: Limit::Time(Duration::from_millis(100)),
        cloud: Limit::Time(Duration::from_millis(230)),
        ci: Limit::Time(Duration::from_millis(100)),
        status: Status::Enforced,
    },
    Budget {
        name: PAGE_FOLD,
        measures: "a page's data folded along its thread: 10,000 writes over 1,000 documents",
        dev: Limit::Time(Duration::from_millis(10)),
        cloud: Limit::Time(Duration::from_millis(25)),
        ci: Limit::Time(Duration::from_millis(15)),
        status: Status::Enforced,
    },
    Budget {
        name: DOC_REPLAY,
        measures: "a doc's text replayed from 20,000 edits",
        dev: Limit::Time(Duration::from_millis(15)),
        cloud: Limit::Time(Duration::from_millis(40)),
        ci: Limit::Time(Duration::from_millis(15)),
        status: Status::Enforced,
    },
    Budget {
        name: SHEET_EVAL,
        measures: "a 1,000-row sheet folded from its cells and computed, a SUM on every row",
        dev: Limit::Time(Duration::from_millis(20)),
        cloud: Limit::Time(Duration::from_millis(50)),
        ci: Limit::Time(Duration::from_millis(25)),
        status: Status::Enforced,
    },
];

pub fn budget(name: &str) -> Option<&'static Budget> {
    BUDGETS.iter().find(|budget| budget.name == name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Within,
    Over,
    /// The measurement is of another kind than the limit.
    Mismatched,
}

pub fn verdict(limit: Limit, measured: Measured) -> Verdict {
    let within = match (limit, measured) {
        (Limit::Time(limit), Measured::Time(measured)) => measured < limit,
        (Limit::Ratio(limit), Measured::Ratio(measured)) => measured <= limit,
        (Limit::CoreShare(limit), Measured::CoreShare(measured)) => measured <= limit,
        _ => return Verdict::Mismatched,
    };
    if within { Verdict::Within } else { Verdict::Over }
}

impl fmt::Display for Limit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Time(limit) => write!(f, "< {}", Measured::Time(*limit)),
            Self::Ratio(limit) => write!(f, "≤ {limit:.2}×"),
            Self::CoreShare(limit) => write!(f, "≤ {}", Measured::CoreShare(*limit)),
            Self::Described(text) => f.write_str(text),
        }
    }
}

impl fmt::Display for Measured {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Time(time) if *time < Duration::from_secs(1) => write!(f, "{:.1} ms", time.as_secs_f64() * 1000.0),
            Self::Time(time) => write!(f, "{:.2} s", time.as_secs_f64()),
            Self::Ratio(ratio) => write!(f, "{ratio:.2}×"),
            Self::CoreShare(share) => write!(f, "{:.2} % of a core", share * 100.0),
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Enforced => f.write_str("enforced"),
            Self::Pending(task) => write!(f, "pending, {task}"),
            Self::Elsewhere(place) => write!(f, "measured by {place}"),
        }
    }
}

/// The smaller of two measurements of one budget, all of which are better
/// smaller; `again` when they're of different kinds.
pub fn better(first: Measured, again: Measured) -> Measured {
    match (first, again) {
        (Measured::Time(first), Measured::Time(again)) => Measured::Time(first.min(again)),
        (Measured::Ratio(first), Measured::Ratio(again)) => Measured::Ratio(first.min(again)),
        (Measured::CoreShare(first), Measured::CoreShare(again)) => Measured::CoreShare(first.min(again)),
        _ => again,
    }
}

/// One row of the report.
pub fn line(budget: &Budget, machine: Machine, measured: Option<Measured>) -> String {
    let limit = budget.limit(machine);
    let (shown, verdict) = match measured {
        Some(measured) => (measured.to_string(), verdict_word(verdict(limit, measured))),
        None => ("-".to_owned(), "not measured"),
    };
    format!("{:<23} {:>18}  {:<30} {:<13} {}", budget.name, shown, limit.to_string(), verdict, budget.status)
}

/// What the gate prints for an enforced budget that's over.
pub fn failure(budget: &Budget, limits: Machine, measured: Measured, machine: &str) -> String {
    format!(
        "over budget: {} ({}) measured {measured}, limit {}, on {machine} with {limits}",
        budget.name,
        budget.measures,
        budget.limit(limits)
    )
}

/// What the gate prints for an enforced budget that's over, but by no more
/// than the tolerance it was run with (ADR 0052).
pub fn tolerated(budget: &Budget, limits: Machine, measured: Measured, tolerance: u32, machine: &str) -> String {
    format!(
        "slower than budget: {} ({}) measured {measured}, limit {}, on {machine} with {limits}; passed, as within {tolerance} % of it",
        budget.name,
        budget.measures,
        budget.limit(limits)
    )
}

/// What's printed for a budget held to another machine's limit that's
/// over the dev limit, which stays the target (ADR 0037). It doesn't fail.
pub fn off_target(budget: &Budget, measured: Measured) -> Option<String> {
    (verdict(budget.dev, measured) == Verdict::Over)
        .then(|| format!("off target: {} measured {measured}, dev target {}", budget.name, budget.dev))
}

/// The report as a Markdown table, for a GitHub Actions job's summary.
pub fn summary(shown: &[&Budget], limits: Machine, measured: &HashMap<&str, Measured>, machine: &str) -> String {
    let mut out = format!("### Performance budgets\n\nOn {machine}, with {limits}.\n\n");
    if limits == Machine::Dev {
        out.push_str("| Budget | Measured | Limit | |\n|---|---|---|---|\n");
    } else {
        out.push_str("| Budget | Measured | Limit | | Dev target | |\n|---|---|---|---|---|---|\n");
    }
    for budget in shown {
        let measured = measured.get(budget.name).copied();
        let (value, held, target) = match measured {
            Some(measured) => {
                (measured.to_string(), mark(verdict(budget.limit(limits), measured)), mark(verdict(budget.dev, measured)))
            }
            None => ("-".to_owned(), "", ""),
        };
        let limit = budget.limit(limits);
        if limits == Machine::Dev {
            out.push_str(&format!("| `{}` | {value} | {limit} | {held} |\n", budget.name));
        } else {
            out.push_str(&format!("| `{}` | {value} | {limit} | {held} | {} | {target} |\n", budget.name, budget.dev));
        }
    }
    out.push('\n');
    out
}

fn mark(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Within => "✅",
        Verdict::Over => "❌",
        Verdict::Mismatched => "",
    }
}

fn verdict_word(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Within => "within",
        Verdict::Over => "OVER",
        Verdict::Mismatched => "mismatched",
    }
}

/// The median of `times`, which takes the middle of three runs as ADR 0034
/// asks for budgets in milliseconds.
pub fn median(mut times: Vec<Duration>) -> Option<Duration> {
    times.sort();
    times.get(times.len() / 2).copied()
}

/// The `quantile` (0 to 1) of `times`, nearest rank.
pub fn quantile(mut times: Vec<Duration>, quantile: f64) -> Option<Duration> {
    times.sort();
    let last = times.len().checked_sub(1)?;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "an index into a short list"
    )]
    let index = (last as f64 * quantile).round() as usize;
    times.get(index).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_at_its_limit_is_over() {
        let limit = Limit::Time(Duration::from_secs(1));
        assert_eq!(verdict(limit, Measured::Time(Duration::from_millis(999))), Verdict::Within);
        assert_eq!(verdict(limit, Measured::Time(Duration::from_secs(1))), Verdict::Over);
    }

    #[test]
    fn a_ratio_or_share_at_its_limit_is_within() {
        assert_eq!(verdict(Limit::Ratio(1.5), Measured::Ratio(1.5)), Verdict::Within);
        assert_eq!(verdict(Limit::Ratio(1.5), Measured::Ratio(1.51)), Verdict::Over);
        assert_eq!(verdict(Limit::CoreShare(0.001), Measured::CoreShare(0.0005)), Verdict::Within);
        assert_eq!(verdict(Limit::CoreShare(0.001), Measured::CoreShare(0.01)), Verdict::Over);
    }

    #[test]
    fn a_measurement_of_another_kind_never_passes() {
        assert_eq!(verdict(Limit::Ratio(1.5), Measured::Time(Duration::ZERO)), Verdict::Mismatched);
        assert_eq!(verdict(Limit::Described("elsewhere"), Measured::Ratio(0.0)), Verdict::Mismatched);
    }

    #[test]
    fn every_budget_measured_here_has_a_comparable_limit() {
        for budget in BUDGETS {
            for limit in [budget.dev, budget.cloud, budget.ci] {
                let comparable = !matches!(limit, Limit::Described(_));
                assert_eq!(comparable, !matches!(budget.status, Status::Elsewhere(_)), "{}", budget.name);
            }
        }
    }

    #[test]
    fn another_machines_limit_is_of_the_same_kind_and_never_tighter() {
        for budget in BUDGETS {
            for other in [budget.cloud, budget.ci] {
                let looser = match (budget.dev, other) {
                    (Limit::Time(dev), Limit::Time(other)) => other >= dev,
                    (Limit::Ratio(dev), Limit::Ratio(other)) | (Limit::CoreShare(dev), Limit::CoreShare(other)) => other >= dev,
                    (Limit::Described(_), Limit::Described(_)) => true,
                    _ => false,
                };
                assert!(looser, "{}", budget.name);
            }
        }
    }

    #[test]
    fn the_machine_is_chosen_by_name_or_by_where_it_runs() {
        let env = |chosen, remote, actions| Env { chosen, remote, actions };
        assert_eq!(Machine::choose(env(None, None, None)), Ok(Machine::Dev));
        assert_eq!(Machine::choose(env(None, Some("true"), None)), Ok(Machine::Cloud));
        assert_eq!(Machine::choose(env(None, Some("false"), None)), Ok(Machine::Dev));
        assert_eq!(Machine::choose(env(None, None, Some("true"))), Ok(Machine::Ci));
        assert_eq!(Machine::choose(env(None, Some("true"), Some("true"))), Ok(Machine::Ci));
        assert_eq!(Machine::choose(env(Some("dev"), Some("true"), Some("true"))), Ok(Machine::Dev));
        assert_eq!(Machine::choose(env(Some("cloud"), None, None)), Ok(Machine::Cloud));
        assert_eq!(Machine::choose(env(Some("ci"), None, None)), Ok(Machine::Ci));
        assert!(Machine::choose(env(Some("laptop"), None, None)).is_err());
    }

    #[test]
    fn the_summary_puts_the_dev_target_beside_another_machines_limit() {
        let shown = [budget(SYNC_PULL).expect("listed"), budget(SYNC_PUSH).expect("listed")];
        let measured = HashMap::from([(SYNC_PULL, Measured::Time(Duration::from_millis(1300)))]);
        let summary = summary(&shown, Machine::Ci, &measured, "4 cores, ext2/ext3");
        assert!(summary.contains("On 4 cores, ext2/ext3, with CI limits."), "{summary}");
        assert!(summary.contains("| `sync.pull` | 1.30 s | < 2.60 s | ✅ | < 1.00 s | ❌ |"), "{summary}");
        assert!(summary.contains("| `sync.push` | - | < 2.20 s |  | < 1.00 s |  |"), "{summary}");
    }

    #[test]
    fn only_a_measurement_over_the_dev_limit_is_off_target() {
        let budget = budget(SYNC_PULL).expect("listed");
        assert_eq!(off_target(budget, Measured::Time(Duration::from_millis(900))), None);
        assert_eq!(
            off_target(budget, Measured::Time(Duration::from_millis(1300))).as_deref(),
            Some("off target: sync.pull measured 1.30 s, dev target < 1.00 s")
        );
    }

    #[test]
    fn names_are_unique() {
        for (index, budget) in BUDGETS.iter().enumerate() {
            assert!(BUDGETS[index + 1..].iter().all(|other| other.name != budget.name), "{}", budget.name);
        }
    }

    #[test]
    fn a_failure_names_the_budget_its_limit_and_the_measurement() {
        let budget = budget(RECORD_IMPORT).expect("listed");
        let message = failure(budget, Machine::Dev, Measured::Time(Duration::from_millis(1500)), "32 cores, ext2/ext3");
        assert_eq!(
            message,
            "over budget: record.import (tracking a 100 MB transcript, the daemon already running) \
             measured 1.50 s, limit < 1.00 s, on 32 cores, ext2/ext3 with dev limits"
        );
    }

    #[test]
    fn a_limit_raised_by_half_lets_through_up_to_half_again() {
        let limit = Limit::Time(Duration::from_secs(1)).raised(50);
        assert_eq!(verdict(limit, Measured::Time(Duration::from_millis(1490))), Verdict::Within);
        assert_eq!(verdict(limit, Measured::Time(Duration::from_millis(1500))), Verdict::Over);
        assert_eq!(verdict(Limit::Ratio(1.5).raised(50), Measured::Ratio(2.25)), Verdict::Within);
        assert_eq!(Limit::CoreShare(0.001).raised(0), Limit::CoreShare(0.001));
    }

    #[test]
    fn measurements_read_in_their_units() {
        assert_eq!(Measured::Time(Duration::from_micros(12_340)).to_string(), "12.3 ms");
        assert_eq!(Measured::Ratio(2.04).to_string(), "2.04×");
        assert_eq!(Measured::CoreShare(0.0123).to_string(), "1.23 % of a core");
        assert_eq!(Limit::CoreShare(0.001).to_string(), "≤ 0.10 % of a core");
    }

    #[test]
    fn the_better_of_two_runs_stands() {
        let (fast, slow) = (Duration::from_millis(18), Duration::from_millis(21));
        assert_eq!(better(Measured::Time(slow), Measured::Time(fast)), Measured::Time(fast));
        assert_eq!(better(Measured::Time(fast), Measured::Time(slow)), Measured::Time(fast));
        assert_eq!(better(Measured::CoreShare(0.002), Measured::CoreShare(0.0005)), Measured::CoreShare(0.0005));
    }

    #[test]
    fn the_median_of_three_is_the_middle_one() {
        let times = [30, 10, 20].map(Duration::from_millis).to_vec();
        assert_eq!(median(times), Some(Duration::from_millis(20)));
        assert_eq!(median(Vec::new()), None);
    }

    #[test]
    fn the_p95_of_forty_is_near_the_top() {
        let times: Vec<Duration> = (1..=40).map(Duration::from_millis).collect();
        assert_eq!(quantile(times, 0.95), Some(Duration::from_millis(38)));
    }
}
