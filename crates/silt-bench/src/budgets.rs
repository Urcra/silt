//! The budgets of ADR 0034, and how a measurement compares with one.

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
    pub limit: Limit,
    pub status: Status,
}

pub const RECORD_IMPORT: &str = "record.import";
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

pub const BUDGETS: &[Budget] = &[
    Budget {
        name: RECORD_IMPORT,
        measures: "tracking a 100 MB transcript, the daemon already running",
        limit: Limit::Time(Duration::from_secs(1)),
        status: Status::Enforced,
    },
    Budget {
        name: RECORD_HOOK,
        measures: "`silt hook` forwarded to the running daemon, p95 of the slowest event, at 100k deltas and 20k files",
        limit: Limit::Time(Duration::from_millis(20)),
        status: Status::Enforced,
    },
    Budget {
        name: RECORD_HOOK_DAEMONLESS,
        measures: "the hook that finds no daemon and captures in process before starting one, p95 of the slowest event, at 100k deltas and 20k files",
        limit: Limit::Time(Duration::from_millis(200)),
        status: Status::Enforced,
    },
    Budget {
        name: STORE_SIZE,
        measures: "the data dir over the transcript bytes it records",
        limit: Limit::Ratio(1.5),
        status: Status::Enforced,
    },
    Budget {
        name: DAEMON_STARTUP,
        measures: "spawning the daemon to its first answer, holding every fixture",
        limit: Limit::Time(Duration::from_millis(100)),
        status: Status::Enforced,
    },
    Budget {
        name: DAEMON_READY,
        measures: "spawning the daemon to its first thread listing, holding every fixture (100 MB of transcript), at 100 MB/s",
        limit: Limit::Time(Duration::from_secs(1)),
        status: Status::Enforced,
    },
    Budget {
        name: CHECKOUT_RESCAN,
        measures: "the capture step after a Bash call, comparing a 50,000-file checkout",
        limit: Limit::Time(Duration::from_millis(200)),
        status: Status::Enforced,
    },
    Budget {
        name: DAEMON_IDLE,
        measures: "CPU of a daemon with nothing to do, over 10 s",
        limit: Limit::CoreShare(0.001),
        status: Status::Enforced,
    },
    Budget {
        name: "web.thread",
        measures: "a 2,000-message thread in the browser",
        limit: Limit::Described("shown < 100 ms, heap < 200 MB"),
        status: Status::Elsewhere("enforced by web/e2e/perf.spec.ts on the perf build"),
    },
    Budget {
        name: SYNC_PUSH,
        measures: "`silt sync --once` pushing a family holding 100 MB of transcript to a server, at 100 MB/s",
        limit: Limit::Time(Duration::from_secs(1)),
        status: Status::Enforced,
    },
    Budget {
        name: SYNC_PULL,
        measures: "`silt resume` pulling that family onto another machine and restoring its session, at 100 MB/s",
        limit: Limit::Time(Duration::from_secs(1)),
        status: Status::Enforced,
    },
    Budget {
        name: APP_THREADS,
        measures: "the server's hosted app listing threads, holding that family",
        limit: Limit::Time(Duration::from_millis(100)),
        status: Status::Enforced,
    },
    Budget {
        name: APP_HISTORY,
        measures: "the hosted app's first page of that family's 100 MB session, the first time it's opened",
        limit: Limit::Time(Duration::from_millis(100)),
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
pub fn line(budget: &Budget, measured: Option<Measured>) -> String {
    let (shown, verdict) = match measured {
        Some(measured) => (measured.to_string(), verdict_word(verdict(budget.limit, measured))),
        None => ("-".to_owned(), "not measured"),
    };
    format!("{:<23} {:>18}  {:<30} {:<13} {}", budget.name, shown, budget.limit.to_string(), verdict, budget.status)
}

/// What the gate prints for an enforced budget that's over.
pub fn failure(budget: &Budget, measured: Measured, machine: &str) -> String {
    format!("over budget: {} ({}) measured {measured}, limit {}, on {machine}", budget.name, budget.measures, budget.limit)
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
            let comparable = !matches!(budget.limit, Limit::Described(_));
            assert_eq!(comparable, !matches!(budget.status, Status::Elsewhere(_)), "{}", budget.name);
        }
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
        let message = failure(budget, Measured::Time(Duration::from_millis(1500)), "32 cores, ext2/ext3");
        assert_eq!(
            message,
            "over budget: record.import (tracking a 100 MB transcript, the daemon already running) \
             measured 1.50 s, limit < 1.00 s, on 32 cores, ext2/ext3"
        );
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
