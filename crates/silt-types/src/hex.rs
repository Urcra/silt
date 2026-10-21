/// An id or hash that didn't parse: `what` it should have been, and the
/// text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {what} {text:?}: expected {digits} hex digits")]
pub struct ParseIdError {
    what: &'static str,
    digits: usize,
    text: String,
}

impl ParseIdError {
    pub(crate) fn new(what: &'static str, digits: usize, text: &str) -> Self {
        Self { what, digits, text: text.to_owned() }
    }
}

/// Exactly `2 * N` hex digits, and nothing else: no sign, no prefix.
pub(crate) fn parse<const N: usize>(what: &'static str, text: &str) -> Result<[u8; N], ParseIdError> {
    let invalid = || ParseIdError::new(what, 2 * N, text);
    if text.len() != 2 * N || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid());
    }
    let mut bytes = [0; N];
    for (byte, pair) in bytes.iter_mut().zip(text.as_bytes().chunks(2)) {
        let pair = std::str::from_utf8(pair).map_err(|_| invalid())?;
        *byte = u8::from_str_radix(pair, 16).map_err(|_| invalid())?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_hex_digits_parse() {
        assert_eq!(parse::<2>("thing", "0aF9"), Ok([0x0a, 0xf9]));
        for text in ["+0aF9", "0aF", "0aF90", "0x0a", " 0aF", "0a-9", "éa"] {
            assert!(parse::<2>("thing", text).is_err(), "{text:?}");
        }
        assert_eq!(parse::<2>("thing", "+abc").unwrap_err().to_string(), r#"invalid thing "+abc": expected 4 hex digits"#);
    }
}
