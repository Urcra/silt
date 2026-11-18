//! JSON with comments and trailing commas, as VS Code writes its themes,
//! made plain JSON.

/// `text` without `//` and `/* */` comments outside strings, and without a
/// comma that only a `}` or `]` follows.
pub fn strip(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                '\\' => out.extend(chars.next()),
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                for skipped in chars.by_ref() {
                    if skipped == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut last = ' ';
                for skipped in chars.by_ref() {
                    if last == '*' && skipped == '/' {
                        break;
                    }
                    last = skipped;
                }
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    without_trailing_commas(&out)
}

fn without_trailing_commas(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut escaped = false;
    let mut pending: Option<String> = None;
    for c in text.chars() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        if let Some(held) = pending.as_mut() {
            if c.is_whitespace() {
                held.push(c);
                continue;
            }
            let held = pending.take().expect("held");
            if c == '}' || c == ']' {
                out.push_str(&held[1..]);
            } else {
                out.push_str(&held);
            }
        }
        match c {
            ',' => pending = Some(",".to_owned()),
            '"' => {
                in_string = true;
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    if let Some(held) = pending {
        out.push_str(&held);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_and_trailing_commas_go_and_strings_stay() {
        let text = r#"{
            // a comment
            "a": "http://example.com/x", /* block, with a comma, */
            "b": ["x // not a comment", "y /* nor this */",],
            "c": "quote \" and // still string",
        }"#;
        let value: serde_json::Value = serde_json::from_str(&strip(text)).expect("plain JSON");
        assert_eq!(value["a"], "http://example.com/x");
        assert_eq!(value["b"][0], "x // not a comment");
        assert_eq!(value["b"][1], "y /* nor this */");
        assert_eq!(value["c"], "quote \" and // still string");
    }
}
