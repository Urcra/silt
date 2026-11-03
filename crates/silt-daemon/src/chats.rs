//! Chats: threads that belong to no repository, each in a folder of its own
//! that is a small repository of its own (ADR 0059).

use std::path::{Path, PathBuf};

use silt_types::ThreadId;

use crate::core::{Core, CoreError, now_ms};
pub use crate::protocol::ChatSettings;
#[cfg(test)]
use crate::protocol::Preset;

const SETTINGS_FILE: &str = "chats.json";

/// The chat settings kept in `data_dir`, or the defaults.
pub fn settings(data_dir: &Path) -> ChatSettings {
    std::fs::read(data_dir.join(SETTINGS_FILE)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

pub(crate) fn save_settings(data_dir: &Path, settings: &ChatSettings) -> Result<(), CoreError> {
    if let Some(dir) = &settings.dir {
        if !dir.is_absolute() {
            return Err(CoreError::Invalid("the chats folder must be an absolute path".into()));
        }
        check_dir(dir)?;
    }
    let bytes = serde_json::to_vec_pretty(settings).map_err(std::io::Error::other)?;
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(data_dir.join(SETTINGS_FILE), bytes)?;
    Ok(())
}

/// Where this machine puts chats' folders.
pub fn chats_dir(data_dir: &Path) -> PathBuf {
    settings(data_dir).dir.unwrap_or_else(|| data_dir.join("chats"))
}

/// Makes a new chat's folder, named from the day and the first `words` of
/// what it's about, and returns it.
pub fn new_folder(data_dir: &Path, words: &str) -> Result<PathBuf, CoreError> {
    let dir = chats_dir(data_dir);
    check_dir(&dir)?;
    let suffix = format!("{:06x}", uuid::Uuid::new_v4().as_u128() & 0xff_ffff);
    let path = dir.join(folder_name(words, now_ms(), &suffix));
    let repo = silt_git::Repo::init_chat(&path)?;
    Ok(repo.root().to_owned())
}

/// The chats folder may not be in a repository that would take in the
/// chats' files: anywhere outside one, or where it ignores them.
fn check_dir(dir: &Path) -> Result<(), CoreError> {
    let existing = dir.ancestors().find(|ancestor| ancestor.exists()).unwrap_or(dir);
    let Ok(outer) = silt_git::Repo::discover(existing) else {
        return Ok(());
    };
    // As a folder, which it may not be yet, so a pattern such as `chats/` matches it.
    let as_folder = format!("{}/", dir.to_string_lossy().trim_end_matches(['/', '\\']));
    if outer.is_chat() || outer.git(&["check-ignore", "-q", &as_folder], None).is_ok() {
        return Ok(());
    }
    Err(CoreError::Invalid(format!(
        "the chats folder {} is inside the git repository at {}, which would take in the chats' files; pick a folder outside any repository",
        dir.display(),
        outer.root().display()
    )))
}

/// `2026-11-03-which-motor-driver-for-a-1a2b3c`: the day, up to five words
/// of what the chat is about, and a suffix that keeps names apart.
fn folder_name(words: &str, now_ms: i64, suffix: &str) -> String {
    let (year, month, day) = silt_agents::time::civil_from_days(now_ms.div_euclid(86_400_000));
    let slug: Vec<String> = words
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(5)
        .map(str::to_ascii_lowercase)
        .collect();
    let mut slug = slug.join("-");
    slug.truncate(40);
    let slug = slug.trim_end_matches('-');
    let slug = if slug.is_empty() { "chat" } else { slug };
    format!("{year:04}-{month:02}-{day:02}-{slug}-{suffix}")
}

impl Core {
    pub(crate) fn is_chat(&self, thread: ThreadId) -> bool {
        self.threads.get(&thread).is_some_and(|record| silt_types::is_chat(&record.repo))
    }

    /// The chat's folder on this machine, when it has one: the folder it
    /// was recorded in, still a chat's repository.
    pub(crate) fn chat_folder_here(&self, thread: ThreadId) -> Option<PathBuf> {
        let root = &self.threads.get(&thread)?.repo_root;
        if root.as_os_str().is_empty() {
            return None;
        }
        let repo = silt_git::Repo::discover(root).ok()?;
        (repo.is_chat() && repo.root() == root).then(|| root.clone())
    }

    /// A new folder for `thread` to continue or fork in, named after what
    /// it's about.
    pub(crate) fn new_chat_folder_for(&self, thread: ThreadId) -> Result<PathBuf, CoreError> {
        let words = self
            .threads
            .get(&thread)
            .and_then(|record| self.families.get(&record.family))
            .and_then(|family| {
                let history: Vec<&silt_log::Delta> = family.history(thread).into_iter().map(|item| item.delta).collect();
                crate::views::title(&history)
            })
            .unwrap_or_default();
        new_folder(&self.data_dir, &words)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chats_folder_is_named_by_its_day_and_first_words() {
        let day = 1_793_664_000_000; // 2026-11-03
        assert_eq!(
            folder_name("Which motor driver for a 12 V stepper?", day, "1a2b3c"),
            "2026-11-03-which-motor-driver-for-a-1a2b3c"
        );
        assert_eq!(folder_name("¿?", day, "1a2b3c"), "2026-11-03-chat-1a2b3c");
        assert_eq!(folder_name("../../etc/passwd", day, "1a2b3c"), "2026-11-03-etc-passwd-1a2b3c", "only plain words");
    }

    #[test]
    fn the_chats_folder_stays_out_of_repositories_that_would_take_in_its_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(check_dir(&dir.path().join("chats")).is_ok(), "outside any repository");
        silt_git::Repo::init(&dir.path().join("code")).expect("inits");
        assert!(check_dir(&dir.path().join("code/chats")).is_err(), "inside one");
        std::fs::write(dir.path().join("code/.gitignore"), "chats/\n").expect("writes");
        assert!(check_dir(&dir.path().join("code/chats")).is_ok(), "ignored by it");
    }

    #[test]
    fn a_new_chat_starts_with_full_access_in_the_data_dir() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert_eq!(settings(dir.path()).preset, Preset::FullAccess);
        assert_eq!(chats_dir(dir.path()), dir.path().join("chats"));
        let folder = new_folder(dir.path(), "Which sensor?").expect("makes it");
        assert!(folder.file_name().expect("named").to_string_lossy().contains("-which-sensor-"));
        assert!(silt_git::Repo::discover(&folder).expect("a repository").is_chat());
    }
}
