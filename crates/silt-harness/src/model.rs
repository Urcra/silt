//! The conversation in the harness's own terms, what it becomes for
//! Anthropic's Messages API or an OpenAI-compatible chat completions API,
//! and the call itself.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text { text: String },
    ToolUse { id: String, name: String, input: Value },
    ToolResult { id: String, output: String, is_error: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub parts: Vec<Part>,
}

/// Why the model stopped answering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stop {
    #[default]
    Done,
    /// It ran out of room for its answer.
    MaxTokens,
    Refused,
}

/// What the model answered: text and the tools it wants run.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Reply {
    pub parts: Vec<Part>,
    pub stop: Stop,
}

impl Reply {
    pub fn tool_uses(&self) -> impl Iterator<Item = (&str, &str, &Value)> {
        self.parts.iter().filter_map(|part| match part {
            Part::ToolUse { id, name, input } => Some((id.as_str(), name.as_str(), input)),
            _ => None,
        })
    }
}

pub type Tool = (&'static str, &'static str, Value);

const MAX_TOKENS: u32 = 16_384;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Api {
    Anthropic,
    OpenAi,
}

#[derive(Debug, Clone)]
pub struct Backend {
    pub api: Api,
    pub base_url: String,
    pub key: String,
    pub model: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error(
        "no model is configured: set ANTHROPIC_API_KEY, or OPENAI_API_KEY with SILT_HARNESS_MODEL (and OPENAI_BASE_URL for OpenRouter or a local server)"
    )]
    NotConfigured,
    #[error("the model API failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("the model API answered {status}: {body}")]
    Status { status: u16, body: String },
}

impl Backend {
    /// The backend the environment chooses: Anthropic when it has a key,
    /// else an OpenAI-compatible API.
    pub fn from_env(env: impl Fn(&str) -> Option<String>) -> Result<Self, ModelError> {
        let model = env("SILT_HARNESS_MODEL");
        if let Some(key) = env("ANTHROPIC_API_KEY") {
            return Ok(Self {
                api: Api::Anthropic,
                base_url: env("ANTHROPIC_BASE_URL").unwrap_or_else(|| "https://api.anthropic.com".into()),
                key,
                model: model.unwrap_or_else(|| "claude-opus-5-5".into()),
            });
        }
        match (env("OPENAI_API_KEY"), model) {
            (Some(key), Some(model)) => Ok(Self {
                api: Api::OpenAi,
                base_url: env("OPENAI_BASE_URL").unwrap_or_else(|| "https://api.openai.com/v1".into()),
                key,
                model,
            }),
            _ => Err(ModelError::NotConfigured),
        }
    }

    pub async fn complete(
        &self,
        client: &reqwest::Client,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<Reply, ModelError> {
        let base = self.base_url.trim_end_matches('/');
        let request = match self.api {
            Api::Anthropic => client
                .post(format!("{base}/v1/messages"))
                .header("x-api-key", &self.key)
                .header("anthropic-version", "2023-06-01")
                .json(&anthropic_request(&self.model, system, messages, tools)),
            Api::OpenAi => {
                let mut body = openai_request(&self.model, system, messages, tools);
                // OpenAI's reasoning models only take the newer name, which
                // other compatible servers may not know.
                if base.contains("api.openai.com") {
                    body["max_completion_tokens"] = body["max_tokens"].take();
                    body.as_object_mut().map(|fields| fields.remove("max_tokens"));
                }
                client.post(format!("{base}/chat/completions")).bearer_auth(&self.key).json(&body)
            }
        };
        let response = request.send().await?;
        let status = response.status();
        if !status.is_success() {
            return Err(ModelError::Status { status: status.as_u16(), body: response.text().await.unwrap_or_default() });
        }
        let body: Value = response.json().await?;
        Ok(match self.api {
            Api::Anthropic => anthropic_reply(&body),
            Api::OpenAi => openai_reply(&body),
        })
    }
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

pub fn anthropic_request(model: &str, system: &str, messages: &[Message], tools: &[Tool]) -> Value {
    let messages: Vec<Value> = messages
        .iter()
        .map(|message| {
            let content: Vec<Value> = message
                .parts
                .iter()
                .map(|part| match part {
                    Part::Text { text } => json!({"type": "text", "text": text}),
                    Part::ToolUse { id, name, input } => json!({"type": "tool_use", "id": id, "name": name, "input": input}),
                    Part::ToolResult { id, output, is_error } => {
                        json!({"type": "tool_result", "tool_use_id": id, "content": output, "is_error": is_error})
                    }
                })
                .collect();
            json!({"role": role_name(message.role), "content": content})
        })
        .collect();
    let tools: Vec<Value> = tools
        .iter()
        .map(|(name, description, schema)| json!({"name": name, "description": description, "input_schema": schema}))
        .collect();
    json!({"model": model, "max_tokens": MAX_TOKENS, "system": system, "messages": messages, "tools": tools})
}

pub fn anthropic_reply(body: &Value) -> Reply {
    let parts = body["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| match block["type"].as_str()? {
            "text" => Some(Part::Text { text: block["text"].as_str()?.to_owned() }),
            "tool_use" => Some(Part::ToolUse {
                id: block["id"].as_str()?.to_owned(),
                name: block["name"].as_str()?.to_owned(),
                input: block["input"].clone(),
            }),
            _ => None,
        })
        .collect();
    let stop = match body["stop_reason"].as_str() {
        Some("max_tokens") => Stop::MaxTokens,
        Some("refusal") => Stop::Refused,
        _ => Stop::Done,
    };
    Reply { parts, stop }
}

pub fn openai_request(model: &str, system: &str, messages: &[Message], tools: &[Tool]) -> Value {
    let mut out = vec![json!({"role": "system", "content": system})];
    for message in messages {
        let text: String = message
            .parts
            .iter()
            .filter_map(|part| match part {
                Part::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        match message.role {
            Role::Assistant => {
                let calls: Vec<Value> = message
                    .parts
                    .iter()
                    .filter_map(|part| match part {
                        Part::ToolUse { id, name, input } => Some(json!({
                            "id": id,
                            "type": "function",
                            "function": {"name": name, "arguments": input.to_string()},
                        })),
                        _ => None,
                    })
                    .collect();
                let mut assistant = json!({"role": "assistant", "content": text});
                if !calls.is_empty() {
                    assistant["tool_calls"] = Value::Array(calls);
                }
                out.push(assistant);
            }
            Role::User => {
                for part in &message.parts {
                    if let Part::ToolResult { id, output, .. } = part {
                        out.push(json!({"role": "tool", "tool_call_id": id, "content": output}));
                    }
                }
                if !text.is_empty() {
                    out.push(json!({"role": "user", "content": text}));
                }
            }
        }
    }
    let tools: Vec<Value> = tools
        .iter()
        .map(|(name, description, schema)| {
            json!({"type": "function", "function": {"name": name, "description": description, "parameters": schema}})
        })
        .collect();
    json!({"model": model, "max_tokens": MAX_TOKENS, "messages": out, "tools": tools})
}

pub fn openai_reply(body: &Value) -> Reply {
    let stop = match body["choices"][0]["finish_reason"].as_str() {
        Some("length") => Stop::MaxTokens,
        Some("content_filter") => Stop::Refused,
        _ => Stop::Done,
    };
    let message = &body["choices"][0]["message"];
    let mut parts = Vec::new();
    if let Some(text) = message["content"].as_str().filter(|text| !text.is_empty()) {
        parts.push(Part::Text { text: text.to_owned() });
    }
    for call in message["tool_calls"].as_array().into_iter().flatten() {
        let arguments = call["function"]["arguments"].as_str().unwrap_or("{}");
        parts.push(Part::ToolUse {
            id: call["id"].as_str().unwrap_or_default().to_owned(),
            name: call["function"]["name"].as_str().unwrap_or_default().to_owned(),
            input: serde_json::from_str(arguments).unwrap_or(Value::Null),
        });
    }
    Reply { parts, stop }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conversation() -> Vec<Message> {
        vec![
            Message { role: Role::User, parts: vec![Part::Text { text: "fix it".into() }] },
            Message {
                role: Role::Assistant,
                parts: vec![
                    Part::Text { text: "Reading.".into() },
                    Part::ToolUse { id: "t1".into(), name: "read_file".into(), input: json!({"path": "a"}) },
                ],
            },
            Message {
                role: Role::User,
                parts: vec![Part::ToolResult { id: "t1".into(), output: "text".into(), is_error: false }],
            },
        ]
    }

    #[test]
    fn the_environment_picks_the_backend() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| pairs.iter().find(|(key, _)| *key == name).map(|(_, value)| (*value).to_owned())
        };
        let anthropic = Backend::from_env(env(&[("ANTHROPIC_API_KEY", "k")])).expect("configured");
        assert_eq!((anthropic.api, anthropic.model.as_str()), (Api::Anthropic, "claude-opus-5-5"));
        let router = Backend::from_env(env(&[
            ("OPENAI_API_KEY", "k"),
            ("OPENAI_BASE_URL", "https://openrouter.ai/api/v1"),
            ("SILT_HARNESS_MODEL", "some/model"),
        ]))
        .expect("configured");
        assert_eq!((router.api, router.base_url.as_str()), (Api::OpenAi, "https://openrouter.ai/api/v1"));
        assert!(Backend::from_env(env(&[("OPENAI_API_KEY", "k")])).is_err(), "OpenAI-compatible APIs need a model");
    }

    #[test]
    fn conversations_become_anthropic_requests_and_back() {
        let request = anthropic_request("m", "sys", &conversation(), &[]);
        assert_eq!(request["messages"][1]["content"][1]["type"], "tool_use");
        assert_eq!(request["messages"][2]["content"][0]["tool_use_id"], "t1");
        let reply = anthropic_reply(&json!({"content": [
            {"type": "text", "text": "ok"},
            {"type": "tool_use", "id": "t2", "name": "run", "input": {"command": "ls"}},
        ]}));
        assert_eq!(reply.tool_uses().collect::<Vec<_>>(), vec![("t2", "run", &json!({"command": "ls"}))]);
    }

    #[test]
    fn conversations_become_openai_requests_and_back() {
        let request = openai_request("m", "sys", &conversation(), &[]);
        let messages = request["messages"].as_array().expect("messages");
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[2]["tool_calls"][0]["function"]["arguments"], r#"{"path":"a"}"#);
        assert_eq!(messages[3], json!({"role": "tool", "tool_call_id": "t1", "content": "text"}));
        let reply = openai_reply(&json!({"choices": [{"message": {
            "content": null,
            "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "list_files", "arguments": "{}"}}],
        }}]}));
        assert_eq!(reply.tool_uses().count(), 1);
    }

    #[test]
    fn stop_reasons_come_through() {
        assert_eq!(anthropic_reply(&json!({"content": [], "stop_reason": "max_tokens"})).stop, Stop::MaxTokens);
        assert_eq!(anthropic_reply(&json!({"content": [], "stop_reason": "refusal"})).stop, Stop::Refused);
        assert_eq!(openai_reply(&json!({"choices": [{"finish_reason": "length", "message": {}}]})).stop, Stop::MaxTokens);
        assert_eq!(openai_reply(&json!({"choices": [{"finish_reason": "stop", "message": {"content": "hi"}}]})).stop, Stop::Done);
    }
}
