//! What a GGUF file's header says about its model: enough to size its
//! context and choose its defaults. A downloaded file is untrusted, so every
//! length is bounded before it's used.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

const MAGIC: &[u8; 4] = b"GGUF";
const MOST_ENTRIES: u64 = 1 << 16;
const MOST_STRING_BYTES: u64 = 1 << 24;
const MOST_ARRAY_ITEMS: u64 = 1 << 24;
/// Strings kept from the header; longer ones are only noted as there.
const KEPT_STRING_BYTES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GgufError {
    #[error("not a GGUF file")]
    NotGguf,
    #[error("GGUF version {0} isn't supported")]
    Version(u32),
    /// The header goes on past the bytes given; read more and try again.
    #[error("the GGUF header goes on past what was read")]
    Truncated,
    #[error("the GGUF header is malformed: {0}")]
    Malformed(&'static str),
}

/// The header's facts about the model.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub architecture: Option<String>,
    pub name: Option<String>,
    pub context_length: Option<u64>,
    pub block_count: Option<u64>,
    pub head_count: Option<u64>,
    /// The most key-value heads any layer has.
    pub head_count_kv: Option<u64>,
    pub embedding_length: Option<u64>,
    pub key_length: Option<u64>,
    pub value_length: Option<u64>,
    pub chat_template: bool,
}

impl Meta {
    /// Bytes of an f16 KV cache per token of context, where the header says
    /// enough to tell.
    pub fn kv_bytes_per_token(&self) -> Option<u64> {
        let layers = self.block_count?;
        let heads = self.head_count_kv.or(self.head_count)?;
        let head_size = self.embedding_length.zip(self.head_count).and_then(|(width, heads)| width.checked_div(heads));
        let key = self.key_length.or(head_size)?;
        let value = self.value_length.or(head_size)?;
        layers.checked_mul(heads)?.checked_mul(key.checked_add(value)?)?.checked_mul(2)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Value {
    Unsigned(u64),
    Signed(i64),
    Text(String),
    /// A string too long to keep.
    LongText,
    Other,
    /// The largest of an array of integers.
    Largest(Option<u64>),
}

impl Value {
    fn number(&self) -> Option<u64> {
        match self {
            Value::Unsigned(number) | Value::Largest(Some(number)) => Some(*number),
            Value::Signed(number) => u64::try_from(*number).ok(),
            _ => None,
        }
    }

    fn text(&self) -> Option<String> {
        match self {
            Value::Text(text) => Some(text.clone()),
            _ => None,
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: u64) -> Result<&'a [u8], GgufError> {
        let count = usize::try_from(count).map_err(|_| GgufError::Malformed("a length doesn't fit"))?;
        let end = self.at.checked_add(count).ok_or(GgufError::Malformed("a length overflows"))?;
        let bytes = self.bytes.get(self.at..end).ok_or(GgufError::Truncated)?;
        self.at = end;
        Ok(bytes)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], GgufError> {
        Ok(self.take(N as u64)?.try_into().expect("took N bytes"))
    }

    fn u32(&mut self) -> Result<u32, GgufError> {
        self.array().map(u32::from_le_bytes)
    }

    fn u64(&mut self) -> Result<u64, GgufError> {
        self.array().map(u64::from_le_bytes)
    }

    fn string(&mut self) -> Result<Value, GgufError> {
        let length = self.u64()?;
        if length > MOST_STRING_BYTES {
            return Err(GgufError::Malformed("a string is too long"));
        }
        let bytes = self.take(length)?;
        if bytes.len() > KEPT_STRING_BYTES {
            return Ok(Value::LongText);
        }
        Ok(std::str::from_utf8(bytes).map_or(Value::Other, |text| Value::Text(text.to_owned())))
    }

    fn scalar(&mut self, kind: u32) -> Result<Value, GgufError> {
        Ok(match kind {
            0 => Value::Unsigned(u64::from(self.array::<1>()?[0])),
            1 => Value::Signed(i64::from(i8::from_le_bytes(self.array()?))),
            2 => Value::Unsigned(u64::from(u16::from_le_bytes(self.array()?))),
            3 => Value::Signed(i64::from(i16::from_le_bytes(self.array()?))),
            4 => Value::Unsigned(u64::from(self.u32()?)),
            5 => Value::Signed(i64::from(i32::from_le_bytes(self.array()?))),
            6 => {
                self.take(4)?;
                Value::Other
            }
            7 => {
                self.take(1)?;
                Value::Other
            }
            8 => self.string()?,
            10 => Value::Unsigned(self.u64()?),
            11 => Value::Signed(i64::from_le_bytes(self.array()?)),
            12 => {
                self.take(8)?;
                Value::Other
            }
            _ => return Err(GgufError::Malformed("an unknown value type")),
        })
    }

    fn value(&mut self, kind: u32) -> Result<Value, GgufError> {
        if kind != 9 {
            return self.scalar(kind);
        }
        let item = self.u32()?;
        let count = self.u64()?;
        if count > MOST_ARRAY_ITEMS {
            return Err(GgufError::Malformed("an array is too long"));
        }
        if item == 9 {
            return Err(GgufError::Malformed("an array of arrays"));
        }
        let mut largest = None;
        for _ in 0..count {
            if let Some(number) = self.scalar(item)?.number() {
                largest = largest.max(Some(number));
            }
        }
        Ok(if matches!(item, 0..=5 | 10 | 11) { Value::Largest(largest) } else { Value::Other })
    }
}

/// Reads the header from the start of a GGUF file.
pub fn parse(bytes: &[u8]) -> Result<Meta, GgufError> {
    let mut reader = Reader { bytes, at: 0 };
    if reader.take(4).map_err(|_| GgufError::NotGguf)? != MAGIC {
        return Err(GgufError::NotGguf);
    }
    let version = reader.u32()?;
    if !(2..=3).contains(&version) {
        return Err(GgufError::Version(version));
    }
    let _tensors = reader.u64()?;
    let entries = reader.u64()?;
    if entries > MOST_ENTRIES {
        return Err(GgufError::Malformed("too many entries"));
    }
    let mut values = HashMap::new();
    for _ in 0..entries {
        let key = match reader.string()? {
            Value::Text(key) => key,
            _ => return Err(GgufError::Malformed("a key isn't a short string")),
        };
        let kind = reader.u32()?;
        values.insert(key, reader.value(kind)?);
    }
    let architecture = values.get("general.architecture").and_then(Value::text);
    let of_model = |field: &str| {
        let arch = architecture.as_deref()?;
        values.get(&format!("{arch}.{field}")).and_then(Value::number)
    };
    Ok(Meta {
        name: values.get("general.name").and_then(Value::text),
        context_length: of_model("context_length"),
        block_count: of_model("block_count"),
        head_count: of_model("attention.head_count"),
        head_count_kv: of_model("attention.head_count_kv"),
        embedding_length: of_model("embedding_length"),
        key_length: of_model("attention.key_length"),
        value_length: of_model("attention.value_length"),
        chat_template: values.contains_key("tokenizer.chat_template"),
        architecture,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a GGUF header, for tests here and in the fake server.
    #[derive(Default)]
    pub(crate) struct Header(Vec<(String, Vec<u8>)>);

    impl Header {
        pub(crate) fn text(mut self, key: &str, value: &str) -> Self {
            let mut bytes = 8u32.to_le_bytes().to_vec();
            bytes.extend((value.len() as u64).to_le_bytes());
            bytes.extend(value.as_bytes());
            self.0.push((key.to_owned(), bytes));
            self
        }

        pub(crate) fn number(mut self, key: &str, value: u32) -> Self {
            let mut bytes = 4u32.to_le_bytes().to_vec();
            bytes.extend(value.to_le_bytes());
            self.0.push((key.to_owned(), bytes));
            self
        }

        fn numbers(mut self, key: &str, values: &[i32]) -> Self {
            let mut bytes = 9u32.to_le_bytes().to_vec();
            bytes.extend(5u32.to_le_bytes());
            bytes.extend((values.len() as u64).to_le_bytes());
            for value in values {
                bytes.extend(value.to_le_bytes());
            }
            self.0.push((key.to_owned(), bytes));
            self
        }

        fn texts(mut self, key: &str, count: usize) -> Self {
            let mut bytes = 9u32.to_le_bytes().to_vec();
            bytes.extend(8u32.to_le_bytes());
            bytes.extend((count as u64).to_le_bytes());
            for index in 0..count {
                let token = format!("t{index}");
                bytes.extend((token.len() as u64).to_le_bytes());
                bytes.extend(token.as_bytes());
            }
            self.0.push((key.to_owned(), bytes));
            self
        }

        pub(crate) fn bytes(&self) -> Vec<u8> {
            let mut bytes = MAGIC.to_vec();
            bytes.extend(3u32.to_le_bytes());
            bytes.extend(0u64.to_le_bytes());
            bytes.extend((self.0.len() as u64).to_le_bytes());
            for (key, value) in &self.0 {
                bytes.extend((key.len() as u64).to_le_bytes());
                bytes.extend(key.as_bytes());
                bytes.extend(value);
            }
            bytes
        }
    }

    fn qwen() -> Header {
        Header::default()
            .text("general.architecture", "qwen3moe")
            .text("general.name", "Qwen3 Coder")
            .number("qwen3moe.context_length", 262_144)
            .number("qwen3moe.block_count", 48)
            .number("qwen3moe.attention.head_count", 32)
            .numbers("qwen3moe.attention.head_count_kv", &[4, 4, 2])
            .number("qwen3moe.embedding_length", 2048)
            .number("qwen3moe.attention.key_length", 128)
            .texts("tokenizer.ggml.tokens", 1000)
            .text("tokenizer.chat_template", &"{% x %}".repeat(100))
    }

    #[test]
    fn the_header_gives_the_models_shape() {
        let meta = parse(&qwen().bytes()).expect("parses");
        assert_eq!(meta.architecture.as_deref(), Some("qwen3moe"));
        assert_eq!(meta.name.as_deref(), Some("Qwen3 Coder"));
        assert_eq!(meta.context_length, Some(262_144));
        assert_eq!(meta.head_count_kv, Some(4));
        assert!(meta.chat_template);
        // 48 layers × 4 heads × (128 + 64 from 2048 / 32) × 2 bytes.
        assert_eq!(meta.kv_bytes_per_token(), Some(48 * 4 * (128 + 64) * 2));
    }

    #[test]
    fn a_short_read_asks_for_more() {
        let bytes = qwen().bytes();
        assert_eq!(parse(&bytes[..bytes.len() - 10]), Err(GgufError::Truncated));
    }

    #[test]
    fn what_isnt_gguf_is_refused() {
        assert_eq!(parse(b"GGML...."), Err(GgufError::NotGguf));
        assert_eq!(parse(b""), Err(GgufError::NotGguf));
        let mut old = qwen().bytes();
        old[4..8].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(parse(&old), Err(GgufError::Version(1)));
    }

    #[test]
    fn lengths_past_the_limits_are_refused_before_reading() {
        let mut bytes = MAGIC.to_vec();
        bytes.extend(3u32.to_le_bytes());
        bytes.extend(0u64.to_le_bytes());
        bytes.extend(1u64.to_le_bytes());
        bytes.extend(u64::MAX.to_le_bytes());
        assert_eq!(parse(&bytes), Err(GgufError::Malformed("a string is too long")));
        let mut many = MAGIC.to_vec();
        many.extend(3u32.to_le_bytes());
        many.extend(0u64.to_le_bytes());
        many.extend(u64::MAX.to_le_bytes());
        assert_eq!(parse(&many), Err(GgufError::Malformed("too many entries")));
    }

    #[test]
    fn every_prefix_of_a_header_parses_or_asks_for_more() {
        let bytes = qwen().bytes();
        for end in 4..bytes.len() {
            assert_eq!(parse(&bytes[..end]), Err(GgufError::Truncated), "at {end}");
        }
    }
}
