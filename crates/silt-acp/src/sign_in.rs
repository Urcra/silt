//! The ways an agent says it can be signed in, and its saying it needs one
//! (ADR 0056).

use serde_json::Value;

/// A way to sign the agent in, as it lists them when it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignInMethod {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub how: SignInHow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignInHow {
    /// The agent's own command with these added, run in a terminal.
    Terminal { args: Vec<String>, env: Vec<(String, String)> },
    /// A program the agent names, run in a terminal: the older form, in
    /// `_meta["terminal-auth"]`, as Copilot lists it.
    Program { program: String, args: Vec<String>, env: Vec<(String, String)> },
    /// The agent signs in by itself once asked to (`authenticate`).
    Agent,
}

/// The methods in an `initialize` answer's `authMethods`, as both protocols
/// write them. One Silt can't follow is left out.
pub(crate) fn methods(raw: &Value) -> Vec<SignInMethod> {
    raw.as_array().into_iter().flatten().filter_map(method).collect()
}

fn method(raw: &Value) -> Option<SignInMethod> {
    let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    // v2 calls it `methodId`.
    let id = text(raw, "id").or_else(|| text(raw, "methodId"))?;
    let name = text(raw, "name").unwrap_or_else(|| id.clone());
    let how = match raw.get("type").and_then(Value::as_str) {
        Some("terminal") => SignInHow::Terminal { args: strings(raw.get("args")), env: env(raw.get("env")) },
        None | Some("agent") => match raw.get("_meta").and_then(|meta| meta.get("terminal-auth")) {
            Some(terminal) => SignInHow::Program {
                program: text(terminal, "command")?,
                args: strings(terminal.get("args")),
                env: env(terminal.get("env")),
            },
            None => SignInHow::Agent,
        },
        // Such as an `env_var` method, whose variables Silt doesn't ask for.
        Some(_) => return None,
    };
    Some(SignInMethod {
        id,
        name,
        description: text(raw, "description").map(|text| text.trim().to_owned()).filter(|text| !text.is_empty()),
        how,
    })
}

fn strings(raw: Option<&Value>) -> Vec<String> {
    raw.and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect()
}

/// An `env` given as an object of names, or as a list of `{name, value}`.
fn env(raw: Option<&Value>) -> Vec<(String, String)> {
    match raw {
        Some(Value::Object(names)) => {
            names.iter().filter_map(|(name, value)| Some((name.clone(), value.as_str()?.to_owned()))).collect()
        }
        Some(Value::Array(pairs)) => pairs
            .iter()
            .filter_map(|pair| Some((pair.get("name")?.as_str()?.to_owned(), pair.get("value")?.as_str()?.to_owned())))
            .collect(),
        _ => Vec::new(),
    }
}

/// Whether the agent answered that it needs a sign-in first, as either
/// protocol's error says it.
pub(crate) fn needed(error: &impl serde::Serialize) -> bool {
    const AUTH_REQUIRED: i64 = -32000;
    serde_json::to_value(error).ok().and_then(|error| error.get("code")?.as_i64()) == Some(AUTH_REQUIRED)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn reads_claude_s_terminal_methods_and_copilot_s_older_form() {
        // As Claude's adapter 0.94.0 and Copilot 1.0.105 list them.
        let raw = json!([
            {"description": "Use Claude subscription ", "name": "Claude Subscription", "id": "claude-ai-login",
             "type": "terminal", "args": ["--cli", "auth", "login", "--claudeai"]},
            {"id": "copilot-login", "name": "Log in with Copilot CLI", "description": "Run `copilot login` in the terminal",
             "_meta": {"terminal-auth": {"command": "/opt/copilot/copilot", "args": ["login"], "label": "Copilot Login"}}},
            {"methodId": "oauth", "name": "OAuth"},
            {"id": "key", "name": "API key", "type": "env_var", "vars": [{"name": "KEY"}]},
        ]);
        let methods = methods(&raw);
        assert_eq!(methods.len(), 3, "the env_var method is left out");
        assert_eq!(methods[0].description.as_deref(), Some("Use Claude subscription"));
        assert_eq!(
            methods[0].how,
            SignInHow::Terminal { args: vec!["--cli".into(), "auth".into(), "login".into(), "--claudeai".into()], env: vec![] }
        );
        assert_eq!(
            methods[1].how,
            SignInHow::Program { program: "/opt/copilot/copilot".into(), args: vec!["login".into()], env: vec![] }
        );
        assert_eq!((methods[2].id.as_str(), &methods[2].how), ("oauth", &SignInHow::Agent));
    }

    #[test]
    fn auth_required_is_a_sign_in_in_either_protocol() {
        assert!(needed(&agent_client_protocol::Error::auth_required()));
        assert!(needed(&agent_client_protocol::schema::v2::Error::auth_required()));
        assert!(!needed(&agent_client_protocol::Error::internal_error()));
    }

    #[test]
    fn reads_env_in_either_shape() {
        let object = method(&json!({"id": "a", "type": "terminal", "env": {"MODE": "login"}})).expect("a method");
        let listed =
            method(&json!({"id": "b", "type": "terminal", "env": [{"name": "MODE", "value": "login"}]})).expect("a method");
        for read in [object, listed] {
            assert_eq!(read.how, SignInHow::Terminal { args: vec![], env: vec![("MODE".into(), "login".into())] });
        }
    }
}
