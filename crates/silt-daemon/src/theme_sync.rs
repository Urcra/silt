//! The person's own themes, synced through each server they use
//! (ADR 0099), as design systems are: each theme's file or folder under
//! `themes/` is the truth on a machine, an index of what each held when last
//! synced tells a change made here from one that came in, and the copy
//! changed last wins.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};
use silt_proto::ThemeCopy;
use silt_theme::font::{FACE_BYTES, FontFile};
use tracing::warn;

use crate::remote::Remote;
use crate::theme::{FOLDER_FILE, sources, themes_dir};

/// What each theme held when last synced, here.
const INDEX: &str = "themes.sync.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Known {
    hash: String,
    at_ms: i64,
    #[serde(default)]
    deleted: bool,
    /// Changed here and not yet taken by every server.
    #[serde(default)]
    pending: bool,
}

type Index = BTreeMap<String, Known>;

fn read_index(data_dir: &Path) -> Index {
    fs::read(data_dir.join(INDEX)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

fn write_index(data_dir: &Path, index: &Index) {
    let path = data_dir.join(INDEX);
    let staging = path.with_extension("json.tmp");
    let written = serde_json::to_vec_pretty(index)
        .map_err(std::io::Error::other)
        .and_then(|json| fs::write(&staging, json))
        .and_then(|()| fs::rename(&staging, &path));
    if let Err(error) = written {
        warn!(%error, "could not keep what the themes held");
    }
}

fn hash(copy: &ThemeCopy) -> String {
    let mut digest = blake3::Hasher::new();
    digest.update(copy.json.as_bytes());
    for (file, data) in &copy.fonts {
        digest.update(b"\0font\0").update(file.as_bytes()).update(b"\0").update(data.as_bytes());
    }
    digest.finalize().to_hex()[..32].to_owned()
}

/// The fonts in a theme's folder, by name, in base64.
fn fonts_of(root: &Path) -> BTreeMap<String, String> {
    let Ok(entries) = fs::read_dir(root.join("fonts")) else { return BTreeMap::new() };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let file = FontFile::parse(&name).filter(|file| file.name() == name)?;
            let meta = entry.metadata().ok().filter(|meta| meta.is_file() && meta.len() <= FACE_BYTES)?;
            let bytes = fs::read(entry.path()).ok().filter(|bytes| bytes.len() as u64 == meta.len())?;
            file.check(&bytes).ok()?;
            Some((name, BASE64.encode(bytes)))
        })
        .collect()
}

/// The person's themes now, by name; one whose name or contents a copy
/// can't carry stays on this machine.
fn local(data_dir: &Path) -> BTreeMap<String, ThemeCopy> {
    let mut out = BTreeMap::new();
    for source in sources(data_dir) {
        let Ok(json) = fs::read_to_string(&source.json) else { continue };
        let fonts = source.root.as_deref().map(fonts_of).unwrap_or_default();
        let copy = ThemeCopy { name: source.name.clone(), at_ms: 0, deleted: false, json, fonts };
        if copy.valid() {
            out.insert(source.name, copy);
        }
    }
    out
}

/// Notes what changed here since the last sync, a new or edited theme
/// stamped `now` and a removed one as a tombstone, and answers the copies
/// still to send.
fn changes(data_dir: &Path, index: &mut Index, now: i64) -> Vec<ThemeCopy> {
    let here = local(data_dir);
    for (name, copy) in &here {
        let hash = hash(copy);
        let known = index.entry(name.clone()).or_default();
        if known.hash != hash || known.deleted {
            *known = Known { hash, at_ms: now.max(known.at_ms + 1), deleted: false, pending: true };
        }
    }
    for (name, known) in index.iter_mut() {
        if !here.contains_key(name) && !known.deleted {
            *known = Known { hash: String::new(), at_ms: now.max(known.at_ms + 1), deleted: true, pending: true };
        }
    }
    index
        .iter()
        .filter(|(_, known)| known.pending)
        .map(|(name, known)| match here.get(name) {
            Some(copy) if !known.deleted => ThemeCopy { at_ms: known.at_ms, ..copy.clone() },
            _ => ThemeCopy { name: name.clone(), at_ms: known.at_ms, deleted: true, json: String::new(), fonts: BTreeMap::new() },
        })
        .collect()
}

fn remove(path: &Path) -> std::io::Result<()> {
    let removed = if path.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) };
    match removed {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Writes a theme as a file, or as a folder when it ships fonts, through a
/// staging name, and removes the other shape.
fn write(root: &Path, copy: &ThemeCopy) -> std::io::Result<()> {
    fs::create_dir_all(root)?;
    let (file, folder) = (root.join(format!("{}.json", copy.name)), root.join(&copy.name));
    if copy.fonts.is_empty() {
        let staging = root.join(format!(".{}.json.incoming", copy.name));
        fs::write(&staging, &copy.json)?;
        remove(&folder)?;
        return fs::rename(&staging, &file);
    }
    let staging: PathBuf = root.join(format!(".{}.incoming", copy.name));
    remove(&staging)?;
    fs::create_dir_all(staging.join("fonts"))?;
    fs::write(staging.join(FOLDER_FILE), &copy.json)?;
    for (name, data) in &copy.fonts {
        let bytes = BASE64.decode(data).map_err(std::io::Error::other)?;
        fs::write(staging.join("fonts").join(name), bytes)?;
    }
    remove(&file)?;
    remove(&folder)?;
    fs::rename(&staging, &folder)
}

/// Takes a copy from a server if it changed later than what's here.
fn take(data_dir: &Path, index: &mut Index, copy: &ThemeCopy) -> bool {
    if !copy.valid() || index.get(&copy.name).is_some_and(|known| known.at_ms >= copy.at_ms) {
        return false;
    }
    let root = themes_dir(data_dir);
    let written = if copy.deleted {
        remove(&root.join(format!("{}.json", copy.name))).and_then(|()| remove(&root.join(&copy.name)))
    } else {
        write(&root, copy)
    };
    if let Err(error) = written {
        warn!(%error, theme = copy.name, "could not take a theme from the server");
        return false;
    }
    let hash = if copy.deleted { String::new() } else { hash(copy) };
    index.insert(copy.name.clone(), Known { hash, at_ms: copy.at_ms, deleted: copy.deleted, pending: false });
    true
}

/// Sends what changed here to every server, and takes what changed on them;
/// whether it took anything, for the pages to show it.
pub(crate) async fn sync(data_dir: PathBuf, remotes: Vec<Remote>, now: i64) -> bool {
    let sending = {
        let data_dir = data_dir.clone();
        tokio::task::spawn_blocking(move || {
            let mut index = read_index(&data_dir);
            let sending = changes(&data_dir, &mut index, now);
            write_index(&data_dir, &index);
            sending
        })
        .await
        .unwrap_or_default()
    };
    let mut fetched = Vec::new();
    // A change counts as sent once every server has it, so one that failed
    // gets it next time.
    let mut all_sent = !remotes.is_empty();
    for remote in &remotes {
        let server = remote.config.url.clone();
        for copy in &sending {
            if let Err(error) = remote.put_theme(copy).await {
                warn!(%error, server, "could not send a theme to the server");
                all_sent = false;
            }
        }
        match remote.themes().await {
            Ok(copies) => fetched.extend(copies),
            Err(error) => warn!(%error, server, "could not fetch themes from the server"),
        }
    }
    let taken: Vec<(String, i64)> =
        if all_sent { sending.iter().map(|copy| (copy.name.clone(), copy.at_ms)).collect() } else { Vec::new() };
    tokio::task::spawn_blocking(move || {
        // A change made since this sync began stays pending.
        let mut latest = read_index(&data_dir);
        for (name, at_ms) in taken {
            if let Some(known) = latest.get_mut(&name).filter(|known| known.at_ms == at_ms) {
                known.pending = false;
            }
        }
        let mut changed = false;
        for copy in &fetched {
            changed |= take(&data_dir, &mut latest, copy);
        }
        write_index(&data_dir, &latest);
        changed
    })
    .await
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const THEME: &str = r##"{ "name": "Mine", "themes": [{ "id": "mine", "name": "Mine", "appearance": "light",
        "colors": { "bg": "#ffffff", "fg-1": "#111111", "accent": "#0055cc" } }] }"##;

    fn copy(name: &str, at_ms: i64, json: &str) -> ThemeCopy {
        ThemeCopy { name: name.into(), at_ms, deleted: false, json: json.into(), fonts: BTreeMap::new() }
    }

    #[test]
    fn a_change_here_is_sent_once_and_a_removal_as_a_tombstone() {
        let data = tempfile::tempdir().expect("temp dir");
        let root = themes_dir(data.path());
        fs::create_dir_all(&root).expect("themes");
        fs::write(root.join("mine.json"), THEME).expect("theme");
        fs::write(root.join("Not A Name.json"), THEME).expect("unnamed");
        let mut index = Index::new();
        let sent = changes(data.path(), &mut index, 100);
        assert_eq!(sent.iter().map(|copy| copy.name.as_str()).collect::<Vec<_>>(), ["mine"]);
        assert_eq!(sent[0].at_ms, 100);
        index.get_mut("mine").expect("known").pending = false;
        assert!(changes(data.path(), &mut index, 200).is_empty(), "nothing changed");

        fs::remove_file(root.join("mine.json")).expect("removed");
        let gone = changes(data.path(), &mut index, 300);
        assert_eq!(gone.len(), 1);
        assert!(gone[0].deleted && gone[0].at_ms == 300);
    }

    #[test]
    fn a_later_copy_wins_an_earlier_one_doesnt_and_fonts_make_a_folder() {
        let data = tempfile::tempdir().expect("temp dir");
        let root = themes_dir(data.path());
        let mut index = Index::new();
        assert!(take(data.path(), &mut index, &copy("mine", 10, THEME)));
        assert_eq!(fs::read_to_string(root.join("mine.json")).expect("written"), THEME);
        assert!(!take(data.path(), &mut index, &copy("mine", 5, "older")), "an earlier copy is ignored");

        let mut fonted = copy("mine", 20, THEME);
        fonted.fonts.insert("sans.woff2".into(), BASE64.encode(b"wOF2 a font"));
        assert!(take(data.path(), &mut index, &fonted));
        assert!(!root.join("mine.json").exists(), "the file became a folder");
        assert_eq!(fs::read(root.join("mine/fonts/sans.woff2")).expect("font"), b"wOF2 a font");
        let again = local(data.path());
        assert_eq!(again["mine"].fonts, fonted.fonts, "read back as it came");

        for escaping in ["../escape", "mine/x", ".hidden"] {
            assert!(!take(data.path(), &mut index, &copy(escaping, 99, THEME)), "{escaping}");
        }
        let mut bad_font = copy("other", 99, THEME);
        bad_font.fonts.insert("../x.woff2".into(), BASE64.encode(b"wOF2"));
        assert!(!take(data.path(), &mut index, &bad_font));

        let tombstone = ThemeCopy { deleted: true, json: String::new(), fonts: BTreeMap::new(), ..copy("mine", 30, "") };
        assert!(take(data.path(), &mut index, &tombstone));
        assert!(!root.join("mine").exists() && !root.join("mine.json").exists());
    }
}
