//! Runs coding agents over the Agent Client Protocol for the web UI
//! (ADR 0005, ADR 0014): finding them in the ACP registry, starting one in
//! a checkout, and relaying its session to the daemon over channels.

mod host;
pub mod registry;

pub use host::{AgentHandle, Command, Event, Permission, Start, Update, start};
