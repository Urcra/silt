//! Adapters for agent CLIs (ADR 0007). The session files an agent writes are
//! the record; this crate finds them, reads them incrementally, and derives
//! an agent-independent view of the conversation from their lines.

pub mod claude;
mod entry;
mod tail;

pub use entry::Entry;
pub use tail::complete_lines;
