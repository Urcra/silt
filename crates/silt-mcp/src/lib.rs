//! A Model Context Protocol server on stdio that gives agents Silt's
//! history tools (ADR 0011). The protocol needs only a few methods, so the
//! JSON-RPC is written out here; what each tool does is up to a [`Backend`].

use std::io::{self, BufRead, Write};

use serde_json::{Value, json};

/// The protocol versions this server speaks, newest first.
const SUPPORTED_PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
/// Most bytes of text one tool answer carries.
const MAX_OUTPUT: usize = 64 * 1024;

/// Runs the tools for the server.
pub trait Backend {
    /// The tool's text output, or an error message for the agent.
    fn call(&mut self, tool: &str, arguments: &Value) -> Result<String, String>;

    /// Prompts the agent offers its user, such as slash commands.
    fn prompts(&mut self) -> Vec<Prompt> {
        Vec::new()
    }

    /// The text of prompt `name`.
    fn prompt(&mut self, name: &str) -> Result<String, String> {
        Err(format!("unknown prompt {name}"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub name: String,
    pub description: String,
}

/// Tells the agent what the tools are for.
pub const INSTRUCTIONS: &str = "Silt records this session's full history, including everything from before \
context compaction, and the history of the threads it was forked from. Use silt_history_search and \
silt_history_read to look back, silt_file_history to see which prompts changed a file, and silt_in_flight \
to see who else is changing a path. silt_skills lists the repository's skills and silt_use_skill loads one.";

/// The tools' names, descriptions and input schemas.
pub fn tools() -> Value {
    let thread =
        json!({"type": "string", "description": "Thread id or its last 12 characters; defaults to this session's thread"});
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
            "name": "silt_skills",
            "description": "List the skills this repository and the user provide (from .silt/skills, .agents/skills and ~/.agents/skills), with what each is for.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "silt_use_skill",
            "description": "Load a skill's instructions, and where its files are, to follow it for the current task.",
            "inputSchema": {
                "type": "object",
                "properties": {"name": {"type": "string", "description": "The skill's name, as silt_skills lists it"}},
                "required": ["name"]
            }
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

/// Serves newline-delimited JSON-RPC from `input` until it ends. A line
/// that isn't UTF-8 gets an error reply like any other unreadable message.
pub fn serve(input: impl BufRead, mut output: impl Write, backend: &mut impl Backend) -> io::Result<()> {
    for line in input.split(b'\n') {
        let line = line?;
        let response = match std::str::from_utf8(&line) {
            Ok(line) if line.trim().is_empty() => continue,
            Ok(line) => respond(line, backend),
            Err(_) => Some(error_response(&Value::Null, -32700, "parse error: the message is not UTF-8")),
        };
        let Some(response) = response else {
            continue;
        };
        writeln!(output, "{response}")?;
        output.flush()?;
    }
    Ok(())
}

/// The version to speak: the client's when it is one of these, else the
/// newest.
fn protocol_version(requested: Option<&str>) -> &'static str {
    SUPPORTED_PROTOCOLS.iter().copied().find(|version| Some(*version) == requested).unwrap_or(SUPPORTED_PROTOCOLS[0])
}

/// Checks a call's integer arguments against the bounds its tool declares.
fn check_arguments(tool: &str, arguments: &Value) -> Result<(), String> {
    let tools = tools();
    let Some(properties) = tools
        .as_array()
        .and_then(|tools| tools.iter().find(|candidate| candidate["name"] == tool))
        .and_then(|found| found.pointer("/inputSchema/properties"))
        .and_then(Value::as_object)
    else {
        return Ok(());
    };
    for (name, schema) in properties {
        let Some(value) = arguments.get(name).filter(|_| schema["type"] == "integer") else {
            continue;
        };
        let number = value.as_i64().ok_or_else(|| format!("{name} must be a whole number"))?;
        if let Some(minimum) = schema["minimum"].as_i64().filter(|minimum| number < *minimum) {
            return Err(format!("{name} must be at least {minimum}"));
        }
        if let Some(maximum) = schema["maximum"].as_i64().filter(|maximum| number > *maximum) {
            return Err(format!("{name} must be at most {maximum}"));
        }
    }
    Ok(())
}

/// A tool's text, clipped so one answer can't fill the agent's context.
fn bounded(text: String) -> String {
    if text.len() <= MAX_OUTPUT {
        return text;
    }
    let mut end = MAX_OUTPUT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… (cut at {MAX_OUTPUT} bytes; ask for less, such as a smaller limit)", &text[..end])
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
            "protocolVersion": protocol_version(params.get("protocolVersion").and_then(Value::as_str)),
            "capabilities": {"tools": {}, "prompts": {}},
            "serverInfo": {"name": "silt", "version": env!("CARGO_PKG_VERSION")},
            "instructions": INSTRUCTIONS
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools() }),
        "prompts/list" => {
            let prompts: Vec<Value> = backend
                .prompts()
                .into_iter()
                .map(|prompt| json!({"name": prompt.name, "description": prompt.description, "arguments": []}))
                .collect();
            json!({ "prompts": prompts })
        }
        "prompts/get" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            match backend.prompt(name) {
                Ok(text) => json!({"messages": [{"role": "user", "content": {"type": "text", "text": text}}]}),
                Err(message) => return Some(error_response(&id, -32602, &message)),
            }
        }
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match check_arguments(name, &arguments).and_then(|()| backend.call(name, &arguments)) {
                Ok(text) => json!({"content": [{"type": "text", "text": bounded(text)}], "isError": false}),
                Err(text) => json!({"content": [{"type": "text", "text": bounded(text)}], "isError": true}),
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

        fn prompts(&mut self) -> Vec<Prompt> {
            vec![Prompt { name: "land".into(), description: "Land the branch".into() }]
        }

        fn prompt(&mut self, name: &str) -> Result<String, String> {
            (name == "land").then(|| "Land it.".to_owned()).ok_or_else(|| "no such prompt".to_owned())
        }
    }

    fn exchange(lines: &[&str]) -> Vec<Value> {
        let mut output = Vec::new();
        serve(lines.join("\n").as_bytes(), &mut output, &mut Echo).expect("serves");
        String::from_utf8(output).expect("utf-8").lines().map(|line| serde_json::from_str(line).expect("json")).collect()
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
        assert_eq!(replies[1]["result"]["tools"].as_array().map(Vec::len), Some(8));
        assert_eq!(replies[2]["result"]["content"][0]["text"], "searched \"lexer\"");
        assert_eq!(replies[3]["result"]["isError"], true);
        assert_eq!(replies[4]["error"]["code"], -32601);
    }

    #[test]
    fn holds_calls_to_their_declared_bounds() {
        let replies = exchange(&[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"silt_history_search","arguments":{"query":"x","limit":51}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"silt_history_read","arguments":{"limit":0}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"silt_history_search","arguments":{"query":"x","limit":50}}}"#,
        ]);
        assert_eq!(replies[0]["result"]["protocolVersion"], SUPPORTED_PROTOCOLS[0], "not a version it can't speak");
        assert_eq!(replies[1]["result"]["isError"], true);
        assert_eq!(replies[1]["result"]["content"][0]["text"], "limit must be at most 50");
        assert_eq!(replies[2]["result"]["content"][0]["text"], "limit must be at least 1");
        assert_eq!(replies[3]["result"]["isError"], false);
        assert!(bounded("é".repeat(MAX_OUTPUT)).len() < MAX_OUTPUT + 100);
    }

    #[test]
    fn a_line_that_isnt_utf8_is_answered_and_serving_goes_on() {
        let mut input = b"\xff\xfe\n".to_vec();
        input.extend_from_slice(br#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#);
        let mut output = Vec::new();
        serve(input.as_slice(), &mut output, &mut Echo).expect("serves");
        let replies: Vec<Value> =
            String::from_utf8(output).expect("utf-8").lines().map(|line| serde_json::from_str(line).expect("json")).collect();
        assert_eq!(replies[0]["error"]["code"], -32700);
        assert_eq!(replies[1]["id"], 2);
    }

    #[test]
    fn offers_prompts() {
        let replies = exchange(&[
            r#"{"jsonrpc":"2.0","id":1,"method":"prompts/list"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"prompts/get","params":{"name":"land"}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"prompts/get","params":{"name":"nope"}}"#,
        ]);
        assert_eq!(replies[0]["result"]["prompts"][0]["name"], "land");
        assert_eq!(replies[1]["result"]["messages"][0]["content"]["text"], "Land it.");
        assert_eq!(replies[2]["error"]["code"], -32602);
    }
}
