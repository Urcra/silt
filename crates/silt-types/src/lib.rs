//! Identifiers shared by every Silt crate.
//!
//! Every unit of change (one inserted character, one deleted character, one
//! conversation record) has a [`DeltaId`]: the replica that made it plus a
//! Lamport timestamp from the history-wide clock. An event touching `n` units
//! occupies the contiguous span `[lamport, lamport + n)` of its actor.

mod entity;
mod frontier;
mod id;
mod oid;

pub use entity::{BlobHash, DeviceId, FamilyId, ParseBlobHashError, ThreadId};
pub use frontier::{Frontier, Version};
pub use id::{ActorId, DeltaId, DeltaSpan, Lamport, ParseActorIdError};
pub use oid::{GitOid, ParseGitOidError};
