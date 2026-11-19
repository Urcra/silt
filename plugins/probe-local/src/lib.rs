//! The probe for the capabilities whose calls Silt can't check: TCP, SSH
//! and a local MCP server, for the daemon's tests. Not shipped.

use std::collections::BTreeMap;

use silt_plugin_api::{Block, Call, Card, Channel, Context, Effect, Output, Plugin, Tool, Value, json, mcp, sockets, ssh};

struct ProbeLocal;

fn tool(name: &str) -> Tool {
    Tool {
        name: name.into(),
        description: format!("The local probe's {name}"),
        input_schema: json!({"type": "object"}),
        action: "reach".into(),
    }
}

fn text<'a>(input: &'a Value, key: &str) -> &'a str {
    input.get(key).and_then(Value::as_str).unwrap_or_default()
}

impl Plugin for ProbeLocal {
    fn tools() -> Vec<Tool> {
        ["socket", "ssh", "mcp_list", "mcp_peek"].into_iter().map(tool).collect()
    }

    fn prepare(tool: &str, _input: &Value, _context: &Context) -> Result<Call, String> {
        let channel = match tool {
            "socket" => Channel::Sockets,
            "ssh" => Channel::Ssh,
            "mcp_list" | "mcp_peek" => Channel::McpProcess,
            other => return Err(format!("no tool {other}")),
        };
        Ok(Call {
            action: "reach".into(),
            fields: BTreeMap::new(),
            card: Card { title: tool.into(), blocks: vec![Block::Fields { pairs: vec![("probe".into(), tool.into())] }] },
            effects: vec![Effect::Opaque { channel, what: format!("the probe's {tool}") }],
        })
    }

    fn run(tool: &str, input: &Value, _call: &Call, _context: &Context) -> Result<Output, String> {
        match tool {
            "socket" => {
                let port = input.get("port").and_then(Value::as_u64).and_then(|port| u16::try_from(port).ok()).unwrap_or(80);
                let connection = sockets::connect(text(input, "host"), port)?;
                connection.send(text(input, "send").as_bytes())?;
                Ok(Output::text(String::from_utf8_lossy(&connection.receive(1024)?).into_owned()))
            }
            "ssh" => {
                let output = ssh::exec(text(input, "host"), text(input, "command"), b"from the plugin")?;
                Ok(Output::text(format!("{}: {}", output.status, String::from_utf8_lossy(&output.stdout))))
            }
            "mcp_list" => Ok(Output::text(mcp::tools()?)),
            "mcp_peek" => Ok(Output::text(mcp::call("peek", &input.to_string())?)),
            other => Err(format!("no tool {other}")),
        }
    }
}

silt_plugin_api::export!(ProbeLocal);
