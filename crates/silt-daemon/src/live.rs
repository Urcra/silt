//! What the web UI needs to work on a thread live (ADR 0014): the family's
//! deltas and git texts for its replica, deltas made in the browser, and a
//! fingerprint of the thread's state to stream changes by.

use std::path::Path;
use std::str::FromStr;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use silt_log::{Author, Delta};
use silt_types::GitOid;

use crate::core::{Core, CoreError, RepoBases};
use crate::protocol::{LiveState, ReplicaDeltas};

impl Core {
    /// The family's deltas after the first `after`, for the browser's
    /// replica, postcard-encoded and then in base64.
    pub(crate) async fn replica_deltas(&mut self, selector: &str, after: usize) -> Result<ReplicaDeltas, CoreError> {
        let thread = self.find_thread(selector)?;
        let family = self.threads[&thread].family;
        let _ = self.ensure_projected(family, Path::new("/")).await;
        let log = self.local_log(family);
        let after = after.min(log.len());
        let bytes = postcard::to_allocvec(&log[after..]).map_err(|error| CoreError::Invalid(error.to_string()))?;
        Ok(ReplicaDeltas {
            total: log.len(),
            deltas: BASE64.encode(bytes),
            writable: self.may_write(thread),
        })
    }

    /// The text of a git blob of the thread's repository.
    pub(crate) fn git_text(&self, selector: &str, oid: &str) -> Result<String, CoreError> {
        let thread = self.find_thread(selector)?;
        let oid = GitOid::from_str(oid).map_err(|error| CoreError::Invalid(error.to_string()))?;
        let repo = self.family_repo(self.threads[&thread].family)?;
        Ok(String::from_utf8_lossy(&repo.blob(oid)?).into_owned())
    }

    /// Takes edits made in the browser's replica: stores them, writes them
    /// to this machine's checkouts and lets sync carry them to the server.
    pub(crate) async fn receive_from_browser(&mut self, selector: &str, encoded: &str) -> Result<usize, CoreError> {
        let thread = self.find_thread(selector)?;
        if !self.may_write(thread) {
            return Err(CoreError::Invalid(
                "this thread belongs to someone else; ask its owner to share it with you".into(),
            ));
        }
        let bytes = BASE64.decode(encoded).map_err(|error| CoreError::Invalid(error.to_string()))?;
        let deltas: Vec<Delta> = postcard::from_bytes(&bytes).map_err(|error| CoreError::Invalid(error.to_string()))?;
        if deltas.iter().any(|delta| delta.thread != thread || delta.author != Author::User) {
            return Err(CoreError::Invalid("the browser can only add a person's edits to this thread".into()));
        }
        let family_id = self.threads[&thread].family;
        self.ensure_projected(family_id, Path::new("/")).await?;
        let repo = self.family_repo(family_id)?;
        let family = self.families.get_mut(&family_id).expect("projected above");
        let mut applied = Vec::new();
        for delta in deltas {
            if family.graph().contains(delta.span.first()) {
                continue;
            }
            family.apply(delta.clone(), &mut RepoBases(&repo))?;
            applied.push(delta);
        }
        self.store.append_deltas(family_id, &applied).await?;
        self.update_checkouts(family_id, &applied).await?;
        Ok(applied.len())
    }

    /// A fingerprint of everything the web UI shows live for the thread.
    pub(crate) fn live_state(&self, selector: &str) -> Result<LiveState, CoreError> {
        let thread = self.find_thread(selector)?;
        let family = self.threads[&thread].family;
        Ok(LiveState {
            deltas: self.local_log(family).len(),
            agent: self.agent_status(thread),
        })
    }
}
