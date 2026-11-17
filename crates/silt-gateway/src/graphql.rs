//! GraphQL a sandbox sends straight to an API through the gateway (ADR
//! 0093), for an environment a plugin lets agents reach directly, such as
//! staging: reads go, a change is refused unless its rule allows it, and
//! what a data rule keeps from agents is refused both ways. What can't be
//! read, a persisted query's hash, an upload, a subscription's socket, is
//! refused rather than passed unread.

use std::sync::Arc;

use serde_json::Value;
use silt_access::{DataClass, Schema};

/// A GraphQL endpoint the gateway reads requests to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphqlRule {
    pub host: String,
    /// Its path, such as `/graphql`.
    pub path: String,
    /// Whether changes may go straight to it.
    pub changes: bool,
    pub data: Vec<DataClass>,
    /// Its schema, for data rules that see past field names.
    pub schema: Option<Arc<Schema>>,
    /// Where changes go instead, for the refusal to say.
    pub plugin: String,
}

impl GraphqlRule {
    pub fn covers(&self, host: &str, path: &str) -> bool {
        self.host.eq_ignore_ascii_case(host) && path.trim_end_matches('/') == self.path.trim_end_matches('/')
    }

    /// Checks one document.
    fn document(&self, document: &str) -> Result<(), String> {
        let classified = silt_access::classify(document)?;
        if classified.changes() && !self.changes {
            return Err(format!(
                "it changes something, which goes through the {} plugin's mutate tool, where the person approves it",
                self.plugin
            ));
        }
        for class in &self.data {
            if let Some(finding) = silt_access::graphql_fields(*class, &classified.fields) {
                return Err(format!("{}, and {} is kept from agents here", finding.what, class.label()));
            }
            if let Some(schema) = &self.schema
                && let Some(finding) = schema.finding(*class, document)?
            {
                return Err(format!("{}, and it's kept from agents here", finding.what));
            }
        }
        Ok(())
    }

    /// Checks a request: `query` is its URL's query string, `body` all of
    /// its body.
    pub fn request(&self, method: &str, query: Option<&str>, content_type: Option<&str>, body: &[u8]) -> Result<(), String> {
        if method.eq_ignore_ascii_case("GET") {
            let fields = form(query.unwrap_or_default());
            let document = fields.iter().find(|(name, _)| name == "query").map(|(_, value)| value.as_str());
            let Some(document) = document else {
                return Err("a GET without a query, such as a persisted query's hash, can't be read".into());
            };
            return self.document(document);
        }
        if !method.eq_ignore_ascii_case("POST") {
            return Err(format!("{method} isn't how a GraphQL request is sent"));
        }
        let kind = content_type.unwrap_or_default().split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
        match kind.as_str() {
            "application/graphql" => self.document(std::str::from_utf8(body).map_err(|_| "the document isn't text")?),
            "application/json" | "" => {
                let value: Value = serde_json::from_slice(body).map_err(|_| "the body isn't JSON")?;
                let operations = match value {
                    Value::Array(batch) => batch,
                    one => vec![one],
                };
                if operations.is_empty() {
                    return Err("an empty batch".into());
                }
                for operation in &operations {
                    let document = operation
                        .get("query")
                        .and_then(Value::as_str)
                        .ok_or("a request without its document, such as a persisted query's hash, can't be read")?;
                    self.document(document)?;
                }
                Ok(())
            }
            other => Err(format!("{other} can't be read, so isn't let through")),
        }
    }

    /// Checks an answer before the sandbox gets it.
    pub fn answer(&self, body: &[u8]) -> Result<(), String> {
        if self.data.is_empty() {
            return Ok(());
        }
        let value: Value = serde_json::from_slice(body).map_err(|_| "its answer isn't JSON, which a data rule can't check")?;
        for class in &self.data {
            if let Some(finding) = silt_access::json(*class, &value) {
                return Err(format!("the answer was kept: {}, and {} is kept from agents here", finding.what, class.label()));
            }
        }
        Ok(())
    }
}

/// `query`'s fields, `+` and `%XX` decoded.
fn form(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode(name), decode(value))
        })
        .collect()
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'+' => out.push(b' '),
            b'%' if at + 2 < bytes.len() => {
                match std::str::from_utf8(&bytes[at + 1..at + 3]).ok().and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                    Some(byte) => {
                        out.push(byte);
                        at += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            byte => out.push(byte),
        }
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(changes: bool) -> GraphqlRule {
        GraphqlRule {
            host: "staging.fleet.example".into(),
            path: "/graphql".into(),
            changes,
            data: vec![DataClass::Location],
            schema: None,
            plugin: "fleet-api".into(),
        }
    }

    #[test]
    fn reads_go_and_changes_go_through_the_plugin() {
        let strict = rule(false);
        assert!(strict.request("POST", None, Some("application/json"), br#"{"query": "{ vehicle { id } }"}"#).is_ok());
        let change = strict.request("POST", None, Some("application/json"), br#"{"query": "mutation { setOta { id } }"}"#);
        assert!(change.unwrap_err().contains("fleet-api plugin's mutate tool"));
        assert!(rule(true).request("POST", None, None, br#"{"query": "mutation { setOta { id } }"}"#).is_ok());
        let batch = br#"[{"query": "{ a }"}, {"query": "mutation { b }"}]"#;
        assert!(strict.request("POST", None, Some("application/json"), batch).is_err(), "a change hidden in a batch");
    }

    #[test]
    fn what_cant_be_read_is_refused() {
        let open = rule(true);
        let persisted = br#"{"extensions": {"persistedQuery": {"version": 1, "sha256Hash": "abc"}}}"#;
        assert!(open.request("POST", None, Some("application/json"), persisted).is_err());
        assert!(open.request("GET", Some("extensions=%7B%7D"), None, b"").is_err());
        assert!(open.request("POST", None, Some("multipart/form-data; boundary=x"), b"").is_err());
        assert!(open.request("GET", Some("query=%7B+vehicle+%7B+id+%7D+%7D"), None, b"").is_ok());
        assert!(open.request("GET", Some("query=mutation+%7B+a+%7D"), None, b"").is_ok(), "changes allowed here");
        assert!(rule(false).request("GET", Some("query=mutation+%7B+a+%7D"), None, b"").is_err());
    }

    #[test]
    fn location_is_refused_both_ways() {
        let rule = rule(false);
        let asks = rule.request("POST", None, None, br#"{"query": "{ vehicle { lastPosition { lat } } }"}"#);
        assert!(asks.unwrap_err().contains("location"));
        assert!(rule.answer(br#"{"data": {"vehicle": {"id": 1}}}"#).is_ok());
        assert!(rule.answer(br#"{"data": {"vehicle": {"spot": {"latitude": 59.33, "longitude": 18.07}}}}"#).is_err());
        assert_eq!(decode("a%20b+c%2"), "a b c%2");
    }
}
