//! `silt harness` (ADR 0014): Silt's own coding agent, an ACP agent on
//! Anthropic's API or any OpenAI-compatible one, with file tools confined
//! to the checkout and commands run in `silt sandbox`.

mod agent;
pub mod model;
pub mod tools;

pub use agent::{Config, serve};
