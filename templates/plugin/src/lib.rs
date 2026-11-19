//! Notes: reads and adds notes in Example Notes. A template for a Silt
//! plugin: rename it, point it at your API, and keep each call's card
//! saying exactly what it does.

use std::collections::BTreeMap;

use silt_plugin_api::http::{Request, send};
use silt_plugin_api::{Block, Call, Card, Context, Effect, Output, Plugin, Tool, Value, body_sha256, json};

const API: &str = "https://api.notes.example";

struct Notes;

fn notebook(input: &Value) -> Result<String, String> {
    let notebook = input.get("notebook").and_then(Value::as_str).map(str::trim).unwrap_or_default();
    if notebook.is_empty() || !notebook.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c)) {
        return Err("give `notebook`, letters, digits, `-` and `_`".into());
    }
    Ok(notebook.to_owned())
}

/// The body `add` sends, built the same way when it's prepared and when it
/// runs, so Silt can check it's the one approved.
fn note_body(text: &str) -> String {
    json!({ "text": text }).to_string()
}

impl Plugin for Notes {
    fn tools() -> Vec<Tool> {
        vec![
            Tool {
                name: "list".into(),
                description: "List the notes in a notebook.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": { "notebook": { "type": "string" } },
                    "required": ["notebook"]
                }),
                action: "list".into(),
            },
            Tool {
                name: "add".into(),
                description: "Add a note to a notebook.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": { "notebook": { "type": "string" }, "text": { "type": "string" } },
                    "required": ["notebook", "text"]
                }),
                action: "add".into(),
            },
        ]
    }

    fn prepare(tool: &str, input: &Value, _context: &Context) -> Result<Call, String> {
        let notebook = notebook(input)?;
        let fields = BTreeMap::from([("notebook".to_owned(), notebook.clone())]);
        match tool {
            "list" => Ok(Call {
                action: "list".into(),
                fields,
                card: Card { title: format!("List the notes in {notebook}"), blocks: vec![] },
                effects: vec![Effect::Http { method: "GET".into(), url: format!("{API}/notebooks/{notebook}/notes"), body_sha256: None }],
            }),
            "add" => {
                let text = input.get("text").and_then(Value::as_str).unwrap_or_default();
                let body = note_body(text);
                Ok(Call {
                    action: "add".into(),
                    fields,
                    card: Card {
                        title: format!("Add a note to {notebook}"),
                        blocks: vec![Block::Code { label: "The note".into(), text: text.to_owned() }],
                    },
                    effects: vec![Effect::Http {
                        method: "POST".into(),
                        url: format!("{API}/notebooks/{notebook}/notes"),
                        body_sha256: Some(body_sha256(body.as_bytes())),
                    }],
                })
            }
            other => Err(format!("no tool {other}")),
        }
    }

    fn run(tool: &str, input: &Value, _call: &Call, _context: &Context) -> Result<Output, String> {
        let notebook = notebook(input)?;
        let url = format!("{API}/notebooks/{notebook}/notes");
        let request = match tool {
            "list" => Request { method: "GET".into(), url, headers: vec![], body: None },
            "add" => Request {
                method: "POST".into(),
                url,
                headers: vec![("content-type".into(), "application/json".into())],
                body: Some(note_body(input.get("text").and_then(Value::as_str).unwrap_or_default()).into_bytes()),
            },
            other => return Err(format!("no tool {other}")),
        };
        let answer = send(&request)?;
        if !(200..300).contains(&answer.status) {
            return Err(format!("Example Notes answered {}", answer.status));
        }
        Ok(Output::text(String::from_utf8_lossy(&answer.body).into_owned()))
    }
}

silt_plugin_api::export!(Notes);
