//! Sync between replicas and a server, as state machines without I/O
//! (ADR 0009). The daemon and server feed them requests and responses.

mod client;
mod room;

pub use client::{ClientSync, Effect, Outcome, Pull, Push, PUSH_BATCH};
pub use room::{Prepared, PullError, PushError, PushOutcome, Room};
