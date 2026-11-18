//! GraphQL API, shipped with Silt (ADR 0103): queries and mutations to an
//! API the person sets up per environment. Silt reads each document, so a
//! mutation can't pass as a query, and a data rule refuses what selects or
//! carries location.

use std::collections::BTreeMap;

use silt_access::DataClass;
use silt_plugin_api::http::{Request, send};
use silt_plugin_api::{Block, Call, Card, Context, Effect, Output, Plugin, Tone, Tool, Value, host, json, serde_json};

struct Graphql;

fn need<'a>(input: &'a Value, key: &str) -> Result<&'a str, String> {
    input.get(key).and_then(Value::as_str).map(str::trim).filter(|value| !value.is_empty()).ok_or_else(|| format!("give `{key}`"))
}

fn endpoint(environment: &str) -> Result<String, String> {
    host::setting("endpoint")
        .and_then(|value| value.as_str().map(str::to_owned))
        .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
        .ok_or_else(|| format!("{environment} has no endpoint; set it with `silt plugin set silt.graphql endpoint <url> --environment {environment}`"))
}

fn kept() -> Vec<DataClass> {
    let named = host::setting("data").and_then(|value| serde_json::from_value::<Vec<String>>(value).ok()).unwrap_or_default();
    named.iter().filter(|name| name.as_str() == "location").map(|_| DataClass::Location).collect()
}

/// The request's body, the same bytes when it's prepared and when it runs.
fn body(input: &Value) -> Result<String, String> {
    let query = need(input, "query")?;
    let mut request = json!({"query": query});
    if let Some(variables) = input.get("variables").filter(|variables| !variables.is_null()) {
        request["variables"] = variables.clone();
    }
    if let Some(operation) = input.get("operation").and_then(Value::as_str) {
        request["operationName"] = json!(operation);
    }
    Ok(request.to_string())
}

fn tool(name: &str, action: &str, description: &str) -> Tool {
    Tool {
        name: name.into(),
        description: description.into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "environment": {"type": "string", "description": "The environment, such as prod"},
                "query": {"type": "string", "description": "The GraphQL document"},
                "variables": {"type": "object"},
                "operation": {"type": "string", "description": "The operation to run, if the document has several"}
            },
            "required": ["environment", "query"]
        }),
        action: action.into(),
    }
}

impl Plugin for Graphql {
    fn tools() -> Vec<Tool> {
        vec![
            tool("query", "query", "Read from a GraphQL API: a document of queries only."),
            tool("mutate", "mutate", "Change something through a GraphQL API: a document with a mutation."),
        ]
    }

    fn prepare(tool: &str, input: &Value, _context: &Context) -> Result<Call, String> {
        let environment = need(input, "environment")?;
        let query = need(input, "query")?;
        let classified = silt_access::classify(query)?;
        let action = match (tool, classified.changes()) {
            ("query", true) => return Err("that document changes something: send it with graphql_mutate".into()),
            ("query", false) => "query",
            ("mutate", _) => "mutate",
            (other, _) => return Err(format!("no tool {other}")),
        };
        for class in kept() {
            if let Some(finding) = silt_access::graphql_fields(class, &classified.fields) {
                return Err(format!("{environment} keeps {} from agents, and {}", class.label(), finding.what));
            }
        }
        let url = endpoint(environment)?;
        let body = body(input)?;
        let mut blocks =
            vec![Block::Fields { pairs: vec![("Environment".into(), environment.into()), ("Endpoint".into(), url.clone())] }];
        blocks.push(Block::Code { label: "Document".into(), text: query.into() });
        if let Some(variables) = input.get("variables").filter(|variables| !variables.is_null()) {
            blocks.push(Block::Code {
                label: "Variables".into(),
                text: serde_json::to_string_pretty(variables).unwrap_or_default(),
            });
        }
        if action == "mutate" {
            blocks.push(Block::Note { tone: Tone::Warn, text: "Sent once, exactly as shown.".into() });
        }
        let operation = input.get("operation").and_then(Value::as_str).unwrap_or("").to_owned();
        Ok(Call {
            action: action.into(),
            fields: BTreeMap::from([("environment".into(), environment.into()), ("operation".into(), operation)]),
            card: Card { title: format!("{} {environment}", if action == "mutate" { "Mutate" } else { "Query" }), blocks },
            effects: vec![Effect::Http {
                method: "POST".into(),
                url,
                body_sha256: Some(silt_plugin_api::body_sha256(body.as_bytes())),
            }],
        })
    }

    fn run(_tool: &str, input: &Value, call: &Call, _context: &Context) -> Result<Output, String> {
        let Some(Effect::Http { url, .. }) = call.effects.first() else { return Err("this call names no request".into()) };
        let body = body(input)?;
        let answer = send(&Request {
            method: "POST".into(),
            url: url.clone(),
            headers: vec![("content-type".into(), "application/json".into()), ("accept".into(), "application/json".into())],
            body: Some(body.into_bytes()),
        })?;
        let value: Value = serde_json::from_slice(&answer.body)
            .map_err(|_| format!("the API answered {} with something other than JSON", answer.status))?;
        for class in kept() {
            if let Some(finding) = silt_access::json(class, &value) {
                return Err(format!("the answer was kept from you: it carries {} ({})", class.label(), finding.what));
            }
        }
        Ok(Output { text: serde_json::to_string_pretty(&value).unwrap_or_default(), data: Some(value) })
    }
}

silt_plugin_api::export!(Graphql);
