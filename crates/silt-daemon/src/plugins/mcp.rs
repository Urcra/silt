//! A plugin that is an MCP server (ADR 0086): Silt starts it on this
//! machine, outside every sandbox, with the environment's credential, and
//! offers its tools to the workspace's agents. A tool its server marks
//! read-only is a read; any other is a change. Each call starts the server
//! afresh, so no state of one call reaches the next.

use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

/// How long the server has to start and to answer one request.
const ANSWER: Duration = Duration::from_secs(60);
const PROTOCOL: &str = "2025-06-18";

pub struct Server {
    child: Child,
    stdin: ChildStdin,
    lines: tokio::io::Lines<BufReader<ChildStdout>>,
    next: u64,
}

impl Server {
    /// Starts `argv` with `env` and opens its MCP session.
    pub async fn start(argv: &[String], env: &[(String, String)]) -> Result<Self, String> {
        let (program, args) = argv.split_first().ok_or("the plugin runs no program")?;
        let mut child = Command::new(program)
            .args(args)
            .envs(env.iter().map(|(name, value)| (name, value)))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| format!("cannot start {program}: {error}"))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let mut server = Self { child, stdin, lines: BufReader::new(stdout).lines(), next: 0 };
        let params = json!({"protocolVersion": PROTOCOL, "capabilities": {}, "clientInfo": {"name": "silt", "version": env!("CARGO_PKG_VERSION")}});
        server.request("initialize", params).await?;
        server.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await?;
        Ok(server)
    }

    async fn send(&mut self, message: &Value) -> Result<(), String> {
        let mut line = message.to_string();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await.map_err(|error| format!("the server stopped listening: {error}"))
    }

    /// Sends `method` and waits for its answer, answering nothing the
    /// server asks meanwhile.
    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next += 1;
        let id = self.next;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).await?;
        let answer = async {
            loop {
                let line = self.lines.next_line().await.map_err(|error| error.to_string())?.ok_or("the server ended")?;
                let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
                if message.get("id").and_then(Value::as_u64) == Some(id) && message.get("method").is_none() {
                    if let Some(error) = message.get("error") {
                        return Err(error.get("message").and_then(Value::as_str).unwrap_or("an error").to_owned());
                    }
                    return Ok(message.get("result").cloned().unwrap_or(Value::Null));
                }
                // A request of the server's own, such as sampling: refused,
                // as nothing here acts for it.
                if let (Some(asked), Some(_)) = (message.get("id").cloned(), message.get("method")) {
                    let refusal = json!({"jsonrpc": "2.0", "id": asked, "error": {"code": -32601, "message": "Silt answers no requests from a plugin"}});
                    let mut line = refusal.to_string();
                    line.push('\n');
                    let _ = self.stdin.write_all(line.as_bytes()).await;
                }
            }
        };
        tokio::time::timeout(ANSWER, answer).await.map_err(|_| format!("the server didn't answer {method} within a minute"))?
    }

    /// Every tool it offers.
    pub async fn tools(&mut self) -> Result<Vec<Value>, String> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..20 {
            let params = cursor.as_ref().map_or_else(|| json!({}), |cursor| json!({"cursor": cursor}));
            let page = self.request("tools/list", params).await?;
            tools.extend(page.get("tools").and_then(Value::as_array).cloned().unwrap_or_default());
            cursor = page.get("nextCursor").and_then(Value::as_str).map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        Ok(tools)
    }

    pub async fn call(&mut self, tool: &str, arguments: &Value) -> Result<Value, String> {
        self.request("tools/call", json!({"name": tool, "arguments": arguments})).await
    }

    pub async fn stop(mut self) {
        let _ = self.child.kill().await;
    }
}

/// Whether a tool only reads, as its server says.
pub fn reads_only(tool: &Value) -> bool {
    tool.pointer("/annotations/readOnlyHint").and_then(Value::as_bool) == Some(true)
}

/// A call's result as text for the agent, or why it can't be checked.
pub fn text_of(result: &Value) -> Result<(String, bool), String> {
    let mut text = String::new();
    for item in result.get("content").and_then(Value::as_array).into_iter().flatten() {
        match item.get("type").and_then(Value::as_str) {
            Some("text") => {
                text.push_str(item.get("text").and_then(Value::as_str).unwrap_or_default());
                text.push('\n');
            }
            Some(other) => return Err(format!("it answered with {other}, which a data rule can't check")),
            None => {}
        }
    }
    if let Some(structured) = result.get("structuredContent") {
        text.push_str(&structured.to_string());
    }
    Ok((text, result.get("isError").and_then(Value::as_bool).unwrap_or(false)))
}
