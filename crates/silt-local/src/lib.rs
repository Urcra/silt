//! Silt's own agent, `local` (ADR 0066): an ACP agent on a model this
//! machine runs, reached through the daemon's model gateway.

mod agent;
mod conversation;
mod model;
mod stream;

pub use agent::{CHAT, serve};
pub use conversation::{Conversation, Entry, Message, Role, is_session_id};
