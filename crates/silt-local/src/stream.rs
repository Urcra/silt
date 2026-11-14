//! Reading a streamed chat completion: server-sent events in, the reply's
//! pieces out.

use serde_json::Value;

/// Splits bytes as they arrive into the `data:` payloads of whole events.
#[derive(Debug, Default)]
pub struct Events {
    pending: Vec<u8>,
}

impl Events {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(bytes);
        let mut payloads = Vec::new();
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\n', '\r']);
            if let Some(data) = line.strip_prefix("data:") {
                payloads.push(data.trim_start().to_owned());
            }
        }
        payloads
    }
}

/// What one streamed chunk adds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Piece {
    pub text: Option<String>,
    /// What a reasoning model thinks before it answers.
    pub thought: Option<String>,
    pub usage: Option<Usage>,
    pub done: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub prompt: u64,
    pub completion: u64,
}

pub fn piece(payload: &str) -> Piece {
    if payload == "[DONE]" {
        return Piece { done: true, ..Piece::default() };
    }
    let Ok(chunk) = serde_json::from_str::<Value>(payload) else {
        return Piece::default();
    };
    let delta = &chunk["choices"][0]["delta"];
    let nonempty = |value: &Value| value.as_str().filter(|text| !text.is_empty()).map(str::to_owned);
    Piece {
        text: nonempty(&delta["content"]),
        thought: nonempty(&delta["reasoning_content"]).or_else(|| nonempty(&delta["reasoning"])),
        usage: chunk["usage"].is_object().then(|| Usage {
            prompt: chunk["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
            completion: chunk["usage"]["completion_tokens"].as_u64().unwrap_or(0),
        }),
        done: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_split_however_the_bytes_arrive() {
        let stream = "data: {\"a\":1}\n\n: a comment\ndata: [DONE]\r\n\r\n";
        let mut events = Events::default();
        let mut payloads = Vec::new();
        for byte in stream.as_bytes() {
            payloads.extend(events.feed(std::slice::from_ref(byte)));
        }
        assert_eq!(payloads, ["{\"a\":1}", "[DONE]"]);
    }

    #[test]
    fn a_chunk_gives_its_text_thought_and_usage() {
        let text = piece(r#"{"choices":[{"delta":{"content":"Hel"}}]}"#);
        assert_eq!(text.text.as_deref(), Some("Hel"));
        let thought = piece(r#"{"choices":[{"delta":{"reasoning_content":"hmm"}}]}"#);
        assert_eq!(thought.thought.as_deref(), Some("hmm"));
        let usage = piece(r#"{"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":3}}"#);
        assert_eq!(usage.usage, Some(Usage { prompt: 10, completion: 3 }));
        assert!(piece("[DONE]").done);
        assert_eq!(piece("garbage"), Piece::default());
        assert_eq!(piece(r#"{"choices":[{"delta":{"content":""}}]}"#).text, None);
    }
}
