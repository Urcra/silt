//! Calls a tool of an MCP server the client gave the session, as a real
//! agent would, so Silt's own tools can be tested through an agent run.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use agent_client_protocol::schema::v1::McpServer;
use serde_json::{Value, json};

/// The text of `tool`'s answer from the session's server named `silt`, or
/// why it failed.
pub(crate) fn call(servers: &[McpServer], cwd: &Path, tool: &str, arguments: Value) -> Result<String, String> {
    let server = servers
        .iter()
        .find_map(|server| match server {
            McpServer::Stdio(stdio) if stdio.name == "silt" => Some(stdio),
            _ => None,
        })
        .ok_or("the session has no silt MCP server")?;
    let mut child = Command::new(&server.command)
        .args(&server.args)
        .envs(server.env.iter().map(|variable| (&variable.name, &variable.value)))
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not start the MCP server: {error}"))?;
    let mut input = child.stdin.take().ok_or("no stdin")?;
    let messages = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": tool, "arguments": arguments}}),
    ];
    for message in messages {
        writeln!(input, "{message}").map_err(|error| error.to_string())?;
    }
    input.flush().map_err(|error| error.to_string())?;
    let output = child.stdout.take().ok_or("no stdout")?;
    let mut answer = Err("the MCP server didn't answer".to_owned());
    for line in BufReader::new(output).lines() {
        let Ok(reply) = serde_json::from_str::<Value>(&line.map_err(|error| error.to_string())?) else {
            continue;
        };
        if reply["id"] == 2 {
            let text = reply["result"]["content"][0]["text"].as_str().unwrap_or_default().to_owned();
            answer = if reply["result"]["isError"] == true { Err(text) } else { Ok(text) };
            break;
        }
    }
    drop(input);
    let _ = child.kill();
    let _ = child.wait();
    answer
}

/// A 1×1 PNG, for an agent showing an image.
pub(crate) const PIXEL_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78,
    0x9c, 0x63, 0xb8, 0xd9, 0x61, 0xf5, 0x1f, 0x00, 0x06, 0x74, 0x02, 0x9b, 0xdc, 0x99, 0x50, 0x13, 0x00, 0x00, 0x00, 0x00, 0x49,
    0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];
