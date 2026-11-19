//! What a plugin may use of this machine in one call: memory, time and the
//! size of its answers.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use wasmtime::{Engine, StoreLimits, StoreLimitsBuilder};

pub(crate) const TICKS_PER_SECOND: u64 = 20;

#[derive(Debug, Clone)]
pub struct Limits {
    pub memory_bytes: usize,
    pub describe_seconds: u64,
    pub prepare_seconds: u64,
    pub run_seconds: u64,
    /// The longest answer a plugin may give, as JSON.
    pub answer_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { memory_bytes: 256 << 20, describe_seconds: 5, prepare_seconds: 60, run_seconds: 300, answer_bytes: 4 << 20 }
    }
}

impl Limits {
    pub(crate) fn store_limits(&self) -> StoreLimits {
        StoreLimitsBuilder::new()
            .memory_size(self.memory_bytes)
            .instances(64)
            .tables(64)
            .memories(16)
            .trap_on_grow_failure(true)
            .build()
    }
}

/// Moves the engine's epoch on, so a plugin past its time traps.
pub(crate) struct Ticker {
    stop: Arc<AtomicBool>,
}

impl Ticker {
    pub(crate) fn start(engine: Engine) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        std::thread::Builder::new()
            .name("silt-plugin-epochs".into())
            .spawn(move || {
                while !stopping.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(1000 / TICKS_PER_SECOND));
                    engine.increment_epoch();
                }
            })
            .ok();
        Self { stop }
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// A trap in words a person can act on.
pub(crate) fn explain(error: &wasmtime::Error) -> String {
    match error.downcast_ref::<wasmtime::Trap>() {
        Some(wasmtime::Trap::Interrupt) => "it ran past its time".into(),
        Some(wasmtime::Trap::StackOverflow) => "it overflowed its stack".into(),
        Some(trap) => format!("it trapped: {trap}"),
        None if format!("{error:#}").contains("memory") => format!("it used more memory than plugins may: {error:#}"),
        None => format!("{error:#}"),
    }
}
