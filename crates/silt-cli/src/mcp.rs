//! `silt mcp`: the history tools of ADR 0011, answered by the daemon.

use std::env;
use std::fmt::Write as _;
use std::io;
use std::path::PathBuf;

use exn::{Exn, ResultExt};
use serde::de::DeserializeOwned;
use serde_json::Value;
use silt_agents::conventions::{self, Skill};
use silt_daemon::protocol::{FileTouch, HistoryHit, HistoryPage, InFlight, Request, Response, SearchResult};
use silt_daemon::{client, paths};

use crate::CliError;
use crate::render::short;

const DEFAULT_LIMIT: usize = 40;
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

struct DaemonTools {
    thread: String,
    cwd: PathBuf,
    checkout: PathBuf,
}

pub fn serve(thread: &str, checkout: Option<PathBuf>) -> Result<(), Exn<CliError>> {
    let cwd = env::current_dir().or_raise(|| CliError("cannot read the current directory".into()))?;
    let mut tools = DaemonTools { thread: thread.to_owned(), checkout: checkout.unwrap_or_else(|| cwd.clone()), cwd };
    silt_mcp::serve(io::stdin().lock(), io::stdout().lock(), &mut tools).or_raise(|| CliError("the MCP connection failed".into()))
}

fn ask<T: DeserializeOwned>(request: &Request) -> Result<T, String> {
    match client::send(&paths::socket_path(), request, Some(REQUEST_TIMEOUT)) {
        Ok(Response::Ok { data }) => serde_json::from_value(data).map_err(|error| error.to_string()),
        Ok(Response::Error { message }) => Err(message),
        Err(error) => Err(format!("the Silt daemon is not answering: {error}")),
    }
}

fn text_arg(arguments: &Value, name: &str) -> Result<String, String> {
    arguments.get(name).and_then(Value::as_str).map(str::to_owned).ok_or_else(|| format!("missing argument {name}"))
}

fn number_arg(arguments: &Value, name: &str) -> Option<usize> {
    arguments.get(name).and_then(Value::as_u64).and_then(|value| usize::try_from(value).ok())
}

impl DaemonTools {
    fn skills(&self) -> Vec<Skill> {
        let home = silt_os::home_dir();
        conventions::discover_skills(&self.checkout, home.as_deref())
    }

    fn skill(&self, name: &str) -> Result<Skill, String> {
        self.skills()
            .into_iter()
            .find(|skill| skill.name == name)
            .ok_or_else(|| format!("no skill named {name}; silt_skills lists them"))
    }
}

impl silt_mcp::Backend for DaemonTools {
    fn prompts(&mut self) -> Vec<silt_mcp::Prompt> {
        self.skills()
            .into_iter()
            .filter(|skill| skill.user_invocable)
            .map(|skill| silt_mcp::Prompt { name: skill.name, description: skill.description })
            .collect()
    }

    fn prompt(&mut self, name: &str) -> Result<String, String> {
        let skill = self.skill(name)?;
        let body = skill.body().map_err(|error| error.to_string())?;
        Ok(format!("Use the {name} skill (its files are in {}):\n\n{body}", skill.dir.display()))
    }

    fn call(&mut self, tool: &str, arguments: &Value) -> Result<String, String> {
        match tool {
            "silt_skills" => {
                let skills: Vec<Skill> = self.skills().into_iter().filter(|skill| skill.model_invocable).collect();
                if skills.is_empty() {
                    return Ok("No skills in .silt/skills, .agents/skills or ~/.agents/skills.".into());
                }
                let mut out = String::from("Skills (load one with silt_use_skill):\n");
                for skill in skills {
                    let _ = writeln!(out, "- {}: {}", skill.name, skill.description);
                }
                return Ok(out);
            }
            "silt_use_skill" => {
                let skill = self.skill(&text_arg(arguments, "name")?)?;
                let body = skill.body().map_err(|error| error.to_string())?;
                let mut files: Vec<String> = std::fs::read_dir(&skill.dir)
                    .map(|entries| {
                        entries
                            .flatten()
                            .map(|entry| entry.file_name().to_string_lossy().into_owned())
                            .filter(|name| name != "SKILL.md")
                            .collect()
                    })
                    .unwrap_or_default();
                files.sort();
                let mut out = format!("Skill {} from {}.", skill.name, skill.dir.display());
                if !files.is_empty() {
                    let _ = write!(out, " Its other files: {}.", files.join(", "));
                }
                let _ = write!(out, "\n\n{body}");
                return Ok(out);
            }
            _ => {}
        }
        let thread = arguments.get("thread").and_then(Value::as_str).map_or_else(|| self.thread.clone(), str::to_owned);
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
                let touches: Vec<FileTouch> = ask(&Request::FileHistory { thread, path: text_arg(arguments, "path")? })?;
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
                let found: Vec<InFlight> = ask(&Request::InFlight { cwd: self.cwd.clone(), path: text_arg(arguments, "path")? })?;
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
        // An agent must not take a rewound turn as part of the conversation (ADR 0029).
        let rewound = if entry.rewound { " (rewound by a revert, no longer part of the conversation)" } else { "" };
        let _ = writeln!(out, "[{}] {}{origin}{rewound}: {}", entry.index, entry.kind, entry.text.trim());
    }
    out
}

fn clip(text: &str) -> String {
    let text = text.trim().replace('\n', " ");
    if text.chars().count() > 160 { text.chars().take(160).collect::<String>() + "…" } else { text }
}
