use silt_crdt::TextOp;
use similar::{Algorithm, DiffOp, capture_diff_slices};

/// Edits turning `old` into `new`, to be applied in order; positions count
/// chars. Lines are diffed first and each changed hunk is then trimmed to
/// the chars that really differ, so a one-character change stays one
/// character (keeping anchors and attribution on the rest of the line).
/// Hunks are emitted back to front, so every position is also valid in `old`.
pub fn text_edits(old: &str, new: &str) -> Vec<TextOp> {
    if old == new {
        return Vec::new();
    }
    let old_lines: Vec<&str> = old.split_inclusive('\n').collect();
    let new_lines: Vec<&str> = new.split_inclusive('\n').collect();
    let old_starts = char_starts(&old_lines);
    let new_starts = char_starts(&new_lines);
    let old_chars: Vec<char> = old.chars().collect();
    let new_chars: Vec<char> = new.chars().collect();

    let mut hunks = Vec::new();
    for op in capture_diff_slices(Algorithm::Myers, &old_lines, &new_lines) {
        if matches!(op, DiffOp::Equal { .. }) {
            continue;
        }
        let old_range = old_starts[op.old_range().start]..old_starts[op.old_range().end];
        let new_range = new_starts[op.new_range().start]..new_starts[op.new_range().end];
        let removed = &old_chars[old_range.clone()];
        let added = &new_chars[new_range];
        let prefix = common_prefix(removed, added);
        let suffix = common_prefix(
            removed[prefix..].iter().rev(),
            added[prefix..].iter().rev(),
        );
        hunks.push((
            old_range.start + prefix,
            removed.len() - prefix - suffix,
            added[prefix..added.len() - suffix].iter().collect::<String>(),
        ));
    }

    let mut ops = Vec::new();
    for (pos, len, content) in hunks.into_iter().rev() {
        if len > 0 {
            ops.push(TextOp::Delete { pos, len });
        }
        if !content.is_empty() {
            ops.push(TextOp::Insert { pos, content });
        }
    }
    ops
}

/// Char offset where each line starts, plus the total length at the end.
fn char_starts(lines: &[&str]) -> Vec<usize> {
    let mut starts = Vec::with_capacity(lines.len() + 1);
    let mut offset = 0;
    starts.push(0);
    for line in lines {
        offset += line.chars().count();
        starts.push(offset);
    }
    starts
}

fn common_prefix<'a>(
    a: impl IntoIterator<Item = &'a char>,
    b: impl IntoIterator<Item = &'a char>,
) -> usize {
    a.into_iter().zip(b).take_while(|(x, y)| x == y).count()
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn apply(text: &str, ops: &[TextOp]) -> String {
        let mut chars: Vec<char> = text.chars().collect();
        for op in ops {
            match op {
                TextOp::Insert { pos, content } => {
                    chars.splice(*pos..*pos, content.chars());
                }
                TextOp::Delete { pos, len } => {
                    chars.drain(*pos..*pos + *len);
                }
            }
        }
        chars.into_iter().collect()
    }

    #[test]
    fn a_one_char_change_is_one_char() {
        let ops = text_edits("fn main() {}\nlet x = 1;\n", "fn main() {}\nlet x = 2;\n");
        assert_eq!(
            ops,
            vec![
                TextOp::Delete { pos: 21, len: 1 },
                TextOp::Insert {
                    pos: 21,
                    content: "2".into()
                }
            ]
        );
    }

    #[test]
    fn unchanged_text_needs_no_edits() {
        assert!(text_edits("same\n", "same\n").is_empty());
    }

    #[test]
    fn handles_missing_final_newline_and_unicode() {
        let old = "héllo\nwörld";
        let new = "héllo\nwörld!\n→ new";
        assert_eq!(apply(old, &text_edits(old, new)), new);
    }

    proptest! {
        #[test]
        fn applying_the_edits_gives_the_new_text(
            old in "([a-c]{0,3}\n?){0,8}",
            new in "([a-c]{0,3}\n?){0,8}",
        ) {
            prop_assert_eq!(apply(&old, &text_edits(&old, &new)), new);
        }
    }
}
