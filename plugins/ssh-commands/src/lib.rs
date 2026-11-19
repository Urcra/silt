//! SSH commands, shipped with Silt (ADR 0104): one command on a host the
//! person named, through the machine's own ssh. What it does there Silt
//! can't check, so the card says so.

use std::collections::BTreeMap;

use silt_plugin_api::{Block, Call, Card, Channel, Context, Effect, Output, Plugin, Tone, Tool, Value, json, ssh};

struct SshCommands;

fn need<'a>(input: &'a Value, key: &str) -> Result<&'a str, String> {
    input.get(key).and_then(Value::as_str).map(str::trim).filter(|value| !value.is_empty()).ok_or_else(|| format!("give `{key}`"))
}

impl Plugin for SshCommands {
    fn tools() -> Vec<Tool> {
        vec![Tool {
            name: "run".into(),
            description: "Run one command on a host of the fleet, such as car-1.fleet.vpn, with optional input on its stdin."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "target": {"type": "string", "description": "The host"},
                    "command": {"type": "string", "description": "The command, as the host's shell runs it"},
                    "input": {"type": "string", "description": "What to give it on stdin"}
                },
                "required": ["target", "command"]
            }),
            action: "run".into(),
        }]
    }

    fn prepare(tool: &str, input: &Value, _context: &Context) -> Result<Call, String> {
        if tool != "run" {
            return Err(format!("no tool {tool}"));
        }
        let target = need(input, "target")?;
        let command = need(input, "command")?;
        let mut blocks = vec![
            Block::Fields { pairs: vec![("Host".into(), target.into())] },
            Block::Code { label: "Command".into(), text: command.into() },
        ];
        if let Some(given) = input.get("input").and_then(Value::as_str).filter(|given| !given.is_empty()) {
            blocks.push(Block::Code { label: "Input".into(), text: given.into() });
        }
        blocks.push(Block::Note { tone: Tone::Warn, text: "Silt can't check what this command does on the host.".into() });
        Ok(Call {
            action: "run".into(),
            fields: BTreeMap::from([("target".into(), target.into())]),
            card: Card { title: format!("Run a command on {target}"), blocks },
            effects: vec![Effect::Opaque { channel: Channel::Ssh, what: format!("`{command}` on {target}") }],
        })
    }

    fn run(_tool: &str, input: &Value, _call: &Call, _context: &Context) -> Result<Output, String> {
        let target = need(input, "target")?;
        let command = need(input, "command")?;
        let stdin = input.get("input").and_then(Value::as_str).unwrap_or("");
        let output = ssh::exec(target, command, stdin.as_bytes())?;
        let mut text = format!("exit {}\n{}", output.status, String::from_utf8_lossy(&output.stdout));
        if !output.stderr.is_empty() {
            text.push_str(&format!("\nstderr:\n{}", String::from_utf8_lossy(&output.stderr)));
        }
        Ok(Output::text(text))
    }
}

silt_plugin_api::export!(SshCommands);
