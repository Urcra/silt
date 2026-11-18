//! Folders, shipped with Silt (ADR 0098): an agent asks for a folder of
//! this machine, and Silt shows it from the agent's next restart once the
//! workspace's rules or the person allow it.

use std::collections::BTreeMap;

use silt_plugin_api::{Block, Call, Card, Context, Effect, Item, Output, Plugin, Tone, Tool, Value, json, reach};

struct Folders;

fn asked(input: &Value) -> Result<(String, bool, String), String> {
    let path = input.get("path").and_then(Value::as_str).filter(|path| !path.trim().is_empty()).ok_or("give `path`")?;
    let writable = input.get("writable").and_then(Value::as_bool).unwrap_or(false);
    let reason = input.get("reason").and_then(Value::as_str).unwrap_or("").trim().to_owned();
    Ok((path.trim().to_owned(), writable, reason))
}

fn access(writable: bool) -> &'static str {
    if writable { "read and write" } else { "read-only" }
}

impl Plugin for Folders {
    fn tools() -> Vec<Tool> {
        vec![Tool {
            name: "request".into(),
            description: "Ask for a folder of this machine outside your checkout, such as another repo you need to \
                          read or change. Give its path, whether you need to write there, and why. Once it's allowed, \
                          end your turn: you're restarted with the folder at the same path and your session goes on."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "The folder, absolute or starting with ~/"},
                    "writable": {"type": "boolean", "description": "Whether you need to change files there", "default": false},
                    "reason": {"type": "string", "description": "Why you need it, for the person deciding"}
                },
                "required": ["path", "reason"]
            }),
            action: "request".into(),
        }]
    }

    fn prepare(tool: &str, input: &Value, _context: &Context) -> Result<Call, String> {
        if tool != "request" {
            return Err(format!("no tool {tool}"));
        }
        let (path, writable, reason) = asked(input)?;
        let folder = reach::resolve(&path, writable)?;
        let mut pairs = vec![("Folder".to_owned(), folder.real.clone()), ("Access".to_owned(), access(writable).to_owned())];
        if folder.asked != folder.real {
            pairs.insert(1, ("Asked as".to_owned(), folder.asked.clone()));
        }
        let mut blocks = vec![Block::Fields { pairs }];
        if !folder.repos.is_empty() {
            blocks.push(Block::List {
                label: "Git repos in it, their config and hooks kept read-only".into(),
                items: folder.repos.iter().map(|repo| Item { key: None, text: repo.clone() }).collect(),
            });
        }
        if !reason.is_empty() {
            blocks.push(Block::Note { tone: Tone::Neutral, text: format!("The agent says: {reason}") });
        }
        blocks.push(Block::Note {
            tone: if writable { Tone::Warn } else { Tone::Neutral },
            text: if writable {
                "What the agent changes there isn't captured with the thread. It's restarted with the folder when its turn ends."
                    .into()
            } else {
                "The agent is restarted with the folder when its turn ends, and its session goes on.".into()
            },
        });
        Ok(Call {
            action: "request".into(),
            fields: BTreeMap::from([("path".to_owned(), folder.real.clone()), ("writable".to_owned(), writable.to_string())]),
            card: Card { title: format!("Show {} to the agent, {}", folder.real, access(writable)), blocks },
            effects: vec![Effect::Reach { path: folder.real, writable }],
        })
    }

    fn run(tool: &str, _input: &Value, call: &Call, _context: &Context) -> Result<Output, String> {
        if tool != "request" {
            return Err(format!("no tool {tool}"));
        }
        let Some(Effect::Reach { path, writable }) = call.effects.first() else {
            return Err("this call names no folder".into());
        };
        reach::grant(path, *writable)?;
        Ok(Output::text(format!(
            "{path} is granted, {}. End your turn now: you're restarted with it at {path}, and your session goes on.",
            access(*writable)
        )))
    }
}

silt_plugin_api::export!(Folders);
