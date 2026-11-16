//! Rules for artifacts that both serving modes and the agent's tools apply
//! alike (ADR 0068, ADR 0073).

use serde::{Deserialize, Serialize};

/// The most edits one call takes.
pub const MAX_EDITS: usize = 64;
/// The largest artifact a version keeps, in bytes.
pub const ARTIFACT_BYTES: u64 = 10 << 20;
/// The most files a version has beside its page (ADR 0074).
pub const FILES: usize = 255;
/// The most one file of a version may be, in bytes.
pub const FILE_BYTES: u64 = 16 << 20;
/// The most a version's page and files may be together, in bytes.
pub const VERSION_BYTES: u64 = 64 << 20;
/// The most an asset someone adds to a page may be, in bytes.
pub const ASSET_BYTES: u64 = 20 << 20;

/// A file of a version, beside its page, served at the frame's own address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ArtifactFile {
    pub blob: String,
    pub size: u64,
    pub media_type: String,
}

/// Whether `path` may name a version's file: relative, `/`-separated, of
/// plain segments, none of them hidden or a step up, and not `index.html`
/// or under `_`, which the page and its assets take.
pub fn file_path_ok(path: &str) -> bool {
    let segments: Vec<&str> = path.split('/').collect();
    (1..=200).contains(&path.len())
        && segments.len() <= 8
        && path != "index.html"
        && !path.starts_with('_')
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && !segment.starts_with('.')
                && segment.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        })
}

/// What a version's file is served as, by its extension; none for a type
/// a frame has no use for.
pub fn file_media_type(path: &str) -> Option<&'static str> {
    let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match extension.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "csv" => "text/csv; charset=utf-8",
        "tsv" => "text/tab-separated-values; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        "md" => "text/markdown; charset=utf-8",
        "xml" => "application/xml",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "wasm" => "application/wasm",
        "pdf" => "application/pdf",
        _ => return None,
    })
}

/// What an asset someone adds may be: an image, a video, a PDF, a font,
/// or text, never a page or a script that would run.
pub fn asset_media_type_ok(media_type: &str) -> bool {
    let base = media_type.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    matches!(
        base.as_str(),
        "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "image/avif"
            | "image/svg+xml"
            | "video/mp4"
            | "video/webm"
            | "audio/mpeg"
            | "audio/wav"
            | "audio/ogg"
            | "application/pdf"
            | "font/woff"
            | "font/woff2"
            | "text/csv"
            | "text/markdown"
            | "text/plain"
            | "application/json"
    )
}

/// The most the files people add to one artifact's page take on a machine,
/// and by default on a server, whose `artifact_assets_mib` may change it.
pub const PAGE_ASSETS_BYTES: u64 = 256 << 20;

/// The runtime contract a page's `silt` is: the newest, and every one this
/// Silt still serves. A version keeps the one it was made with until a new
/// version asks for another (ADR 0072).
pub const CONTRACT: &str = "1";
pub const CONTRACTS: &[&str] = &["1"];

/// The contract a new version gets: the one it asks for (`latest` for the
/// newest), else its last version's, else the newest.
pub fn contract_for(asked: Option<&str>, previous: Option<&str>) -> Result<String, String> {
    match asked {
        Some("latest") => Ok(CONTRACT.to_owned()),
        Some(asked) if CONTRACTS.contains(&asked) => Ok(asked.to_owned()),
        Some(asked) => Err(format!("this Silt serves contract {} only, not {asked}", CONTRACTS.join(", "))),
        None => Ok(previous.filter(|kept| CONTRACTS.contains(kept)).unwrap_or(CONTRACT).to_owned()),
    }
}

/// The most of a page's question to `sample`, in bytes.
pub const SAMPLE_BYTES: usize = 256 * 1024;
/// The most images one question shows, each at most [`SAMPLE_IMAGE_BYTES`]
/// of base64, as the model takes them.
pub const SAMPLE_IMAGES: usize = 4;
pub const SAMPLE_IMAGE_BYTES: usize = 5 << 20;

/// An image a page's question shows the agent, which the page declared
/// `{sample: {images: true}}` for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SampleImage {
    pub media_type: String,
    /// Base64.
    pub data: String,
}

/// Whether `images` are ones a question may show: few, small, and of the
/// kinds the model reads.
pub fn sample_images(images: &[SampleImage]) -> Result<(), String> {
    use base64::Engine as _;
    if images.len() > SAMPLE_IMAGES {
        return Err(format!("a question shows at most {SAMPLE_IMAGES} images"));
    }
    for image in images {
        if !matches!(image.media_type.as_str(), "image/png" | "image/jpeg" | "image/gif" | "image/webp") {
            return Err(format!("{} isn't an image the agent reads; use PNG, JPEG, GIF or WebP", image.media_type));
        }
        if image.data.len() > SAMPLE_IMAGE_BYTES {
            return Err(format!("an image is at most {} MiB of base64", SAMPLE_IMAGE_BYTES >> 20));
        }
        if base64::engine::general_purpose::STANDARD.decode(&image.data).is_err() {
            return Err("an image's data isn't base64".into());
        }
    }
    Ok(())
}

/// The most tools a page offers one question.
pub const SAMPLE_TOOLS: usize = 16;

/// A tool a page offers the agent answering its question: the page runs
/// it, in the viewer's browser, when the agent calls it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SampleTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// A JSON Schema for its input; any object when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub input_schema: Option<serde_json::Value>,
}

/// Whether `tools` are ones a question may offer: few, named plainly, and
/// described briefly.
pub fn sample_tools(tools: &[SampleTool]) -> Result<(), String> {
    if tools.len() > SAMPLE_TOOLS {
        return Err(format!("a question offers at most {SAMPLE_TOOLS} tools"));
    }
    let mut seen = std::collections::BTreeSet::new();
    for tool in tools {
        let plain =
            (1..=64).contains(&tool.name.len()) && tool.name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if !plain {
            return Err(format!("{:?} isn't a tool's name: 1 to 64 letters, digits, _ and -", tool.name));
        }
        if !seen.insert(&tool.name) {
            return Err(format!("two tools are named {}", tool.name));
        }
        if tool.description.chars().count() > 1000 {
            return Err(format!("{}'s description is over 1,000 characters", tool.name));
        }
        if let Some(schema) = &tool.input_schema
            && (!schema.is_object() || schema.to_string().len() > 16 * 1024)
        {
            return Err(format!("{}'s input schema is an object of at most 16 KiB", tool.name));
        }
    }
    Ok(())
}

/// Which model answers a page's question: a tier, or a model the page
/// names; Claude Code's own default when neither.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SampleModel {
    /// `quick`, `default` or `complex`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_tier: Option<String>,
    /// A Claude model's id, such as `claude-sonnet-5-5`; it wins over the tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl SampleModel {
    /// What print mode's `--model` is given, if anything.
    pub fn resolved(&self) -> Result<Option<String>, String> {
        if let Some(model) = &self.model {
            let named = model.starts_with("claude-")
                && model.len() <= 64
                && model.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.');
            return if named { Ok(Some(model.clone())) } else { Err(format!("{model:?} isn't a Claude model's id")) };
        }
        match self.model_tier.as_deref() {
            None | Some("default") => Ok(None),
            Some("quick") => Ok(Some("haiku".into())),
            Some("complex") => Ok(Some("opus".into())),
            Some(other) => Err(format!("{other} isn't a model tier: quick, default or complex")),
        }
    }
}

/// Whether a version's declaration lets its page show the agent images.
pub fn sample_takes_images(declared: &Capabilities) -> bool {
    declared.get("sample").and_then(|sample| sample.get("images")).and_then(serde_json::Value::as_bool) == Some(true)
}

/// What a page's `sample` call asks, as one message to an agent's print
/// mode: a string as it is, or a conversation ending on the person's turn,
/// each turn marked.
pub fn sample_question(input: &serde_json::Value) -> Result<String, String> {
    use serde_json::Value;
    let question = match input {
        Value::String(text) => text.clone(),
        Value::Array(turns) => {
            let mut said = Vec::new();
            for turn in turns {
                let role = turn.get("role").and_then(Value::as_str).ok_or("each turn has a role")?;
                let content = turn.get("content").and_then(Value::as_str).ok_or("each turn's content is text")?;
                let tag = match role {
                    "user" => "user",
                    "assistant" => "assistant",
                    _ => return Err(format!("{role} isn't a role")),
                };
                said.push((tag, content));
            }
            if said.last().is_none_or(|(role, _)| *role != "user") {
                return Err("a conversation ends on the user's turn".into());
            }
            let earlier: String =
                said[..said.len() - 1].iter().map(|(role, content)| format!("<{role}>\n{content}\n</{role}>\n")).collect();
            let last = said[said.len() - 1].1;
            if earlier.is_empty() {
                last.to_owned()
            } else {
                format!("The conversation so far:\n{earlier}\nAnswer the user's latest message:\n{last}")
            }
        }
        _ => return Err("a question is text or a list of turns".into()),
    };
    if question.trim().is_empty() {
        return Err("there's nothing to ask".into());
    }
    if question.len() > SAMPLE_BYTES {
        return Err(format!("a question is at most {} KiB", SAMPLE_BYTES >> 10));
    }
    Ok(question)
}

/// The longest note on a version, in characters.
pub const NOTE_CHARS: usize = 200;
/// The longest description of an artifact, in characters.
pub const DESCRIPTION_CHARS: usize = 300;

/// The capabilities a page may declare (ADR 0072 and the ADRs after it).
pub const CAPABILITIES: &[&str] =
    &["artifact", "assets", "comments", "db", "dictation", "downloads", "edit", "files", "mcp", "room", "sample", "tts", "user"];

/// The capabilities a version declares, by name, each with its config.
pub type Capabilities = std::collections::BTreeMap<String, serde_json::Value>;

/// The largest declaration, as JSON.
const CAPABILITIES_BYTES: usize = 16 * 1024;

/// A declaration checked: known names, each with an object or `true` as
/// its config, `true` read as `{}`.
pub fn checked_capabilities(declared: &serde_json::Map<String, serde_json::Value>) -> Result<Capabilities, String> {
    if serde_json::to_string(declared).map_or(true, |text| text.len() > CAPABILITIES_BYTES) {
        return Err(format!("a capabilities declaration is at most {} KiB", CAPABILITIES_BYTES / 1024));
    }
    let mut checked = Capabilities::new();
    for (name, config) in declared {
        if !CAPABILITIES.contains(&name.as_str()) {
            return Err(format!("{name} isn't a capability; they're {}", CAPABILITIES.join(", ")));
        }
        let config = match config {
            serde_json::Value::Bool(true) => serde_json::json!({}),
            serde_json::Value::Object(_) => config.clone(),
            _ => return Err(format!("{name}'s config is an object, or true")),
        };
        checked.insert(name.clone(), config);
    }
    Ok(checked)
}

/// What `declared` says of `capability`, as text that compares alike
/// however its keys were ordered. A viewer's grant holds while the latest
/// version declares the same, so a version that widens what a page may do
/// asks again.
pub fn declaration(declared: Option<&Capabilities>, capability: &str) -> Option<String> {
    fn sorted(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let mut keys: Vec<_> = map.keys().collect();
                keys.sort();
                serde_json::Value::Object(keys.into_iter().map(|key| (key.clone(), sorted(&map[key]))).collect())
            }
            serde_json::Value::Array(items) => serde_json::Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    declared?.get(capability).map(|config| sorted(config).to_string())
}

/// What a version of `kind` declares, with what its kind always has: a doc,
/// a sheet or a whiteboard is edited in place (`edit`), and a whiteboard
/// keeps its items in page data, its cursors in a room and its images as
/// assets (ADRs 0077, 0081). A version declaring them keeps its own.
pub fn with_kind_defaults(kind: &str, declared: Option<Capabilities>) -> Option<Capabilities> {
    if !crate::edited::is_edited(kind) {
        return declared;
    }
    let mut declared = declared.unwrap_or_default();
    declared.entry("edit".to_owned()).or_insert_with(|| serde_json::json!({}));
    if kind == "whiteboard" {
        declared.entry("db".to_owned()).or_insert_with(|| serde_json::json!({}));
        declared.entry("room".to_owned()).or_insert_with(|| serde_json::json!({}));
        declared.entry("assets".to_owned()).or_insert_with(|| serde_json::json!({"upload": "comment"}));
    }
    Some(declared)
}

/// The capabilities that run a page's own code with what's the person's:
/// their agent, their connectors, their thread's files. A grant of one
/// holds for the page it was given to, so a new page asks again.
pub const PAGE_BOUND: &[&str] = &["sample", "mcp", "files"];

/// What a grant of `capability` is given for, by `version`: what it
/// declares of it, and for [`PAGE_BOUND`] capabilities, its page too.
pub fn grant_basis(version: Option<&crate::view::ArtifactView>, capability: &str) -> Option<String> {
    let version = version?;
    let declared = declaration(version.capabilities.as_ref(), capability)?;
    Some(if PAGE_BOUND.contains(&capability) { format!("{declared}@{}", version.blob) } else { declared })
}

#[cfg(test)]
mod kind_tests {
    #[test]
    fn a_whiteboard_always_takes_images_and_a_page_keeps_its_own() {
        let old: super::Capabilities = serde_json::from_str(r#"{"edit": {}, "db": {}, "room": {}}"#).expect("json");
        let now = super::with_kind_defaults("whiteboard", Some(old)).expect("some");
        assert_eq!(now["assets"], serde_json::json!({"upload": "comment"}));
        let sheet: super::Capabilities = serde_json::from_str(r#"{"edit": {"level": "collaborator"}}"#).expect("json");
        assert_eq!(
            super::with_kind_defaults("sheet", Some(sheet)).expect("some")["edit"],
            serde_json::json!({"level": "collaborator"})
        );
        assert_eq!(super::with_kind_defaults("html", None), None);
    }
}

/// One change to a version's source: `old` occurs exactly once, and
/// becomes `new`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Edit {
    pub old: String,
    pub new: String,
    /// The version's file it changes, rather than its page (ADR 0074).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

/// Why an edit couldn't be made; nothing was changed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("no edits were given")]
    None,
    #[error("at most {MAX_EDITS} edits go in one call")]
    TooMany,
    #[error("edit {index}: its old text is empty")]
    Empty { index: usize },
    #[error("edit {index}: its old text isn't in the source")]
    Missing { index: usize },
    #[error("edit {index}: its old text occurs {count} times; give more of it so it occurs once")]
    Ambiguous { index: usize, count: usize },
}

/// `source` with `edits` made in order, each on the result of the ones
/// before it, all or none. Edits are numbered from 1 in errors.
pub fn apply_edits(source: &str, edits: &[Edit]) -> Result<String, EditError> {
    if edits.is_empty() {
        return Err(EditError::None);
    }
    if edits.len() > MAX_EDITS {
        return Err(EditError::TooMany);
    }
    let mut text = source.to_owned();
    for (at, edit) in edits.iter().enumerate() {
        let index = at + 1;
        if edit.old.is_empty() {
            return Err(EditError::Empty { index });
        }
        let count = text.matches(edit.old.as_str()).count();
        match count {
            0 => return Err(EditError::Missing { index }),
            1 => text = text.replacen(edit.old.as_str(), &edit.new, 1),
            count => return Err(EditError::Ambiguous { index, count }),
        }
    }
    Ok(text)
}

/// A note or description trimmed, empty as none, refused past `limit`
/// characters.
pub fn short_text(text: Option<&str>, limit: usize, what: &str) -> Result<Option<String>, String> {
    let Some(text) = text.map(str::trim).filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    if text.chars().count() > limit {
        return Err(format!("{what} is at most {limit} characters"));
    }
    Ok(Some(text.to_owned()))
}

#[cfg(test)]
mod file_tests {
    use super::*;

    #[test]
    fn a_versions_files_have_plain_relative_paths_of_types_a_frame_uses() {
        for good in ["data.json", "css/site.css", "img/a-b_c.v2.png", "fonts/Inter.woff2"] {
            assert!(file_path_ok(good) && file_media_type(good).is_some(), "{good}");
        }
        for bad in
            ["", "/etc/passwd", "../x.css", "a/../b.css", ".hidden/x.js", "index.html", "_assets/x.png", "a//b.css", "a\\b.css"]
        {
            assert!(!file_path_ok(bad), "{bad}");
        }
        assert_eq!(file_media_type("x.exe"), None);
        assert_eq!(file_media_type("Photo.JPG"), Some("image/jpeg"));
        assert!(asset_media_type_ok("image/png") && asset_media_type_ok("text/csv; charset=utf-8"));
        assert!(!asset_media_type_ok("text/html") && !asset_media_type_ok("text/javascript"));
    }
}

#[cfg(test)]
mod sample_tests {
    use super::sample_question;
    use serde_json::json;

    #[test]
    fn a_question_is_text_or_turns_ending_on_the_users() {
        assert_eq!(sample_question(&json!("Summarise this")).as_deref(), Ok("Summarise this"));
        let turns = json!([{"role": "user", "content": "Hi"}, {"role": "assistant", "content": "Hello"}, {"role": "user", "content": "And?"}]);
        let asked = sample_question(&turns).expect("turns");
        assert!(asked.contains("<assistant>\nHello\n</assistant>") && asked.ends_with("And?"));
        assert!(sample_question(&json!([{"role": "assistant", "content": "Hello"}])).is_err());
        assert!(sample_question(&json!([{"role": "system", "content": "x"}])).is_err());
        assert!(sample_question(&json!("  ")).is_err());
        assert!(sample_question(&json!(3)).is_err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_declaration_reads_alike_whatever_its_order() {
        let one: Capabilities =
            serde_json::from_str(r#"{"mcp":{"servers":[{"tools":["a"],"server":"x"}],"z":1}}"#).expect("json");
        let two: Capabilities =
            serde_json::from_str(r#"{"mcp":{"z":1,"servers":[{"server":"x","tools":["a"]}]}}"#).expect("json");
        assert_eq!(declaration(Some(&one), "mcp"), declaration(Some(&two), "mcp"));
        let wider: Capabilities =
            serde_json::from_str(r#"{"mcp":{"z":1,"servers":[{"server":"x","tools":["a","b"]}]}}"#).expect("json");
        assert_ne!(declaration(Some(&one), "mcp"), declaration(Some(&wider), "mcp"));
        assert_eq!(declaration(Some(&one), "sample"), None);
    }

    #[test]
    fn a_version_keeps_its_contract_until_it_asks_for_another() {
        assert_eq!(contract_for(None, None).as_deref(), Ok(CONTRACT));
        assert_eq!(contract_for(None, Some("1")).as_deref(), Ok("1"));
        assert_eq!(contract_for(Some("latest"), Some("1")).as_deref(), Ok(CONTRACT));
        assert!(contract_for(Some("9"), None).is_err());
    }

    #[test]
    fn a_question_names_a_tier_or_a_model() {
        let asked = |tier: Option<&str>, model: Option<&str>| {
            SampleModel { model_tier: tier.map(str::to_owned), model: model.map(str::to_owned) }.resolved()
        };
        assert_eq!(asked(None, None), Ok(None));
        assert_eq!(asked(Some("quick"), None), Ok(Some("haiku".into())));
        assert_eq!(asked(Some("complex"), Some("claude-sonnet-5-5")), Ok(Some("claude-sonnet-5-5".into())), "a named model wins");
        assert!(asked(Some("huge"), None).is_err());
        assert!(asked(None, Some("--dangerously-skip")).is_err(), "never an option");
    }

    #[test]
    fn a_question_offers_few_plainly_named_tools() {
        let tool = |name: &str| SampleTool { name: name.into(), description: String::new(), input_schema: None };
        assert_eq!(sample_tools(&[tool("lookup"), tool("add_row")]), Ok(()));
        assert!(sample_tools(&[tool("bad name")]).is_err());
        assert!(sample_tools(&[tool("twice"), tool("twice")]).is_err());
        let listed = SampleTool { input_schema: Some(serde_json::json!(["not", "an", "object"])), ..tool("odd") };
        assert!(sample_tools(&[listed]).is_err());
        assert!(sample_tools(&(0..=SAMPLE_TOOLS).map(|at| tool(&format!("t{at}"))).collect::<Vec<_>>()).is_err());
    }

    #[test]
    fn a_question_shows_only_images_its_page_declared_and_the_agent_reads() {
        let declared = |sample: serde_json::Value| Capabilities::from([("sample".to_owned(), sample)]);
        assert!(sample_takes_images(&declared(serde_json::json!({"images": true}))));
        assert!(!sample_takes_images(&declared(serde_json::json!({}))));
        let image = |media_type: &str, data: &str| SampleImage { media_type: media_type.into(), data: data.into() };
        assert_eq!(sample_images(&[image("image/png", "aGk=")]), Ok(()));
        assert!(sample_images(&[image("image/svg+xml", "aGk=")]).is_err());
        assert!(sample_images(&[image("image/png", "not base64!")]).is_err());
        assert!(sample_images(&vec![image("image/png", "aGk="); SAMPLE_IMAGES + 1]).is_err());
    }

    fn edit(old: &str, new: &str) -> Edit {
        Edit { old: old.into(), new: new.into(), file: None }
    }

    #[test]
    fn edits_apply_in_order_all_or_none() {
        let source = "<h1>Rescan</h1><p>v1</p>";
        assert_eq!(
            apply_edits(source, &[edit("v1", "v2"), edit("<h1>Rescan", "<h1>Rescan benchmark")]).as_deref(),
            Ok("<h1>Rescan benchmark</h1><p>v2</p>")
        );
        assert_eq!(
            apply_edits(source, &[edit("v1", "v2"), edit("v1", "v3")]),
            Err(EditError::Missing { index: 2 }),
            "an edit sees the ones before it"
        );
        assert_eq!(apply_edits("a a", &[edit("a", "b")]), Err(EditError::Ambiguous { index: 1, count: 2 }));
        assert_eq!(apply_edits(source, &[edit("", "x")]), Err(EditError::Empty { index: 1 }));
        assert_eq!(apply_edits(source, &[]), Err(EditError::None));
        assert_eq!(apply_edits(source, &vec![edit("v1", "v1"); MAX_EDITS + 1]), Err(EditError::TooMany));
        assert_eq!(apply_edits(source, &[edit("</p>", "")]).as_deref(), Ok("<h1>Rescan</h1><p>v1"), "an empty new text deletes");
    }

    #[test]
    fn a_declaration_names_known_capabilities_with_objects() {
        let declared = serde_json::json!({"db": {"rules": []}, "downloads": true});
        let checked = checked_capabilities(declared.as_object().expect("an object")).expect("checked");
        assert_eq!(checked["downloads"], serde_json::json!({}));
        assert_eq!(checked["db"], serde_json::json!({"rules": []}));
        assert!(checked_capabilities(serde_json::json!({"camera": {}}).as_object().expect("an object")).is_err());
        assert!(checked_capabilities(serde_json::json!({"db": 3}).as_object().expect("an object")).is_err());
    }

    #[test]
    fn notes_are_trimmed_and_bounded() {
        assert_eq!(short_text(Some("  Darker axis  "), NOTE_CHARS, "a note"), Ok(Some("Darker axis".into())));
        assert_eq!(short_text(Some("   "), NOTE_CHARS, "a note"), Ok(None));
        assert_eq!(short_text(None, NOTE_CHARS, "a note"), Ok(None));
        assert!(short_text(Some(&"x".repeat(NOTE_CHARS + 1)), NOTE_CHARS, "a note").is_err());
    }
}
