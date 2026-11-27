//! Identifiers shared by every Silt crate.
//!
//! Every unit of change (one inserted character, one deleted character, one
//! conversation record) has a [`DeltaId`]: the replica that made it plus a
//! Lamport timestamp from the history-wide clock. An event touching `n` units
//! occupies the contiguous span `[lamport, lamport + n)` of its actor.

mod hex_serde;

mod artifact;
mod cloud;
mod entity;
mod frontier;
mod hex;
mod id;
mod oid;

pub use artifact::ArtifactId;
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

/// The projects of a person's memory (ADR 0137): `silt:memory` for what
/// holds in every project, and `silt:memory/<project>` for a project's.
pub const MEMORY: &str = "silt:memory";

/// The project of the memory of `project`, or of what holds everywhere.
pub fn memory_project(project: Option<&str>) -> String {
    match project {
        Some(project) => format!("{MEMORY}/{project}"),
        None => MEMORY.to_owned(),
    }
}

/// Whose memory a memory project holds: `Some(None)` for everywhere's,
/// `Some(Some(project))` for a project's, `None` when it isn't one.
pub fn remembered(project: &str) -> Option<Option<&str>> {
    if project == MEMORY {
        return Some(None);
    }
    project.strip_prefix(MEMORY)?.strip_prefix('/').filter(|rest| !rest.is_empty()).map(Some)
}

/// Whether `project` holds a person's memory.
pub fn is_memory(project: &str) -> bool {
    remembered(project).is_some()
}

/// Whether `project`'s threads are the person's own, in no org: chats,
/// until shared, and memory, always.
pub fn is_personal(project: &str) -> bool {
    is_chat(project) || is_memory(project)
}

#[cfg(test)]
mod memory_tests {
    use super::*;

    #[test]
    fn memory_projects_name_whose_memory_they_hold() {
        assert_eq!(memory_project(Some("github.com/acme/silt")), "silt:memory/github.com/acme/silt");
        assert_eq!(remembered("silt:memory/github.com/acme/silt"), Some(Some("github.com/acme/silt")));
        assert_eq!(remembered(MEMORY), Some(None));
        for other in ["silt:memoryx", "silt:memory/", "silt:chats", "github.com/acme/silt"] {
            assert_eq!(remembered(other), None, "{other}");
        }
        assert!(is_personal(CHATS) && is_personal(MEMORY) && !is_personal("github.com/acme/silt"));
    }
}
