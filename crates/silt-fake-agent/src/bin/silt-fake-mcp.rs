//! A local MCP server for the tests of plugins that wrap one (ADR 0103):
//! over stdio, one tool, `peek`, which says whether it could read `path`
//! and reach `host`:`port` from where it runs.

use std::io::{BufRead as _, Write as _};

use serde_json::{Value, json};

fn peek(arguments: &Value) -> String {
    let path = arguments.get("path").and_then(Value::as_str).unwrap_or_default();
    let read = std::fs::read(path).is_ok();
    let host = arguments.get("host").and_then(Value::as_str).unwrap_or_default();
    let port = arguments.get("port").and_then(Value::as_u64).unwrap_or(80);
    let reached = !host.is_empty()
        && std::net::ToSocketAddrs::to_socket_addrs(&(host, u16::try_from(port).unwrap_or(80)))
            .ok()
            .and_then(|mut addresses| addresses.next())
            .is_some_and(|address| std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_secs(2)).is_ok());
    format!("read {path}: {}; reached {host}:{port}: {}", if read { "yes" } else { "no" }, if reached { "yes" } else { "no" })
}

fn main() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(id) = message.get("id").cloned() else { continue };
        let result = match message.get("method").and_then(Value::as_str) {
            Some("initialize") => {
                json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "fake-mcp", "version": "1"}})
            }
            Some("tools/list") => {
                json!({"tools": [{"name": "peek", "description": "Says what it can read and reach", "inputSchema": {"type": "object"}}]})
            }
            Some("tools/call") => {
                let arguments = message.pointer("/params/arguments").cloned().unwrap_or(Value::Null);
                json!({"content": [{"type": "text", "text": peek(&arguments)}]})
            }
            _ => json!({}),
        };
        let _ = writeln!(stdout, "{}", json!({"jsonrpc": "2.0", "id": id, "result": result}));
        let _ = stdout.flush();
    }
}
