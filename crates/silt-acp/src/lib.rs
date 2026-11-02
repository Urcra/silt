//! Runs coding agents over the Agent Client Protocol for the web UI
//! (ADR 0005, ADR 0014): finding them in the ACP registry, starting one in
//! a checkout, and relaying its session to the daemon over channels.

mod host;
mod modes;
mod processes;
mod prompt;
pub mod registry;
mod sign_in;
mod v1;
mod v2;

pub use host::{
    AgentHandle, Command, Event, HostError, Permission, Question, QuestionChoice, QuestionField, Start, StopReason, TaskNews,
    Update, start,
};
pub use modes::{Choice, Modes, Setting, SlashCommand, built_in, plan_mode, presents_plans};
pub use prompt::{Accepts, Attachment};
pub use sign_in::{SignInHow, SignInMethod};
