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

    /// Whether a person watches this run in the web UI, so the tools that
    /// show them something are offered (ADR 0068).
    fn shows(&self) -> bool {
        false
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

/// Added to [`INSTRUCTIONS`] when the run is watched in the web UI.
pub const SHOW_INSTRUCTIONS: &str = " The person follows this session in Silt's web UI: to show them an image, call \
silt_show; to give them a page, chart, diagram, mockup or document they can open beside the conversation, call \
silt_artifact_guide first, follow what it says, then call silt_artifact, and call it again with the same id for a new \
version. silt_artifact_read gives back a version's source, to revise it after a handoff or a fork.";

/// The tools only a run watched in the web UI gets.
pub const SHOW_TOOLS: &[&str] = &[
    "silt_show",
    "silt_artifact_guide",
    "silt_artifact",
    "silt_artifact_read",
    "silt_artifact_check",
    "silt_artifact_data",
    "silt_artifact_doc",
    "silt_artifact_sheet",
];

/// What the server says it's for, with the showing tools when offered.
pub fn instructions(shows: bool) -> String {
    if shows { format!("{INSTRUCTIONS}{SHOW_INSTRUCTIONS}") } else { INSTRUCTIONS.to_owned() }
}

/// The tools' names, descriptions and input schemas; `shows` adds the tools
/// that show the person something.
pub fn tools(shows: bool) -> Value {
    let mut tools = history_tools();
    if shows && let Some(list) = tools.as_array_mut() {
        list.extend(show_tools());
    }
    tools
}

fn show_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "silt_show",
            "description": "Show the person an image (PNG, JPEG, GIF, WebP or SVG) in the conversation, such as a screenshot or a chart you rendered. Write the file first, then pass its path.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "The image file, absolute or relative to the working directory"},
                    "caption": {"type": "string", "description": "A few words under the image"}
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "silt_artifact_guide",
            "description": "Read before making an artifact: how to design it for Silt's frame, the design system that applies (its rules and token names), and the others there are. Call it once per artifact, before writing its first version.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": {"type": "string", "enum": ["html", "svg", "markdown", "mermaid", "image", "slides", "canvas", "data", "doc", "sheet"], "description": "What the artifact will be: slides a deck, canvas boards of mockups, data a page of numbers with their sources, doc a document people edit with you, sheet a grid with formulas"},
                    "design_system": {"type": "string", "description": "A design system to read instead of the one that applies, by the name the guide lists"}
                },
                "required": ["kind"]
            }
        }),
        json!({
            "name": "silt_artifact_check",
            "description": "Render a version of an HTML artifact as the person's panel shows it, at a phone's width and a desktop's, in light and dark, with a headless browser where Node and Playwright are installed: says what was blocked, what failed and whether it's too wide, and where each screenshot is. Use it once before telling the person, not in a loop.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "The artifact's id"},
                    "version": {"type": "integer", "description": "A version other than the latest"}
                },
                "required": ["id"]
            }
        }),
        json!({
            "name": "silt_artifact_read",
            "description": "Read back a version of one of this thread's artifacts, the latest by default, to revise it: its source is written to a file outside the checkout, whose path this returns, and a short source is also returned whole.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "The artifact's id, as silt_artifact returned it or its card shows it"},
                    "version": {"type": "integer", "description": "A version other than the latest"}
                },
                "required": ["id"]
            }
        }),
        json!({
            "name": "silt_artifact_data",
            "description": "Read or write an artifact's page data, which its page reaches with silt.use(\"db\") (ADR 0072): seed a tracker or a poll after the first version, or read back what people entered. The artifact must declare db in its capabilities. Paths are collection/doc, collection/doc/collection/doc and so on; values are JSON.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "The artifact's id"},
                    "op": {"type": "string", "enum": ["get", "list", "query", "set", "update", "delete", "batch"], "description": "What to do"},
                    "path": {"type": "string", "description": "A document's path, for get, set, update and delete"},
                    "collection": {"type": "string", "description": "A collection's path, for list and query"},
                    "value": {"description": "The document, for set"},
                    "fields": {"type": "object", "description": "Fields to merge in, for update; a null field removes it"},
                    "query": {"type": "object", "description": "For query: where (a list of {field, op, value}), order_by, descending, limit"},
                    "writes": {"type": "array", "description": "For batch: each {op: set|update|delete, path, value|fields}, all or none"}
                },
                "required": ["id", "op"]
            }
        }),
        json!({
            "name": "silt_artifact_doc",
            "description": "Read or edit a doc artifact's text, which people may be editing at the same moment (ADR 0077): each edit merges with theirs. Make the doc first with silt_artifact, kind doc and Markdown content; then change it here, not by a new version. Each edit's old text must occur exactly once in the doc as it is now; read it first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "The doc's id"},
                    "edits": {"type": "array", "description": "Changes in order, all or none; leave out to read the text", "items": {
                        "type": "object", "properties": {"old": {"type": "string"}, "new": {"type": "string"}}, "required": ["old", "new"]
                    }}
                },
                "required": ["id"]
            }
        }),
        json!({
            "name": "silt_artifact_sheet",
            "description": "Read or write a sheet artifact's cells (ADR 0077). Make the sheet first with silt_artifact, kind sheet and content {\"rows\": [[...], ...]}; then write cells here by address, with formulas such as =SUM(B2:B9). Rows and columns grow to fit. Formulas: SUM, AVERAGE, MIN, MAX, COUNT, COUNTA, IF, IFS, AND, OR, NOT, ROUND, ABS, CONCAT, TEXT, LEFT, RIGHT, LEN, TODAY, DATE, VLOOKUP, XLOOKUP, SUMIF, COUNTIF.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "The sheet's id"},
                    "set": {"type": "object", "description": "Cells to write, such as {\"A1\": \"Task\", \"B7\": \"=SUM(B2:B6)\"}; \"\" clears one"},
                    "read": {"type": "string", "description": "A range whose computed values to read, such as A1:D20; leave both out to read the whole sheet"}
                },
                "required": ["id"]
            }
        }),
        json!({
            "name": "silt_artifact",
            "description": "Give the person a page, chart, diagram or document that opens beside the conversation, versioned. Without id it makes a new artifact (give title and kind) and returns its id; with the id of one you made, it adds a version, from new content or from edits to the last one. HTML runs sandboxed: scripts load only from cdnjs.cloudflare.com, cdn.jsdelivr.net/npm/, unpkg.com, cdn.tailwindcss.com and code.jquery.com, pinned to an exact version, and stylesheets only from Google Fonts; nothing else loads and nothing connects, so inline other styles, images and data. Call silt_artifact_guide before the first version.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": {"type": "string", "description": "A short name, such as \"Rescan benchmark\""},
                    "kind": {"type": "string", "enum": ["html", "svg", "markdown", "mermaid", "image", "slides", "canvas", "data", "doc", "sheet"], "description": "What the content is; slides, canvas and data are JSON that silt_artifact_guide describes; doc (Markdown) and sheet ({\"rows\": [[...]]}) are edited in place afterwards with silt_artifact_doc and silt_artifact_sheet"},
                    "content": {"type": "string", "description": "The text of an html, svg, markdown or mermaid artifact; give this or path"},
                    "path": {"type": "string", "description": "A file holding the content, absolute or relative to the working directory; needed for an image"},
                    "id": {"type": "string", "description": "The id silt_artifact returned, to add a version of that artifact; title and kind then default to its last version's"},
                    "edits": {
                        "type": "array",
                        "description": "With id, in place of content or path: changes to the latest version's source, applied in order, all or none; each old text must occur exactly once",
                        "items": {
                            "type": "object",
                            "properties": {
                                "old": {"type": "string", "description": "Text copied exactly from the source"},
                                "new": {"type": "string", "description": "What replaces it; empty deletes it"},
                                "file": {"type": "string", "description": "One of the version's files to change, rather than the page"}
                            },
                            "required": ["old", "new"]
                        }
                    },
                    "patch": {"type": "array", "description": "With id, for slides, canvas or data: an RFC 6902 JSON Patch of the latest version's content, such as [{\"op\": \"add\", \"path\": \"/slides/-\", \"value\": {...}}]", "items": {"type": "object"}},
                    "files": {"type": "object", "description": "For an html page: files beside it, by relative path such as data.json or css/site.css, each {content}, {base64} or {path}, or null to remove one; the page fetches them relative to itself. The last version's files are kept unless named"},
                    "note": {"type": "string", "description": "What this version changed, in a few words, shown in the version menu"},
                    "description": {"type": "string", "description": "One sentence on what the artifact is, shown on its card; kept for later versions"},
                    "capabilities": {"type": "object", "description": "What the html page may use through silt.use(name), as {name: config}: db, artifact, user, downloads, comments, sample and the others silt_artifact_guide lists. Leave it out to keep the last version's; {} clears them"},
                    "design_system": {"type": "string", "description": "The design system whose tokens an html artifact's frame gets, by name, or none; by default the last version's, else the one silt_artifact_guide says applies"}
                },
                "required": []
            }
        }),
    ]
}

fn history_tools() -> Value {
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
    let mut tools = json!([
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
    ]);
    // Each only reads, which a client such as a page connector relies on (ADR 0080).
    for tool in tools.as_array_mut().into_iter().flatten() {
        tool["annotations"] = json!({"readOnlyHint": true});
    }
    tools
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
fn check_arguments(tool: &str, arguments: &Value, shows: bool) -> Result<(), String> {
    if !shows && SHOW_TOOLS.contains(&tool) {
        return Err(format!("unknown tool {tool}"));
    }
    let tools = tools(shows);
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
            "instructions": instructions(backend.shows())
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools(backend.shows()) }),
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
            match check_arguments(name, &arguments, backend.shows()).and_then(|()| backend.call(name, &arguments)) {
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

    #[derive(Default)]
    struct Echo {
        shows: bool,
    }

    impl Backend for Echo {
        fn shows(&self) -> bool {
            self.shows
        }

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
        serve(lines.join("\n").as_bytes(), &mut output, &mut Echo::default()).expect("serves");
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
        serve(input.as_slice(), &mut output, &mut Echo::default()).expect("serves");
        let replies: Vec<Value> =
            String::from_utf8(output).expect("utf-8").lines().map(|line| serde_json::from_str(line).expect("json")).collect();
        assert_eq!(replies[0]["error"]["code"], -32700);
        assert_eq!(replies[1]["id"], 2);
    }

    #[test]
    fn the_showing_tools_are_offered_only_to_a_watched_run() {
        let lines = [
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"silt_show","arguments":{"path":"a.png"}}}"#,
        ];
        let names = |reply: &Value| -> Vec<String> {
            reply["result"]["tools"]
                .as_array()
                .expect("tools")
                .iter()
                .map(|tool| tool["name"].as_str().unwrap_or_default().to_owned())
                .collect()
        };
        let quiet = exchange(&lines);
        assert!(!names(&quiet[1]).iter().any(|name| SHOW_TOOLS.contains(&name.as_str())));
        assert!(!quiet[0]["result"]["instructions"].as_str().unwrap_or_default().contains("silt_show"));
        assert_eq!(quiet[2]["result"]["content"][0]["text"], "unknown tool silt_show");

        let mut output = Vec::new();
        serve(lines.join("\n").as_bytes(), &mut output, &mut Echo { shows: true }).expect("serves");
        let watched: Vec<Value> =
            String::from_utf8(output).expect("utf-8").lines().map(|line| serde_json::from_str(line).expect("json")).collect();
        assert!(SHOW_TOOLS.iter().all(|tool| names(&watched[1]).iter().any(|name| name == tool)));
        assert!(watched[0]["result"]["instructions"].as_str().unwrap_or_default().contains("silt_artifact"));
        assert_eq!(watched[2]["result"]["content"][0]["text"], "no silt_show", "the backend answers it");
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
