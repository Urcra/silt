//! A project's memory sent to Copilot Memory on GitHub, when the person
//! turns that on for the project (ADR 0137). Copilot keeps its memory on
//! GitHub with no API to read it back, so Silt only sends: what changed
//! since the last send, every few minutes while the daemon runs, with
//! `copilot memories import`.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::Digest;
use silt_agents::copilot::{GitHubRepo, github_repo, memory_import, memory_import_launch};
use silt_agents::memory::{self, Entry};

use crate::core::{Core, CoreError, now_ms};
use crate::protocol::CopilotMemoryView;

/// How often what changed is sent.
const EVERY: Duration = Duration::from_secs(5 * 60);

const STATE: &str = "memory-copilot.json";

/// What was last sent of a project, by its clone.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Sent {
    pub(crate) sent_ms: Option<i64>,
    pub(crate) error: Option<String>,
    /// Each memory's file and the hash of what was sent of it.
    #[serde(default)]
    pub(crate) files: BTreeMap<String, String>,
}

#[derive(Default)]
pub(crate) struct CopilotMemory {
    last: Option<Instant>,
    sending: Arc<AtomicBool>,
}

/// One project's memories to send.
struct Batch {
    clone: PathBuf,
    repo: GitHubRepo,
    entries: Vec<Entry>,
}

impl Core {
    /// Sends what changed of the projects it's on for, when due and not
    /// under way, off the core.
    pub(crate) fn drive_copilot_memory(&mut self) {
        let due = self.copilot_memory.last.is_none_or(|last| last.elapsed() >= EVERY);
        if !due || self.copilot_memory.sending.load(Ordering::SeqCst) {
            return;
        }
        self.copilot_memory.last = Some(Instant::now());
        let on = memory::settings(&self.data_dir).copilot;
        if on.is_empty() {
            return;
        }
        let batches: Vec<Batch> = on.iter().filter_map(|clone| self.copilot_batch(clone)).collect();
        let data_dir = self.data_dir.clone();
        let offline = crate::network::allowed(&data_dir).offline;
        let sending = Arc::clone(&self.copilot_memory.sending);
        sending.store(true, Ordering::SeqCst);
        tokio::task::spawn_blocking(move || {
            let program = memory_import_launch(Path::new("")).program;
            for batch in batches {
                let mut state = read_state(&data_dir);
                let sent = state.remove(&batch.clone).unwrap_or_default();
                let sent = if offline {
                    Sent { error: Some("Silt is offline, so nothing is sent".into()), ..sent }
                } else {
                    send(&program, &data_dir, &batch, sent)
                };
                state.insert(batch.clone, sent);
                write_state(&data_dir, &state);
            }
            sending.store(false, Ordering::SeqCst);
        });
    }

    /// Turns sending the project's memory to Copilot on or off; on, it's
    /// sent at once.
    pub(crate) fn set_copilot_memory(&mut self, clone: &Path, on: bool) -> Result<&'static str, CoreError> {
        let (_, clone) = self.known_clone(clone)?;
        if on && self.copilot_repo(&clone).is_none() {
            return Err(CoreError::Invalid("Copilot Memory is for a project on GitHub, and this clone's origin isn't".into()));
        }
        let mut settings = memory::settings(&self.data_dir);
        if on {
            settings.copilot.insert(clone);
        } else {
            settings.copilot.remove(&clone);
        }
        memory::save_settings(&self.data_dir, &settings)
            .map_err(|error| CoreError::Invalid(format!("couldn't save memory.json: {error}")))?;
        if on {
            self.copilot_memory.last = None;
            self.drive_copilot_memory();
        }
        Ok(if on { "sending" } else { "saved" })
    }

    /// Sends what changed now, rather than at the next round.
    pub(crate) fn send_copilot_memory(&mut self) -> &'static str {
        self.copilot_memory.last = None;
        self.drive_copilot_memory();
        "sending"
    }

    /// Settings › Memory's row for a project's Copilot Memory.
    pub(crate) fn copilot_view(&self, clone: &Path) -> CopilotMemoryView {
        let repo = self.copilot_repo(clone);
        let on = memory::settings(&self.data_dir).copilot.contains(clone);
        let sent = read_state(&self.data_dir).remove(clone).unwrap_or_default();
        let waiting = match self.copilot_batch(clone) {
            Some(batch) => batch.entries.iter().filter(|entry| sent.files.get(&entry.file) != Some(&hash(&entry.text))).count(),
            None => 0,
        };
        CopilotMemoryView {
            on,
            repo: repo.map(|repo| format!("{}/{}/{}", repo.host, repo.owner, repo.name)),
            sent_ms: sent.sent_ms,
            error: sent.error,
            waiting,
            sending: self.copilot_memory.sending.load(Ordering::SeqCst),
        }
    }

    fn copilot_repo(&self, clone: &Path) -> Option<GitHubRepo> {
        let url = silt_git::Repo::discover(clone).ok()?.remote_url("origin").ok()??;
        github_repo(&url)
    }

    /// The project's memories Copilot is sent: Claude Code's store of it,
    /// which every agent shares, but what's about the person, which holds
    /// everywhere rather than in this repository.
    fn copilot_batch(&self, clone: &Path) -> Option<Batch> {
        let (repo_id, clone) = self.known_clone(clone).ok()?;
        let repo = self.copilot_repo(&clone)?;
        let store = self.project_memory_store(&repo_id, &clone)?;
        let entries =
            memory::read_indexed(&store).ok()?.into_iter().filter(|entry| entry.kind.as_deref() != Some("user")).collect();
        Some(Batch { clone, repo, entries })
    }
}

/// Sends what changed of `batch` since `sent` with the Copilot CLI
/// `program`, and gives what's been sent now.
fn send(program: &OsString, data_dir: &Path, batch: &Batch, sent: Sent) -> Sent {
    let changed: Vec<Entry> =
        batch.entries.iter().filter(|entry| sent.files.get(&entry.file) != Some(&hash(&entry.text))).cloned().collect();
    if changed.is_empty() {
        return Sent { error: None, ..sent };
    }
    let failed = |error: String| Sent { error: Some(error), ..sent.clone() };
    let scratch = data_dir.join("runs");
    if let Err(error) = std::fs::create_dir_all(&scratch) {
        return failed(error.to_string());
    }
    let file = scratch.join(format!("copilot-memory-{}-{}.jsonl", std::process::id(), now_ms()));
    if let Err(error) = std::fs::write(&file, memory_import(&changed, &batch.repo)) {
        return failed(error.to_string());
    }
    let mut launch = memory_import_launch(&file);
    launch.program = program.clone();
    let output = std::process::Command::new(&launch.program).args(&launch.args).current_dir(&batch.clone).output();
    let _ = std::fs::remove_file(&file);
    match output {
        Ok(output) if output.status.success() => {
            let mut files = sent.files.clone();
            for entry in &changed {
                files.insert(entry.file.clone(), hash(&entry.text));
            }
            Sent { sent_ms: Some(now_ms()), error: None, files }
        }
        Ok(output) => {
            let said = String::from_utf8_lossy(&output.stderr).trim().lines().last().unwrap_or_default().to_owned();
            failed(format!(
                "copilot memories import failed ({}){}",
                output.status,
                if said.is_empty() { String::new() } else { format!(": {said}") }
            ))
        }
        Err(error) => failed(format!("couldn't run the Copilot CLI: {error}")),
    }
}

fn hash(text: &str) -> String {
    sha2::Sha256::digest(text.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn read_state(data_dir: &Path) -> BTreeMap<PathBuf, Sent> {
    std::fs::read_to_string(data_dir.join(STATE)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

fn write_state(data_dir: &Path, state: &BTreeMap<PathBuf, Sent>) {
    if let Ok(json) = serde_json::to_vec_pretty(state)
        && let Err(error) = crate::design::write_atomically(&data_dir.join(STATE), &json)
    {
        tracing::warn!(%error, "couldn't keep what was sent to Copilot Memory");
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn only_what_changed_is_sent_and_a_failure_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("copilot");
        let log = dir.path().join("sent.jsonl");
        std::fs::write(
            &program,
            format!("#!/bin/sh\n[ -f {0}.fail ] && {{ echo 'not signed in' >&2; exit 3; }}\ncat \"$3\" >> {0}\n", log.display()),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let entry = |file: &str, text: &str| memory::parse_entry(file, text, None, None);
        let repo = GitHubRepo { host: "github.com".into(), owner: "acme".into(), name: "silt".into() };
        let mut batch = Batch {
            clone: dir.path().to_owned(),
            repo,
            entries: vec![entry("a.md", "# A\n\nfirst"), entry("b.md", "# B\n\nsecond")],
        };

        let sent = send(&program.clone().into_os_string(), dir.path(), &batch, Sent::default());
        assert_eq!((sent.files.len(), sent.error.clone()), (2, None));
        assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), 2);

        batch.entries[1] = entry("b.md", "# B\n\nchanged");
        let again = send(&program.clone().into_os_string(), dir.path(), &batch, sent.clone());
        assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), 3, "only b.md again");
        assert_ne!(again.files["b.md"], sent.files["b.md"]);

        batch.entries[0] = entry("a.md", "# A\n\nchanged too");
        std::fs::write(dir.path().join("sent.jsonl.fail"), "").unwrap();
        let failed = send(&program.into_os_string(), dir.path(), &batch, again.clone());
        assert!(failed.error.as_deref().is_some_and(|error| error.contains("not signed in")), "{failed:?}");
        assert_eq!(failed.files, again.files, "kept to send again");
    }
}
