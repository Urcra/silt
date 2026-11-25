//! A request's `Range` header, as a blob is read by it (ADR 0121).

/// What part of a resource a request asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ranged {
    /// No range, or one this doesn't answer by parts, such as several at once.
    Whole,
    /// Bytes `start` to `end`, both included.
    Part { start: u64, end: u64 },
    /// A range wholly past the end.
    Unsatisfiable,
}

/// The part of a resource of `size` bytes that `header` asks for: one
/// range of bytes, from a start, from a start to an end, or the last so many.
pub fn range(header: Option<&str>, size: u64) -> Ranged {
    let Some(spec) = header.map(str::trim).and_then(|header| header.strip_prefix("bytes=")) else {
        return Ranged::Whole;
    };
    if spec.contains(',') {
        return Ranged::Whole;
    }
    let Some((start, end)) = spec.split_once('-') else {
        return Ranged::Whole;
    };
    let (start, end) = (start.trim(), end.trim());
    let parsed = |text: &str| text.parse::<u64>().ok();
    match (start.is_empty(), end.is_empty()) {
        (true, true) => Ranged::Whole,
        // The last `end` bytes.
        (true, false) => match parsed(end) {
            Some(0) => Ranged::Unsatisfiable,
            Some(_) if size == 0 => Ranged::Unsatisfiable,
            Some(last) => Ranged::Part { start: size.saturating_sub(last), end: size - 1 },
            None => Ranged::Whole,
        },
        (false, open) => {
            let Some(start) = parsed(start) else {
                return Ranged::Whole;
            };
            let end = if open { Some(u64::MAX) } else { parsed(end) };
            match end {
                None => Ranged::Whole,
                Some(end) if end < start => Ranged::Whole,
                Some(_) if start >= size => Ranged::Unsatisfiable,
                Some(end) => Ranged::Part { start, end: end.min(size - 1) },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_read_as_browsers_send_them() {
        assert_eq!(range(None, 100), Ranged::Whole);
        assert_eq!(range(Some("bytes=0-"), 100), Ranged::Part { start: 0, end: 99 });
        assert_eq!(range(Some("bytes=10-19"), 100), Ranged::Part { start: 10, end: 19 });
        assert_eq!(range(Some("bytes=90-500"), 100), Ranged::Part { start: 90, end: 99 });
        assert_eq!(range(Some("bytes=-30"), 100), Ranged::Part { start: 70, end: 99 });
        assert_eq!(range(Some("bytes=-300"), 100), Ranged::Part { start: 0, end: 99 });
        assert_eq!(range(Some("bytes=100-"), 100), Ranged::Unsatisfiable);
        assert_eq!(range(Some("bytes=-0"), 100), Ranged::Unsatisfiable);
        assert_eq!(range(Some("bytes=0-1,5-6"), 100), Ranged::Whole);
        assert_eq!(range(Some("bytes=9-3"), 100), Ranged::Whole);
        assert_eq!(range(Some("items=0-3"), 100), Ranged::Whole);
        assert_eq!(range(Some("bytes=x-3"), 100), Ranged::Whole);
        assert_eq!(range(Some("bytes=0-"), 0), Ranged::Unsatisfiable);
    }
}
