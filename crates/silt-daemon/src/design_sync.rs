//! The person's own design systems, synced through each server they use
//! (ADR 0071). Each system's folder under the data dir is the truth on a
//! machine; an index of what each held when last synced tells a change made
//! here from one that came in, and the copy changed last wins.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};
use silt_proto::DesignSystemCopy;
use tracing::warn;

use crate::core::CoreError;
use crate::design::{
    ASSETS_DIR, COMPONENTS_CSS, COMPONENTS_DIR, COVER, PERSONAL_DIR, asset_files, component_files, component_guides,
};
use crate::remote::Remote;

/// What each system held when last synced, here.
const INDEX: &str = "design-systems.sync.json";
const README_BYTES: u64 = 256 << 10;
const TOKENS_BYTES: u64 = 512 << 10;
const FONT_BYTES: u64 = 1 << 20;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Known {
    hash: String,
    at_ms: i64,
    #[serde(default)]
    deleted: bool,
    /// Changed here and not yet taken by a server.
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
        warn!(%error, "could not keep what the design systems held");
    }
}

fn read_capped(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let meta = fs::metadata(path).ok()?;
    (meta.is_file() && meta.len() <= limit).then(|| fs::read(path).ok()).flatten()
}

/// A system's folder as a copy, without its time: its README, tokens and
/// plainly named font files.
fn read_folder(name: &str, dir: &Path) -> Option<DesignSystemCopy> {
    let tokens = String::from_utf8(read_capped(&dir.join("tokens.json"), TOKENS_BYTES)?).ok()?;
    let readme =
        read_capped(&dir.join("README.md"), README_BYTES).and_then(|bytes| String::from_utf8(bytes).ok()).unwrap_or_default();
    let mut fonts = BTreeMap::new();
    if let Ok(entries) = fs::read_dir(dir.join("fonts")) {
        for entry in entries.flatten() {
            let file = entry.file_name().to_string_lossy().into_owned();
            if let Some(bytes) = read_capped(&entry.path(), FONT_BYTES) {
                fonts.insert(file, BASE64.encode(bytes));
            }
        }
    }
    let components_css = read_capped(&dir.join(COMPONENTS_CSS), silt_proto::DESIGN_COMPONENTS_CSS_BYTES as u64)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default();
    let components = component_files(&dir.join(COMPONENTS_DIR));
    let component_guides = component_guides(&dir.join(COMPONENTS_DIR), components.keys());
    let cover = read_capped(&dir.join(COVER), silt_proto::DESIGN_COVER_BYTES as u64)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default();
    let (kept, _) = silt_design::assets(asset_files(&dir.join(ASSETS_DIR)));
    let assets = kept
        .into_iter()
        .filter_map(|(path, _, _)| {
            let bytes = read_capped(&dir.join(ASSETS_DIR).join(&path), silt_proto::DESIGN_ASSET_BYTES as u64)?;
            Some((path, BASE64.encode(bytes)))
        })
        .collect();
    let copy = DesignSystemCopy {
        name: name.to_owned(),
        at_ms: 0,
        deleted: false,
        readme,
        tokens,
        fonts,
        components_css,
        components,
        component_guides,
        cover,
        assets,
    };
    copy.valid().then_some(copy)
}

fn hash(copy: &DesignSystemCopy) -> String {
    let mut digest = blake3::Hasher::new();
    for part in [copy.readme.as_str(), copy.tokens.as_str()] {
        digest.update(part.as_bytes());
        digest.update(&[0]);
    }
    for (file, data) in &copy.fonts {
        digest.update(file.as_bytes());
        digest.update(&[0]);
        digest.update(data.as_bytes());
        digest.update(&[0]);
    }
    // Only a system with components digests them, so one without keeps the
    // hash it had before they existed and isn't sent again.
    if !copy.components_css.is_empty() || !copy.components.is_empty() {
        digest.update(b"components\0");
        digest.update(copy.components_css.as_bytes());
        digest.update(&[0]);
        for (name, html) in &copy.components {
            digest.update(name.as_bytes());
            digest.update(&[0]);
            digest.update(html.as_bytes());
            digest.update(&[0]);
        }
    }
    // The same for what ADR 0088 added: a system without them keeps its hash.
    for (part, entries) in [("guides", &copy.component_guides), ("assets", &copy.assets)] {
        if !entries.is_empty() {
            digest.update(part.as_bytes());
            digest.update(&[0]);
            for (name, text) in entries {
                digest.update(name.as_bytes());
                digest.update(&[0]);
                digest.update(text.as_bytes());
                digest.update(&[0]);
            }
        }
    }
    if !copy.cover.is_empty() {
        digest.update(b"cover\0");
        digest.update(copy.cover.as_bytes());
    }
    digest.finalize().to_hex()[..32].to_owned()
}

/// The systems in the person's folder now, by name.
fn local(data_dir: &Path) -> BTreeMap<String, DesignSystemCopy> {
    let mut out = BTreeMap::new();
    let Ok(entries) = fs::read_dir(data_dir.join(PERSONAL_DIR)) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(copy) = read_folder(&name, &entry.path()) {
            out.insert(name, copy);
        }
    }
    out
}

/// Notes what changed here since the last sync, a new or edited system
/// stamped `now` and a removed one as a tombstone, and answers the copies
/// still to send.
fn changes(data_dir: &Path, index: &mut Index, now: i64) -> Vec<DesignSystemCopy> {
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
            Some(copy) if !known.deleted => DesignSystemCopy { at_ms: known.at_ms, ..copy.clone() },
            _ => DesignSystemCopy {
                name: name.clone(),
                at_ms: known.at_ms,
                deleted: true,
                readme: String::new(),
                tokens: String::new(),
                fonts: BTreeMap::new(),
                components_css: String::new(),
                components: BTreeMap::new(),
                component_guides: BTreeMap::new(),
                cover: String::new(),
                assets: BTreeMap::new(),
            },
        })
        .collect()
}

/// Copies the system in `from` to the person's own as `name`, holding what
/// a synced one holds, unless they have one by that name already.
pub(crate) fn keep_as_personal(data_dir: &Path, name: &str, from: &Path) -> Result<(), CoreError> {
    let root = data_dir.join(PERSONAL_DIR);
    let dir = root.join(name);
    if dir.exists() {
        return Err(CoreError::Invalid(format!("you have a design system {name} already")));
    }
    let copy = read_folder(name, from)
        .ok_or_else(|| CoreError::Invalid(format!("{name} has no readable tokens, or holds more than a design system may")))?;
    fs::create_dir_all(&root)?;
    write_folder(&root, &dir, &copy)?;
    Ok(())
}

/// Takes a copy from a server if it changed later than what's here,
/// writing or removing the system's folder.
fn take(data_dir: &Path, index: &mut Index, copy: &DesignSystemCopy) -> bool {
    if !copy.valid() || index.get(&copy.name).is_some_and(|known| known.at_ms >= copy.at_ms) {
        return false;
    }
    let root = data_dir.join(PERSONAL_DIR);
    let dir = root.join(&copy.name);
    let written = if copy.deleted { remove(&dir) } else { write_folder(&root, &dir, copy) };
    if let Err(error) = written {
        warn!(%error, system = copy.name, "could not take a design system from the server");
        return false;
    }
    let hash = if copy.deleted { String::new() } else { hash(copy) };
    index.insert(copy.name.clone(), Known { hash, at_ms: copy.at_ms, deleted: copy.deleted, pending: false });
    true
}

fn remove(dir: &Path) -> std::io::Result<()> {
    match fs::remove_dir_all(dir) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn write_folder(root: &Path, dir: &Path, copy: &DesignSystemCopy) -> std::io::Result<()> {
    let staging: PathBuf = root.join(format!(".{}.incoming", copy.name));
    remove(&staging)?;
    fs::create_dir_all(staging.join("fonts"))?;
    fs::write(staging.join("tokens.json"), &copy.tokens)?;
    fs::write(staging.join("README.md"), &copy.readme)?;
    for (file, data) in &copy.fonts {
        let bytes = BASE64.decode(data).map_err(std::io::Error::other)?;
        fs::write(staging.join("fonts").join(file), bytes)?;
    }
    if !copy.components_css.is_empty() {
        fs::write(staging.join(COMPONENTS_CSS), &copy.components_css)?;
    }
    if !copy.components.is_empty() {
        fs::create_dir_all(staging.join(COMPONENTS_DIR))?;
        for (name, html) in &copy.components {
            fs::write(staging.join(COMPONENTS_DIR).join(format!("{name}.html")), html)?;
        }
        for (name, guide) in &copy.component_guides {
            fs::write(staging.join(COMPONENTS_DIR).join(format!("{name}.md")), guide)?;
        }
    }
    if !copy.cover.is_empty() {
        fs::write(staging.join(COVER), &copy.cover)?;
    }
    for (path, data) in &copy.assets {
        let bytes = BASE64.decode(data).map_err(std::io::Error::other)?;
        let file = staging.join(ASSETS_DIR).join(path);
        if let Some(group) = file.parent() {
            fs::create_dir_all(group)?;
        }
        fs::write(file, bytes)?;
    }
    remove(dir)?;
    fs::rename(&staging, dir)
}

/// Sends what changed here to every server, and takes what changed on them.
pub(crate) async fn sync(data_dir: PathBuf, remotes: Vec<Remote>, now: i64) {
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
    let mut taken = Vec::new();
    for remote in &remotes {
        let server = remote.config.url.clone();
        let mut all_sent = true;
        for copy in &sending {
            if let Err(error) = remote.put_design_system(copy).await {
                warn!(%error, server, "could not send a design system to the server");
                all_sent = false;
            }
        }
        if all_sent {
            taken.extend(sending.iter().map(|copy| (copy.name.clone(), copy.at_ms)));
        }
        match remote.design_systems().await {
            Ok(copies) => fetched.extend(copies),
            Err(error) => warn!(%error, server, "could not fetch design systems from the server"),
        }
    }
    let _ = tokio::task::spawn_blocking(move || {
        // A change made since this sync began stays pending.
        let mut latest = read_index(&data_dir);
        for (name, at_ms) in taken {
            if let Some(known) = latest.get_mut(&name).filter(|known| known.at_ms == at_ms) {
                known.pending = false;
            }
        }
        for copy in &fetched {
            take(&data_dir, &mut latest, copy);
        }
        write_index(&data_dir, &latest);
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system(data: &Path, name: &str, tokens: &str) {
        let dir = data.join(PERSONAL_DIR).join(name);
        fs::create_dir_all(dir.join("fonts")).expect("dir");
        fs::write(dir.join("tokens.json"), tokens).expect("tokens");
        fs::write(dir.join("README.md"), "Use bg.").expect("readme");
        fs::write(dir.join("fonts/acme.woff2"), b"wOF2").expect("font");
    }

    #[test]
    fn a_change_here_is_sent_once_and_a_removal_as_a_tombstone() {
        let data = tempfile::tempdir().expect("temp dir");
        system(data.path(), "acme", r#"{"name":"Acme"}"#);
        let mut index = Index::new();
        let sent = changes(data.path(), &mut index, 100);
        assert_eq!((sent.len(), sent[0].at_ms, sent[0].fonts.len()), (1, 100, 1));
        index.get_mut("acme").expect("known").pending = false;
        assert!(changes(data.path(), &mut index, 200).is_empty(), "nothing changed");
        system(data.path(), "acme", r#"{"name":"Acme 2"}"#);
        assert_eq!(changes(data.path(), &mut index, 300)[0].at_ms, 300);
        index.get_mut("acme").expect("known").pending = false;
        fs::remove_dir_all(data.path().join(PERSONAL_DIR).join("acme")).expect("remove");
        let gone = changes(data.path(), &mut index, 400);
        assert!(gone[0].deleted && gone[0].at_ms == 400, "{gone:?}");
    }

    #[test]
    fn a_later_copy_from_the_server_wins_and_an_earlier_one_doesnt() {
        let data = tempfile::tempdir().expect("temp dir");
        system(data.path(), "acme", r#"{"name":"Mine"}"#);
        let mut index = Index::new();
        changes(data.path(), &mut index, 500);
        let mut theirs = read_folder("acme", &data.path().join(PERSONAL_DIR).join("acme")).expect("copy");
        theirs.tokens = r#"{"name":"Theirs"}"#.into();
        theirs.at_ms = 400;
        assert!(!take(data.path(), &mut index, &theirs), "older than what's here");
        theirs.at_ms = 600;
        assert!(take(data.path(), &mut index, &theirs));
        let tokens = fs::read_to_string(data.path().join(PERSONAL_DIR).join("acme/tokens.json")).expect("tokens");
        assert_eq!(tokens, r#"{"name":"Theirs"}"#);
        assert!(data.path().join(PERSONAL_DIR).join("acme/fonts/acme.woff2").is_file());
        assert!(changes(data.path(), &mut index, 700).is_empty(), "what came in isn't sent back");

        let gone =
            DesignSystemCopy { deleted: true, at_ms: 800, tokens: String::new(), fonts: BTreeMap::new(), ..theirs.clone() };
        assert!(take(data.path(), &mut index, &gone));
        assert!(!data.path().join(PERSONAL_DIR).join("acme").exists());

        let hostile = DesignSystemCopy { name: "../../etc".into(), at_ms: 900, ..theirs.clone() };
        assert!(!take(data.path(), &mut index, &hostile));
        let mut bad_font = theirs.clone();
        bad_font.at_ms = 900;
        bad_font.fonts.insert("../x".into(), "AA==".into());
        assert!(!take(data.path(), &mut index, &bad_font));
    }

    #[test]
    fn components_travel_with_their_system_and_only_plainly_named() {
        let here = tempfile::tempdir().expect("temp dir");
        system(here.path(), "acme", r#"{"name":"Acme"}"#);
        let mut index = Index::new();
        let without = changes(here.path(), &mut index, 100).remove(0);
        let dir = here.path().join(PERSONAL_DIR).join("acme");
        fs::create_dir_all(dir.join(COMPONENTS_DIR)).expect("dir");
        fs::write(dir.join(COMPONENTS_CSS), ".a-row { height: 28px; }").expect("css");
        fs::write(dir.join(COMPONENTS_DIR).join("row.html"), "<a class=\"a-row\">x</a>").expect("row");
        let with = changes(here.path(), &mut index, 200).remove(0);
        assert_ne!(hash(&without), hash(&with), "adding components is a change");
        assert_eq!(with.components_css, ".a-row { height: 28px; }");
        assert_eq!(with.components.keys().collect::<Vec<_>>(), ["row"]);

        let there = tempfile::tempdir().expect("temp dir");
        let mut theirs = Index::new();
        assert!(take(there.path(), &mut theirs, &with));
        let taken = there.path().join(PERSONAL_DIR).join("acme");
        assert_eq!(fs::read_to_string(taken.join(COMPONENTS_CSS)).expect("css"), ".a-row { height: 28px; }");
        assert_eq!(fs::read_to_string(taken.join(COMPONENTS_DIR).join("row.html")).expect("row"), "<a class=\"a-row\">x</a>");

        let mut hostile = DesignSystemCopy { at_ms: 300, ..with.clone() };
        hostile.components.insert("../../escape".into(), "<b></b>".into());
        assert!(!take(there.path(), &mut theirs, &hostile));
        let mut huge = DesignSystemCopy { at_ms: 300, ..with.clone() };
        huge.components.insert("big".into(), "x".repeat(silt_proto::DESIGN_COMPONENT_BYTES + 1));
        assert!(!take(there.path(), &mut theirs, &huge));
        let mut escaping = DesignSystemCopy { at_ms: 300, ..with };
        escaping.assets.insert("../../../escape.svg".into(), "PHN2Zy8+".into());
        assert!(!take(there.path(), &mut theirs, &escaping), "an asset's path stays under assets/");
    }

    #[test]
    fn guidelines_a_cover_and_assets_travel_too() {
        let here = tempfile::tempdir().expect("temp dir");
        system(here.path(), "acme", r#"{"name":"Acme"}"#);
        let mut index = Index::new();
        let without = changes(here.path(), &mut index, 100).remove(0);
        let dir = here.path().join(PERSONAL_DIR).join("acme");
        fs::create_dir_all(dir.join(COMPONENTS_DIR)).expect("dir");
        fs::write(dir.join(COMPONENTS_DIR).join("row.html"), "<a>x</a>").expect("row");
        fs::write(dir.join(COMPONENTS_DIR).join("row.md"), "A list row.").expect("guide");
        fs::write(dir.join(COVER), "<svg></svg>").expect("cover");
        fs::create_dir_all(dir.join(ASSETS_DIR).join("Logos")).expect("assets");
        fs::write(dir.join(ASSETS_DIR).join("Logos/mark.svg"), "<svg/>").expect("mark");
        fs::write(dir.join(ASSETS_DIR).join("Logos/notes.txt"), "not kept").expect("notes");
        let with = changes(here.path(), &mut index, 200).remove(0);
        assert_ne!(hash(&without), hash(&with));
        assert_eq!(with.component_guides.get("row").map(String::as_str), Some("A list row."));
        assert_eq!(with.cover, "<svg></svg>");
        assert_eq!(with.assets.keys().collect::<Vec<_>>(), ["Logos/mark.svg"]);

        let there = tempfile::tempdir().expect("temp dir");
        assert!(take(there.path(), &mut Index::new(), &with));
        let taken = there.path().join(PERSONAL_DIR).join("acme");
        assert_eq!(fs::read_to_string(taken.join(COMPONENTS_DIR).join("row.md")).expect("guide"), "A list row.");
        assert_eq!(fs::read_to_string(taken.join(COVER)).expect("cover"), "<svg></svg>");
        assert_eq!(fs::read(taken.join(ASSETS_DIR).join("Logos/mark.svg")).expect("mark"), b"<svg/>");
    }

    #[test]
    fn the_wire_keeps_what_a_system_may_hold() {
        assert_eq!(silt_proto::DESIGN_COMPONENTS, silt_design::MAX_COMPONENTS);
        assert_eq!(silt_proto::DESIGN_COMPONENT_BYTES, silt_design::COMPONENT_BYTES);
        assert_eq!(silt_proto::DESIGN_COMPONENTS_CSS_BYTES, silt_design::COMPONENTS_CSS_BYTES);
        assert_eq!(silt_proto::DESIGN_COVER_BYTES, silt_design::COVER_BYTES);
        assert_eq!(silt_proto::DESIGN_ASSETS, silt_design::MAX_ASSETS);
        assert_eq!(silt_proto::DESIGN_ASSET_BYTES, silt_design::ASSET_BYTES);
    }
}
