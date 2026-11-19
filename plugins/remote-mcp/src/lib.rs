//! Remote MCP, shipped with Silt (ADR 0104): a remote MCP server's tools
//! over streamable HTTP. Every request is declared with its body, so what
//! goes is what was shown.

use std::collections::BTreeMap;

use silt_plugin_api::http::{Request, Response, send};
use silt_plugin_api::{Block, Call, Card, Context, Effect, Output, Plugin, Tool, Value, host, json, serde_json};

struct RemoteMcp;

fn url() -> Result<String, String> {
    host::setting("url")
        .and_then(|value| value.as_str().map(str::to_owned))
        .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
        .ok_or_else(|| "set the server with `silt plugin set silt.remote-mcp url <url>`".to_owned())
}

/// The three requests a call is: the handshake, then `method`.
fn bodies(method: &str, params: &Value) -> [String; 3] {
    [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "silt", "version": "1"}}}).to_string(),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string(),
        json!({"jsonrpc": "2.0", "id": 2, "method": method, "params": params}).to_string(),
    ]
}

fn request(tool: &str, input: &Value) -> Result<(&'static str, String, Value), String> {
    match tool {
        "tools" => Ok(("list", String::new(), json!({}))),
        "call" => {
            let name = input.get("tool").and_then(Value::as_str).filter(|name| !name.is_empty()).ok_or("give `tool`")?;
            let arguments = input.get("arguments").cloned().unwrap_or_else(|| json!({}));
            Ok(("call", name.to_owned(), json!({"name": name, "arguments": arguments})))
        }
        other => Err(format!("no tool {other}")),
    }
}

/// A JSON-RPC answer, from JSON or from an event stream's `data:` lines.
fn answer_of(response: &Response) -> Option<Value> {
    let text = String::from_utf8_lossy(&response.body);
    serde_json::from_str::<Value>(&text).ok().or_else(|| {
        text.lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .filter_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
            .find(|message| message.get("id") == Some(&json!(2)))
    })
}

impl Plugin for RemoteMcp {
    fn tools() -> Vec<Tool> {
        vec![
            Tool {
                name: "tools".into(),
                description: "List the remote MCP server's tools, with what each takes.".into(),
                input_schema: json!({"type": "object"}),
                action: "list".into(),
            },
            Tool {
                name: "call".into(),
                description: "Call one of the remote MCP server's tools with its arguments.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {"tool": {"type": "string"}, "arguments": {"type": "object"}},
                    "required": ["tool"]
                }),
                action: "call".into(),
            },
        ]
    }

    fn prepare(tool: &str, input: &Value, _context: &Context) -> Result<Call, String> {
        let (action, name, params) = request(tool, input)?;
        let url = url()?;
        let method = if action == "list" { "tools/list" } else { "tools/call" };
        let effects = bodies(method, &params)
            .iter()
            .map(|body| Effect::Http {
                method: "POST".into(),
                url: url.clone(),
                body_sha256: Some(silt_plugin_api::body_sha256(body.as_bytes())),
            })
            .collect();
        let mut blocks = vec![Block::Fields { pairs: vec![("Server".into(), url.clone())] }];
        let title = if action == "list" {
            "List the remote MCP server's tools".to_owned()
        } else {
            blocks.push(Block::Code {
                label: "Arguments".into(),
                text: serde_json::to_string_pretty(&params["arguments"]).unwrap_or_default(),
            });
            format!("Call {name} on the remote MCP server")
        };
        let fields = if action == "call" { BTreeMap::from([("tool".into(), name)]) } else { BTreeMap::new() };
        Ok(Call { action: action.into(), fields, card: Card { title, blocks }, effects })
    }

    fn run(tool: &str, input: &Value, call: &Call, _context: &Context) -> Result<Output, String> {
        let (action, _, params) = request(tool, input)?;
        let Some(Effect::Http { url, .. }) = call.effects.first() else { return Err("this call names no server".into()) };
        let method = if action == "list" { "tools/list" } else { "tools/call" };
        let mut session: Option<String> = None;
        let mut last = None;
        for body in bodies(method, &params) {
            let mut headers = vec![
                ("content-type".to_owned(), "application/json".to_owned()),
                ("accept".to_owned(), "application/json, text/event-stream".to_owned()),
            ];
            if let Some(session) = &session {
                headers.push(("mcp-session-id".into(), session.clone()));
            }
            let response = send(&Request { method: "POST".into(), url: url.clone(), headers, body: Some(body.into_bytes()) })?;
            if !(200..300).contains(&response.status) {
                return Err(format!("the server answered {}", response.status));
            }
            if session.is_none() {
                session = response
                    .headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("mcp-session-id"))
                    .map(|(_, value)| value.clone());
            }
            last = Some(response);
        }
        let answer = last.as_ref().and_then(answer_of).ok_or("the server's answer couldn't be read")?;
        if let Some(error) = answer.get("error") {
            return Err(error.get("message").and_then(Value::as_str).unwrap_or("it failed").to_owned());
        }
        let result = answer.get("result").cloned().unwrap_or(Value::Null);
        let texts: Vec<&str> = result
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|part| part.get("text")?.as_str())
            .collect();
        let text = if texts.is_empty() { serde_json::to_string_pretty(&result).unwrap_or_default() } else { texts.join("\n") };
        Ok(Output { text, data: Some(result) })
    }
}

silt_plugin_api::export!(RemoteMcp);
