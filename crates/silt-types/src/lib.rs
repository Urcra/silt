//! Identifiers shared by every Silt crate.
//!
//! Every unit of change (one inserted character, one deleted character, one
//! conversation record) has a [`DeltaId`]: the replica that made it plus a
//! Lamport timestamp from the history-wide clock. An event touching `n` units
//! occupies the contiguous span `[lamport, lamport + n)` of its actor.

mod hex_serde;

mod cloud;
mod entity;
mod frontier;
mod hex;
mod id;
mod oid;

pub use cloud::{CloudSessionId, ParseCloudSessionError};
pub use entity::{BlobHash, DeviceId, FamilyId, ThreadId};
pub use frontier::{Frontier, Version};
pub use hex::ParseIdError;
pub use id::{ActorId, DeltaId, DeltaSpan, Lamport};
pub use oid::GitOid;

/// The project of chats, threads that belong to no repository (ADR 0059).
/// No remote or folder normalizes to a name with `:` before its first `/`,
/// so it can't name a repository.
pub const CHATS: &str = "silt:chats";

/// Whether `project` is [`CHATS`].
pub fn is_chat(project: &str) -> bool {
    project == CHATS
}
