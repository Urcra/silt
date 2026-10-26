//! Runs coding agents over the Agent Client Protocol for the web UI
//! (ADR 0005, ADR 0014): finding them in the ACP registry, starting one in
//! a checkout, and relaying its session to the daemon over channels.

mod host;
mod modes;
mod prompt;
pub mod registry;

pub use host::{
    AgentHandle, Command, Event, HostError, Permission, Question, QuestionChoice, QuestionField, Start, StopReason, Update, start,
};
pub use modes::{Choice, Modes, Setting, built_in, plan_mode};
pub use prompt::{Accepts, Attachment};
