//! Artifact types (ADR 0076): kinds whose content is JSON that Silt's own
//! viewers draw. Each is checked here before a version is kept, on the
//! daemon and the server alike, and described for the agent by a JSON
//! Schema and an example.

use serde::Deserialize;
use serde_json::{Value, json};

/// The kinds whose content is JSON checked here.
pub const TYPES: &[&str] = &["slides", "canvas", "data"];

/// The most slides a deck has, boards a canvas, or blocks a data page.
pub const PARTS: usize = 200;
/// The most HTML one slide or board holds, in bytes.
pub const PART_HTML_BYTES: usize = 512 * 1024;
/// The most points one chart has.
pub const POINTS: usize = 10_000;

pub fn is_type(kind: &str) -> bool {
    TYPES.contains(&kind)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Deck {
    aspect: Option<String>,
    #[serde(default)]
    style: Option<String>,
    slides: Vec<Slide>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Slide {
    title: String,
    html: String,
    #[serde(default)]
    notes: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Canvas {
    #[serde(default)]
    style: Option<String>,
    boards: Vec<Board>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Board {
    id: String,
    name: String,
    #[serde(default)]
    caption: Option<String>,
    width: u32,
    height: u32,
    #[serde(default)]
    x: Option<f64>,
    #[serde(default)]
    y: Option<f64>,
    html: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DataPage {
    title: String,
    #[serde(default)]
    summary: Option<String>,
    sources: Vec<Source>,
    blocks: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    id: String,
    name: String,
    kind: String,
    detail: String,
    at: String,
}

const CHARTS: &[&str] = &["line", "area", "bar", "dot", "scatter", "heatmap", "segmented"];
const SOURCE_KINDS: &[&str] = &["query", "file", "command", "agent", "manual"];

/// Whether `text` is a valid `kind` version, and why not.
pub fn validate(kind: &str, text: &str) -> Result<(), String> {
    let refused = |error: serde_json::Error| format!("a {kind} artifact's content doesn't fit its schema: {error}");
    match kind {
        "slides" => {
            let deck: Deck = serde_json::from_str(text).map_err(refused)?;
            if deck.aspect.as_deref().is_some_and(|aspect| aspect != "16:9" && aspect != "4:3") {
                return Err("a deck's aspect is 16:9 or 4:3".into());
            }
            parts(deck.slides.len(), "slide")?;
            for (at, slide) in deck.slides.iter().enumerate() {
                if slide.title.trim().is_empty() {
                    return Err(format!("slide {} has no title", at + 1));
                }
                html(&slide.html, &format!("slide {}", at + 1))?;
                if slide.notes.as_ref().is_some_and(|notes| notes.len() > 16 * 1024) {
                    return Err(format!("slide {}'s notes are over 16 KiB", at + 1));
                }
            }
            deck.style.as_deref().map_or(Ok(()), |style| html(style, "the deck's style"))
        }
        "canvas" => {
            let canvas: Canvas = serde_json::from_str(text).map_err(refused)?;
            parts(canvas.boards.len(), "board")?;
            let mut ids = std::collections::HashSet::new();
            for board in &canvas.boards {
                if !ids.insert(board.id.as_str()) || board.id.is_empty() || board.id.len() > 64 {
                    return Err(format!("board ids are unique, 1 to 64 characters: {}", board.id));
                }
                if !(1..=8192).contains(&board.width) || !(1..=8192).contains(&board.height) {
                    return Err(format!("board {}'s size is 1 to 8192 pixels each way", board.id));
                }
                if board.x.is_some_and(|x| !x.is_finite()) || board.y.is_some_and(|y| !y.is_finite()) {
                    return Err(format!("board {}'s position is a number", board.id));
                }
                let _ = (&board.name, &board.caption);
                html(&board.html, &format!("board {}", board.id))?;
            }
            canvas.style.as_deref().map_or(Ok(()), |style| html(style, "the canvas's style"))
        }
        "data" => {
            let page: DataPage = serde_json::from_str(text).map_err(refused)?;
            let _ = (&page.title, &page.summary);
            let mut sources = std::collections::HashSet::new();
            for source in &page.sources {
                if !SOURCE_KINDS.contains(&source.kind.as_str()) {
                    return Err(format!("source {}'s kind is query, file, command, agent or manual", source.id));
                }
                let _ = (&source.name, &source.detail, &source.at);
                sources.insert(source.id.as_str());
            }
            parts(page.blocks.len(), "block")?;
            for (at, block) in page.blocks.iter().enumerate() {
                data_block(block, &sources).map_err(|why| format!("block {}: {why}", at + 1))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn parts(count: usize, what: &str) -> Result<(), String> {
    if count == 0 {
        return Err(format!("at least one {what}"));
    }
    if count > PARTS {
        return Err(format!("at most {PARTS} of them, each a {what}"));
    }
    Ok(())
}

fn html(text: &str, what: &str) -> Result<(), String> {
    if text.len() > PART_HTML_BYTES { Err(format!("{what} is over {} KiB", PART_HTML_BYTES >> 10)) } else { Ok(()) }
}

fn data_block(block: &Value, sources: &std::collections::HashSet<&str>) -> Result<(), String> {
    let kind = block.get("type").and_then(Value::as_str).ok_or("each block has a type: stat, chart, table or text")?;
    let source = || -> Result<(), String> {
        let named = block.get("source").and_then(Value::as_str).ok_or("a block with numbers names its source")?;
        if sources.contains(named) { Ok(()) } else { Err(format!("no source {named}")) }
    };
    match kind {
        "text" => block.get("text").and_then(Value::as_str).map(|_| ()).ok_or_else(|| "a text block has text".into()),
        "stat" => {
            source()?;
            block.get("label").and_then(Value::as_str).ok_or("a stat has a label")?;
            block.get("value").filter(|value| value.is_number() || value.is_string()).ok_or("a stat has a value")?;
            Ok(())
        }
        "table" => {
            source()?;
            let columns = block.get("columns").and_then(Value::as_array).ok_or("a table has columns")?;
            let rows = block.get("rows").and_then(Value::as_array).ok_or("a table has rows")?;
            if rows.iter().any(|row| row.as_array().is_none_or(|cells| cells.len() != columns.len())) {
                return Err("each row has a cell per column".into());
            }
            Ok(())
        }
        "chart" => {
            source()?;
            let chart = block.get("chart").and_then(Value::as_str).ok_or("a chart names its form")?;
            if !CHARTS.contains(&chart) {
                return Err(format!("a chart is {}", CHARTS.join(", ")));
            }
            let series = block.get("series").and_then(Value::as_array).ok_or("a chart has series")?;
            if series.is_empty() || series.len() > 12 {
                return Err("a chart has 1 to 12 series".into());
            }
            let mut points = 0;
            for one in series {
                one.get("name").and_then(Value::as_str).ok_or("each series has a name")?;
                let data = one.get("points").and_then(Value::as_array).ok_or("each series has points")?;
                for point in data {
                    let pair = point.as_array().filter(|pair| pair.len() == 2).ok_or("a point is [x, y]")?;
                    if !pair[1].is_number() && !pair[1].is_null() {
                        return Err("a point's y is a number or null".into());
                    }
                }
                points += data.len();
            }
            if points > POINTS {
                return Err(format!("a chart has at most {POINTS} points"));
            }
            Ok(())
        }
        other => Err(format!("{other} isn't a block type")),
    }
}

/// `source` with RFC 6902 `patch` applied, for revising a type's content in
/// place; nothing changes when an operation fails.
pub fn apply_patch(source: &str, patch: &Value) -> Result<String, String> {
    let mut document: Value = serde_json::from_str(source).map_err(|_| "the latest version isn't JSON".to_owned())?;
    let patch: json_patch::Patch =
        serde_json::from_value(patch.clone()).map_err(|error| format!("that isn't a JSON Patch: {error}"))?;
    json_patch::patch(&mut document, &patch).map_err(|error| format!("the patch didn't apply: {error}"))?;
    serde_json::to_string_pretty(&document).map_err(|error| error.to_string())
}

/// A type's schema, as a JSON Schema the agent reads, and an example.
pub fn guide(kind: &str) -> Option<(Value, Value)> {
    let html = json!({"type": "string", "description": "A fragment of HTML; inline styles and scripts, no doctype"});
    Some(match kind {
        "slides" => (
            json!({
                "type": "object",
                "required": ["slides"],
                "additionalProperties": false,
                "properties": {
                    "aspect": {"enum": ["16:9", "4:3"]},
                    "style": {"type": "string", "description": "CSS every slide shares"},
                    "slides": {"type": "array", "minItems": 1, "maxItems": PARTS, "items": {
                        "type": "object", "required": ["title", "html"], "additionalProperties": false,
                        "properties": {"title": {"type": "string"}, "html": html, "notes": {"type": "string", "description": "What the presenter says"}}
                    }}
                }
            }),
            json!({"aspect": "16:9", "style": "h1{font-size:64px}", "slides": [
                {"title": "Rescans", "html": "<h1>Rescans got 3× faster</h1>", "notes": "Open with the headline."},
                {"title": "Why", "html": "<ul><li>One walk</li><li>No re-hash</li></ul>"}
            ]}),
        ),
        "canvas" => (
            json!({
                "type": "object",
                "required": ["boards"],
                "additionalProperties": false,
                "properties": {
                    "style": {"type": "string"},
                    "boards": {"type": "array", "minItems": 1, "maxItems": PARTS, "items": {
                        "type": "object", "required": ["id", "name", "width", "height", "html"], "additionalProperties": false,
                        "properties": {
                            "id": {"type": "string"}, "name": {"type": "string"}, "caption": {"type": "string"},
                            "width": {"type": "integer", "minimum": 1, "maximum": 8192},
                            "height": {"type": "integer", "minimum": 1, "maximum": 8192},
                            "x": {"type": "number"}, "y": {"type": "number"}, "html": html
                        }
                    }}
                }
            }),
            json!({"boards": [
                {"id": "desktop", "name": "Desktop", "width": 1280, "height": 800, "html": "<main>…</main>"},
                {"id": "phone", "name": "Phone", "caption": "Stacked", "width": 390, "height": 844, "html": "<main>…</main>"}
            ]}),
        ),
        "data" => (
            {
                let source = json!({
                    "type": "object", "required": ["id", "name", "kind", "detail", "at"], "additionalProperties": false,
                    "properties": {"id": {"type": "string"}, "name": {"type": "string"}, "kind": {"enum": SOURCE_KINDS},
                        "detail": {"type": "string", "description": "What was run or read"}, "at": {"type": "string", "description": "When, ISO 8601"}}
                });
                let text = json!({"type": "object", "required": ["type", "text"], "properties": {"type": {"const": "text"}, "text": {"type": "string", "description": "Markdown"}}});
                let stat = json!({"type": "object", "required": ["type", "label", "value", "source"], "properties": {
                    "type": {"const": "stat"}, "label": {"type": "string"}, "value": {"type": ["number", "string"]},
                    "unit": {"type": "string"}, "change": {"type": "number", "description": "A fraction: -0.62 is down 62%"}, "source": {"type": "string"}}});
                let table = json!({"type": "object", "required": ["type", "columns", "rows", "source"], "properties": {
                    "type": {"const": "table"}, "title": {"type": "string"}, "columns": {"type": "array", "items": {"type": "string"}},
                    "rows": {"type": "array", "items": {"type": "array"}}, "source": {"type": "string"}}});
                let series = json!({"type": "object", "required": ["name", "points"], "properties": {
                    "name": {"type": "string"}, "points": {"type": "array", "items": {"type": "array", "minItems": 2, "maxItems": 2}}}});
                let chart = json!({"type": "object", "required": ["type", "chart", "series", "source"], "properties": {
                    "type": {"const": "chart"}, "title": {"type": "string"}, "chart": {"enum": CHARTS}, "x": {"type": "string"},
                    "y": {"type": "string"}, "stacked": {"type": "boolean"},
                    "series": {"type": "array", "minItems": 1, "maxItems": 12, "items": series}, "source": {"type": "string"}}});
                json!({
                    "type": "object",
                    "required": ["title", "sources", "blocks"],
                    "additionalProperties": false,
                    "properties": {
                        "title": {"type": "string"},
                        "summary": {"type": "string"},
                        "sources": {"type": "array", "items": source},
                        "blocks": {"type": "array", "minItems": 1, "maxItems": PARTS, "items": {"oneOf": [text, stat, table, chart]}}
                    }
                })
            },
            json!({"title": "Rescan benchmark", "summary": "Rescans of a 50k-file tree.", "sources": [
                {"id": "bench", "name": "silt-bench rescan", "kind": "command", "detail": "target/perf/silt-bench run --only rescan", "at": "2026-11-15T09:00:00Z"}
            ], "blocks": [
                {"type": "stat", "label": "p50", "value": 412, "unit": "ms", "change": -0.62, "source": "bench"},
                {"type": "chart", "chart": "line", "title": "p50 by commit", "x": "commit", "y": "ms", "source": "bench", "series": [
                    {"name": "p50", "points": [["a1", 1080], ["b2", 640], ["c3", 412]]}
                ]},
                {"type": "text", "text": "The walk now happens once."}
            ]}),
        ),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_types_example_fits_its_schema() {
        for kind in TYPES {
            let (_, example) = guide(kind).expect("a guide");
            assert_eq!(validate(kind, &example.to_string()), Ok(()), "{kind}");
        }
    }

    #[test]
    fn a_version_that_doesnt_fit_is_refused_with_why() {
        assert!(validate("slides", r#"{"slides": []}"#).unwrap_err().contains("at least one slide"));
        assert!(validate("slides", r#"{"slides": [{"title": "a", "html": "x", "extra": 1}]}"#).is_err());
        assert!(validate("slides", r#"{"aspect": "21:9", "slides": [{"title": "a", "html": "x"}]}"#).is_err());
        let twins = r#"{"boards": [{"id": "a", "name": "A", "width": 10, "height": 10, "html": ""}, {"id": "a", "name": "B", "width": 10, "height": 10, "html": ""}]}"#;
        assert!(validate("canvas", twins).unwrap_err().contains("unique"));
        let unsourced = r#"{"title": "t", "sources": [], "blocks": [{"type": "stat", "label": "n", "value": 1, "source": "x"}]}"#;
        assert!(validate("data", unsourced).unwrap_err().contains("no source x"));
        let pie = r#"{"title": "t", "sources": [{"id": "s", "name": "s", "kind": "manual", "detail": "", "at": ""}], "blocks": [{"type": "chart", "chart": "pie", "series": [], "source": "s"}]}"#;
        assert!(validate("data", pie).is_err());
        assert_eq!(validate("html", "<p>anything</p>"), Ok(()));
    }

    #[test]
    fn a_patch_adds_a_slide_or_changes_nothing() {
        let deck = r#"{"slides": [{"title": "One", "html": "<h1>1</h1>"}]}"#;
        let patch = json!([{"op": "add", "path": "/slides/-", "value": {"title": "Two", "html": "<h1>2</h1>"}}]);
        let patched = apply_patch(deck, &patch).expect("patched");
        assert_eq!(serde_json::from_str::<Value>(&patched).expect("json")["slides"][1]["title"], "Two");
        assert!(apply_patch(deck, &json!([{"op": "remove", "path": "/slides/7"}])).is_err());
        assert!(apply_patch(deck, &json!({"op": "add"})).is_err());
    }
}
