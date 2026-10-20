//! A Model Context Protocol server on stdio that gives agents Silt's
//! history tools (ADR 0011). The protocol needs only a few methods, so the
//! JSON-RPC is written out here; what each tool does is up to a [`Backend`].

use std::io::{self, BufRead, Write};

use serde_json::{Value, json};

/// The protocol version answered when the client asks for none.
const DEFAULT_PROTOCOL: &str = "2025-06-18";

/// Runs the tools for the server.
pub trait Backend {
    /// The tool's text output, or an error message for the agent.
    fn call(&mut self, tool: &str, arguments: &Value) -> Result<String, String>;
}

/// Tells the agent what the tools are for.
pub const INSTRUCTIONS: &str = "Silt records this session's full history, including everything from before \
context compaction, and the history of the threads it was forked from. Use silt_history_search and \
silt_history_read to look back, silt_file_history to see which prompts changed a file, and silt_in_flight \
to see who else is changing a path.";

/// The tools' names, descriptions and input schemas.
pub fn tools() -> Value {
    let thread = json!({"type": "string", "description": "Thread id or its last 12 characters; defaults to this session's thread"});
    let page = |required: &[&str]| {
        json!({
            "type": "object",
            "properties": {
                "thread": thread,
                "offset": {"type": "integer", "minimum": 0, "description": "First entry to read; defaults to the start"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 200, "description": "Entries to read; defaults to 40"}
            },
            "required": required
        })
    };
    let query = |extra: &str| {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": format!("Words to look for{extra}")},
                "limit": {"type": "integer", "minimum": 1, "maximum": 50}
            },
            "required": ["query"]
        })
    };
    json!([
        {
            "name": "silt_history_search",
            "description": "Search this session's full history, including what was compacted away and what the thread inherited from the thread it was forked from. Returns entry numbers to read with silt_history_read.",
            "inputSchema": query(" in this thread's history")
        },
        {
            "name": "silt_history_read",
            "description": "Read numbered entries of this session's full history (user and assistant messages, tool calls and results, compaction summaries), including everything before compaction.",
            "inputSchema": page(&[])
        },
        {
            "name": "silt_file_history",
            "description": "Which prompts and tool calls changed a file in this thread, oldest first.",
            "inputSchema": {
                "type": "object",
                "properties": {"path": {"type": "string", "description": "Path relative to the repository root"}},
                "required": ["path"]
            }
        },
        {
            "name": "silt_thread_search",
            "description": "Search the conversations of every thread on this machine, yours and your team's.",
            "inputSchema": query(" in all threads")
        },
        {
            "name": "silt_fork_read",
            "description": "Read another thread's history, such as the thread this one was forked from or a fork of it.",
            "inputSchema": page(&["thread"])
        },
        {
            "name": "silt_in_flight",
            "description": "Other threads in this repository that are changing a file or directory, with their owners and when they last touched it.",
            "inputSchema": {
                "type": "object",
                "properties": {"path": {"type": "string", "description": "File or directory, relative to the working directory"}},
                "required": ["path"]
            }
        }
    ])
}

/// Serves newline-delimited JSON-RPC from `input` until it ends.
pub fn serve(input: impl BufRead, mut output: impl Write, backend: &mut impl Backend) -> io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Some(response) = respond(&line, backend) else {
            continue;
        };
        writeln!(output, "{response}")?;
        output.flush()?;
    }
    Ok(())
}

/// The response to one message; `None` for notifications.
fn respond(line: &str, backend: &mut impl Backend) -> Option<Value> {
    let message: Value = match serde_json::from_str(line) {
        Ok(message) => message,
        Err(error) => return Some(error_response(&Value::Null, -32700, &format!("parse error: {error}"))),
    };
    let id = message.get("id")?.clone();
    let method = message.get("method").and_then(Value::as_str).unwrap_or_default();
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => json!({
            "protocolVersion": params.get("protocolVersion").and_then(Value::as_str).unwrap_or(DEFAULT_PROTOCOL),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "silt", "version": env!("CARGO_PKG_VERSION")},
            "instructions": INSTRUCTIONS
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match backend.call(name, &arguments) {
                Ok(text) => json!({"content": [{"type": "text", "text": text}], "isError": false}),
                Err(text) => json!({"content": [{"type": "text", "text": text}], "isError": true}),
            }
        }
        other => return Some(error_response(&id, -32601, &format!("unknown method {other}"))),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn error_response(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;

    impl Backend for Echo {
        fn call(&mut self, tool: &str, arguments: &Value) -> Result<String, String> {
            match tool {
                "silt_history_search" => Ok(format!("searched {}", arguments["query"])),
                other => Err(format!("no {other}")),
            }
        }
    }

    fn exchange(lines: &[&str]) -> Vec<Value> {
        let mut output = Vec::new();
        serve(lines.join("\n").as_bytes(), &mut output, &mut Echo).expect("serves");
        String::from_utf8(output)
            .expect("utf-8")
            .lines()
            .map(|line| serde_json::from_str(line).expect("json"))
            .collect()
    }

    #[test]
    fn answers_the_handshake_lists_tools_and_calls_them() {
        let replies = exchange(&[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{}}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"silt_history_search","arguments":{"query":"lexer"}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"nope","arguments":{}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"resources/list"}"#,
        ]);
        assert_eq!(replies.len(), 5, "notifications get no reply");
        assert_eq!(replies[0]["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(replies[1]["result"]["tools"].as_array().map(Vec::len), Some(6));
        assert_eq!(replies[2]["result"]["content"][0]["text"], "searched \"lexer\"");
        assert_eq!(replies[3]["result"]["isError"], true);
        assert_eq!(replies[4]["error"]["code"], -32601);
    }
}
