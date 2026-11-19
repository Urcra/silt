//! Local MCP, shipped with Silt (ADR 0104): a local MCP server's tools,
//! the server run by Silt in its own sandbox. Each call is opaque.

use std::collections::BTreeMap;

use silt_plugin_api::{Block, Call, Card, Channel, Context, Effect, Output, Plugin, Tone, Tool, Value, json, mcp, serde_json};

struct LocalMcp;

/// The text of an MCP tool's result.
pub(crate) fn result_text(result: &Value) -> String {
    let texts: Vec<&str> = result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|part| part.get("text")?.as_str())
        .collect();
    if texts.is_empty() { serde_json::to_string_pretty(result).unwrap_or_default() } else { texts.join("\n") }
}

impl Plugin for LocalMcp {
    fn tools() -> Vec<Tool> {
        let Ok(listed) = mcp::tools() else { return Vec::new() };
        let listed: Value = serde_json::from_str(&listed).unwrap_or(Value::Null);
        listed
            .get("tools")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|tool| {
                Some(Tool {
                    name: tool.get("name")?.as_str()?.to_owned(),
                    description: tool.get("description").and_then(Value::as_str).unwrap_or_default().to_owned(),
                    input_schema: tool.get("inputSchema").cloned().unwrap_or_else(|| json!({"type": "object"})),
                    action: "call".into(),
                })
            })
            .collect()
    }

    fn prepare(tool: &str, input: &Value, _context: &Context) -> Result<Call, String> {
        Ok(Call {
            action: "call".into(),
            fields: BTreeMap::from([("tool".into(), tool.into())]),
            card: Card {
                title: format!("Call {tool} on the local MCP server"),
                blocks: vec![
                    Block::Code { label: "Arguments".into(), text: serde_json::to_string_pretty(input).unwrap_or_default() },
                    Block::Note { tone: Tone::Warn, text: "Silt can't check what the server does with them.".into() },
                ],
            },
            effects: vec![Effect::Opaque { channel: Channel::McpProcess, what: format!("its tool {tool}") }],
        })
    }

    fn run(tool: &str, input: &Value, _call: &Call, _context: &Context) -> Result<Output, String> {
        let called: Value = serde_json::from_str(&mcp::call(tool, &input.to_string())?).unwrap_or(Value::Null);
        let text = result_text(&called);
        if called.get("isError").and_then(Value::as_bool).unwrap_or(false) { Err(text) } else { Ok(Output::text(text)) }
    }
}

silt_plugin_api::export!(LocalMcp);
