//! The Artifacts page on the daemon (ADR 0075): every artifact in the
//! threads here, the person's pins, and archiving by the thread's owner.

use std::collections::BTreeSet;

use silt_log::PageOp;
use silt_proto::page::Level;
use silt_proto::view::{ArtifactListed, artifact_listing, latest_artifacts};
use silt_types::ArtifactId;

use crate::core::{Core, CoreError};
use crate::design::{read_capped, write_atomically};

/// Where this machine keeps the person's pins, as `thread/artifact`.
const PINS: &str = "artifact-pins.json";
/// The page document saying whether the thread's owner archived the artifact.
pub(crate) const ARCHIVED: &str = "_meta/archived";

impl Core {
    /// Every artifact in the threads this machine has, as the page lists them.
    pub(crate) fn artifact_listing(&self) -> Vec<ArtifactListed> {
        let pins = self.artifact_pins();
        let mut rows = Vec::new();
        for summary in self.thread_summaries() {
            let Ok(thread) = self.find_thread(&summary.id) else { continue };
            let entries = self.history_entries(thread);
            for (artifact, time_ms) in latest_artifacts(entries.iter().map(|(_, entry)| entry)) {
                let archived = artifact.id.parse::<ArtifactId>().is_ok_and(|id| {
                    self.page_docs(self.threads[&thread].family, thread, id).get(ARCHIVED) == Some(&serde_json::Value::Bool(true))
                });
                rows.push(ArtifactListed {
                    pinned: pins.contains(&format!("{}/{}", summary.id, artifact.id)),
                    thread: summary.id.clone(),
                    thread_title: summary.title.clone(),
                    repo: summary.repo.clone(),
                    mine: summary.owner.is_empty() || summary.owner == self.own_user(thread),
                    owner: summary.owner.clone(),
                    artifact,
                    time_ms,
                    archived,
                });
            }
        }
        artifact_listing(rows)
    }

    fn artifact_pins(&self) -> BTreeSet<String> {
        read_capped(&self.data_dir.join(PINS), 1 << 20).and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default()
    }

    /// The listing with the pins the person's servers keep merged in, so a
    /// pin made on another machine shows here too.
    pub(crate) fn artifact_listing_merged(
        &self,
    ) -> impl Future<Output = Result<Vec<ArtifactListed>, CoreError>> + Send + 'static + use<> {
        let rows = self.artifact_listing();
        let remotes = self.remotes.clone();
        async move {
            let mut pinned = std::collections::HashSet::new();
            for remote in remotes {
                let asked = tokio::time::timeout(std::time::Duration::from_secs(3), remote.artifact_pins()).await;
                if let Ok(Ok(pins)) = asked {
                    pinned.extend(pins.into_iter().map(|pin| (pin.thread, pin.artifact)));
                }
            }
            let rows = rows
                .into_iter()
                .map(|mut row| {
                    row.pinned |= pinned.contains(&(row.thread.clone(), row.artifact.id.clone()));
                    row
                })
                .collect();
            Ok(artifact_listing(rows))
        }
    }

    /// Sends a pin to the server the thread syncs with, so it follows the
    /// person; kept here either way.
    pub(crate) fn share_pin(
        &self,
        thread: &str,
        artifact: &str,
        pinned: bool,
    ) -> impl Future<Output = Result<&'static str, CoreError>> + Send + 'static + use<> {
        let remote = self.find_thread(thread).ok().and_then(|id| self.remote_for(id)).cloned();
        let pin = silt_proto::view::ArtifactPin { thread: thread.to_owned(), artifact: artifact.to_owned(), pinned };
        async move {
            if let Some(remote) = remote {
                remote
                    .pin_artifact(&pin)
                    .await
                    .map_err(|error| CoreError::Invalid(format!("kept here, not on the server: {error}")))?;
            }
            Ok("saved")
        }
    }

    /// Pins an artifact to the top of the page for this machine's person, or unpins it.
    pub(crate) fn pin_artifact(&self, thread: &str, artifact: &str, pinned: bool) -> Result<(), CoreError> {
        let thread = self.find_thread(thread)?;
        let _: ArtifactId = artifact.parse().map_err(|_| CoreError::Invalid(format!("{artifact} isn't an artifact id")))?;
        let mut pins = self.artifact_pins();
        let key = format!("{thread}/{artifact}");
        if pinned {
            pins.insert(key);
        } else {
            pins.remove(&key);
        }
        let json = serde_json::to_vec(&pins).map_err(|error| CoreError::Invalid(error.to_string()))?;
        write_atomically(&self.data_dir.join(PINS), &json)
    }

    /// Hides an artifact from the Artifacts tab and page for everyone, or
    /// shows it again: the thread's owner's to do, kept in the thread's
    /// page data so it hands off, forks and syncs (ADR 0075).
    pub(crate) async fn archive_artifact(&mut self, thread: &str, artifact: &str, archived: bool) -> Result<(), CoreError> {
        let thread = self.find_thread(thread)?;
        let id: ArtifactId = artifact.parse().map_err(|_| CoreError::Invalid(format!("{artifact} isn't an artifact id")))?;
        if self.page_level(thread) < Level::Owner {
            return Err(CoreError::Invalid("only the thread's owner archives its artifacts".into()));
        }
        let family = self.threads[&thread].family;
        self.append_page(family, thread, PageOp::Set { artifact: id, path: ARCHIVED.into(), value: archived.to_string() }).await
    }
}
