//! `silt mcp`: the history tools of ADR 0011, answered by the daemon.

use std::env;
use std::fmt::Write as _;
use std::io;
use std::path::PathBuf;

use exn::{Exn, ResultExt};
use serde::de::DeserializeOwned;
use serde_json::Value;
use silt_daemon::protocol::{FileTouch, HistoryHit, HistoryPage, InFlight, Request, Response, SearchResult};
use silt_daemon::{client, paths};

use crate::CliError;

const DEFAULT_LIMIT: usize = 40;
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

struct DaemonTools {
    thread: String,
    cwd: PathBuf,
}

pub fn serve(thread: &str) -> Result<(), Exn<CliError>> {
    let mut tools = DaemonTools {
        thread: thread.to_owned(),
        cwd: env::current_dir().or_raise(|| CliError("cannot read the current directory".into()))?,
    };
    silt_mcp::serve(io::stdin().lock(), io::stdout().lock(), &mut tools)
        .or_raise(|| CliError("the MCP connection failed".into()))
}

fn ask<T: DeserializeOwned>(request: &Request) -> Result<T, String> {
    match client::send(&paths::socket_path(), request, REQUEST_TIMEOUT) {
        Ok(Response::Ok { data }) => serde_json::from_value(data).map_err(|error| error.to_string()),
        Ok(Response::Error { message }) => Err(message),
        Err(error) => Err(format!("the Silt daemon is not answering: {error}")),
    }
}

fn text_arg(arguments: &Value, name: &str) -> Result<String, String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("missing argument {name}"))
}

fn number_arg(arguments: &Value, name: &str) -> Option<usize> {
    arguments
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

impl silt_mcp::Backend for DaemonTools {
    fn call(&mut self, tool: &str, arguments: &Value) -> Result<String, String> {
        let thread = arguments
            .get("thread")
            .and_then(Value::as_str)
            .map_or_else(|| self.thread.clone(), str::to_owned);
        match tool {
            "silt_history_read" | "silt_fork_read" => {
                let page: HistoryPage = ask(&Request::HistoryRead {
                    thread,
                    offset: number_arg(arguments, "offset").unwrap_or(0),
                    limit: number_arg(arguments, "limit").unwrap_or(DEFAULT_LIMIT),
                })?;
                Ok(render_page(&page))
            }
            "silt_history_search" => {
                let hits: Vec<HistoryHit> = ask(&Request::HistorySearch {
                    thread,
                    query: text_arg(arguments, "query")?,
                    limit: u32::try_from(number_arg(arguments, "limit").unwrap_or(10)).unwrap_or(10),
                })?;
                if hits.is_empty() {
                    return Ok("No matches in this thread's history.".into());
                }
                let mut out = String::from("Matches (read around one with silt_history_read, offset a few entries before it):\n");
                for hit in hits {
                    let _ = writeln!(out, "[{}] {}: {}", hit.index, hit.kind, hit.snippet);
                }
                Ok(out)
            }
            "silt_file_history" => {
                let touches: Vec<FileTouch> = ask(&Request::FileHistory {
                    thread,
                    path: text_arg(arguments, "path")?,
                })?;
                if touches.is_empty() {
                    return Ok("No recorded changes to this file in this thread.".into());
                }
                let mut out = String::new();
                for touch in touches {
                    let who = match (&touch.tool_use_id, touch.by_agent) {
                        (Some(tool), _) => format!("tool call {tool}"),
                        (None, true) => "the agent".to_owned(),
                        (None, false) => "edits outside tool calls".to_owned(),
                    };
                    let prompt = touch.prompt.as_deref().map(clip).unwrap_or("(before any prompt)".into());
                    let _ = writeln!(
                        out,
                        "thread {}: {who}, +{} -{} chars, in the turn for: {prompt}",
                        short(&touch.thread),
                        touch.inserted,
                        touch.deleted
                    );
                }
                Ok(out)
            }
            "silt_thread_search" => {
                let hits: Vec<SearchResult> = ask(&Request::Search {
                    query: text_arg(arguments, "query")?,
                    limit: u32::try_from(number_arg(arguments, "limit").unwrap_or(20)).unwrap_or(20),
                    filters: Default::default(),
                })?;
                if hits.is_empty() {
                    return Ok("No matches.".into());
                }
                let mut out = String::from("Matches (read a thread with silt_fork_read):\n");
                for hit in hits {
                    let _ = writeln!(out, "thread {} {}: {}", hit.thread, hit.kind, hit.snippet);
                }
                Ok(out)
            }
            "silt_in_flight" => {
                let found: Vec<InFlight> = ask(&Request::InFlight {
                    cwd: self.cwd.clone(),
                    path: text_arg(arguments, "path")?,
                })?;
                if found.is_empty() {
                    return Ok("No other thread is changing this path.".into());
                }
                let mut out = String::new();
                for item in found {
                    let owner = if item.owner.is_empty() { "you" } else { &item.owner };
                    let _ = writeln!(out, "thread {} ({owner}): {}", item.thread, item.files.join(", "));
                }
                Ok(out)
            }
            other => Err(format!("unknown tool {other}")),
        }
    }
}

fn render_page(page: &HistoryPage) -> String {
    let mut out = format!("Thread {} has {} history entries.", page.thread, page.total);
    match (page.entries.first(), page.entries.last()) {
        (Some(first), Some(last)) => {
            let _ = writeln!(out, " Entries {} to {}:", first.index, last.index);
        }
        _ => out.push_str(" No entries in this range.\n"),
    }
    for entry in &page.entries {
        let origin = match entry.origin.as_str() {
            "own" => String::new(),
            other => format!(" ({other} from {})", short(&entry.thread)),
        };
        let _ = writeln!(out, "[{}] {}{origin}: {}", entry.index, entry.kind, entry.text.trim());
    }
    out
}

fn clip(text: &str) -> String {
    let text = text.trim().replace('\n', " ");
    if text.chars().count() > 160 {
        text.chars().take(160).collect::<String>() + "…"
    } else {
        text
    }
}

fn short(thread: &str) -> &str {
    &thread[thread.len().saturating_sub(12)..]
}
