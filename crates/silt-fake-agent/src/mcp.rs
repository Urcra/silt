//! Calls a tool of an MCP server the client gave the session, as a real
//! agent would, so Silt's own tools can be tested through an agent run.

use std::path::Path;

use agent_client_protocol::v1::schema::McpServer;
use serde_json::Value;

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
    let env: Vec<(String, String)> = server.env.iter().map(|variable| (variable.name.clone(), variable.value.clone())).collect();
    silt_fake_agent::call_tool(&server.command.to_string_lossy(), &server.args, &env, cwd, tool, arguments)
}

/// A 1×1 PNG, for an agent showing an image.
pub(crate) const PIXEL_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78,
    0x9c, 0x63, 0xb8, 0xd9, 0x61, 0xf5, 0x1f, 0x00, 0x06, 0x74, 0x02, 0x9b, 0xdc, 0x99, 0x50, 0x13, 0x00, 0x00, 0x00, 0x00, 0x49,
    0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];
