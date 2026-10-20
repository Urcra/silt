/// The complete (newline-terminated) lines at the start of `buffer`, and how
/// many bytes they span. A trailing partial line is left for the next read.
pub fn complete_lines(buffer: &[u8]) -> (Vec<String>, usize) {
    let (lines, consumed) = complete_lines_ending(buffer);
    (lines.into_iter().map(|(line, _)| line).collect(), consumed)
}

/// Like [`complete_lines`], with where each line's newline ends, so a
/// reader can stop after any line and know how far it got.
pub fn complete_lines_ending(buffer: &[u8]) -> (Vec<(String, usize)>, usize) {
    let mut lines = Vec::new();
    let mut consumed = 0;
    while let Some(end) = buffer[consumed..].iter().position(|byte| *byte == b'\n') {
        let line = &buffer[consumed..consumed + end];
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        consumed += end + 1;
        if !line.is_empty() {
            lines.push((String::from_utf8_lossy(line).into_owned(), consumed));
        }
    }
    (lines, consumed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_partial_lines_for_later() {
        let (lines, consumed) = complete_lines(b"{\"a\":1}\n{\"b\":2}\n{\"c\"");
        assert_eq!(lines, vec!["{\"a\":1}", "{\"b\":2}"]);
        assert_eq!(consumed, 16);
        let (lines, consumed) = complete_lines(b"partial");
        assert!(lines.is_empty());
        assert_eq!(consumed, 0);
    }

    #[test]
    fn tells_where_each_line_ends() {
        let (lines, consumed) = complete_lines_ending(b"{}\n\n[]\r\nx");
        assert_eq!(lines, vec![("{}".to_owned(), 3), ("[]".to_owned(), 8)]);
        assert_eq!(consumed, 8);
    }

    #[test]
    fn skips_blank_lines_and_carriage_returns() {
        let (lines, consumed) = complete_lines(b"\n{}\r\n");
        assert_eq!(lines, vec!["{}"]);
        assert_eq!(consumed, 5);
    }
}
