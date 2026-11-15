//! Silt's own agent, `local` (ADR 0066): an ACP agent on a model this
//! machine runs, reached through the daemon's model gateway.

mod agent;
mod conversation;
mod model;
mod stream;
mod tools;

pub use agent::{CHAT, FIRST_MODE, MODES, serve};
pub use conversation::{Call, Conversation, Entry, Message, Role, is_session_id};
