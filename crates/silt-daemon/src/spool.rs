//! Hooks that couldn't get their turn at writing in time, say while a
//! stopped daemon holds the capture lock (ADR 0018). Rather than lose its
//! turn mark, such a hook leaves its input here, and the next writer that
//! records its session captures it first.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::core::{Core, CoreError};

#[derive(Debug, Serialize, Deserialize)]
struct Spooled {
    event: String,
    input: serde_json::Value,
}

fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("spooled-hooks")
}

/// Leaves a hook to be captured later.
pub(crate) fn spool(data_dir: &Path, event: &str, input: &serde_json::Value) -> io::Result<()> {
    let dir = dir(data_dir);
    fs::create_dir_all(&dir)?;
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_nanos());
    let name = format!("{nanos:024}-{}.json", std::process::id());
    let spooled = Spooled { event: event.to_owned(), input: input.clone() };
    // Written aside and moved in, so a reader never sees half of it.
    let partial = dir.join(format!(".{name}"));
    fs::write(&partial, serde_json::to_vec(&spooled)?)?;
    fs::rename(partial, dir.join(name))
}

/// The hooks left, oldest first.
fn spooled(data_dir: &Path) -> Vec<(PathBuf, Spooled)> {
    let Ok(entries) = fs::read_dir(dir(data_dir)) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "json"))
        .filter(|path| !path.file_name().is_some_and(|name| name.to_string_lossy().starts_with('.')))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| match fs::read(&path).map(|bytes| serde_json::from_slice::<Spooled>(&bytes)) {
            Ok(Ok(spooled)) => Some((path, spooled)),
            Ok(Err(error)) => {
                warn!(%error, path = %path.display(), "dropping a spooled hook that can't be read");
                let _ = fs::remove_file(&path);
                None
            }
            Err(_) => None,
        })
        .collect()
}

impl Core {
    /// Captures the spooled hooks of sessions this core records, oldest
    /// first. The daemon drops those of sessions nobody records.
    pub(crate) async fn capture_spooled(&mut self) -> Result<(), CoreError> {
        for (path, spooled) in spooled(&self.data_dir) {
            let input: silt_agents::claude::HookInput = match serde_json::from_value(spooled.input) {
                Ok(input) => input,
                Err(error) => {
                    warn!(%error, "dropping a spooled hook that isn't one");
                    let _ = fs::remove_file(&path);
                    continue;
                }
            };
            if self.session(&input.session_id).is_none() {
                if self.resident {
                    let _ = fs::remove_file(&path);
                }
                continue;
            }
            // Removed even when capturing fails, so one bad hook can't stop
            // every later one.
            let captured = self.capture(&spooled.event, &input).await;
            let _ = fs::remove_file(&path);
            captured?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spooled_hooks_come_back_oldest_first() {
        let data = tempfile::tempdir().expect("temp dir");
        assert!(spooled(data.path()).is_empty());
        for event in ["PreToolUse", "PostToolUse"] {
            spool(data.path(), event, &serde_json::json!({ "session_id": "s" })).expect("spools");
        }
        let events: Vec<String> = spooled(data.path()).into_iter().map(|(_, spooled)| spooled.event).collect();
        assert_eq!(events, ["PreToolUse", "PostToolUse"]);
    }
}
