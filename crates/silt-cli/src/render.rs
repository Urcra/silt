use std::fmt::Write;

use silt_daemon::protocol::{SearchResult, ShowLine, ThreadSummary};

/// The last 12 hex digits: UUIDv7 ids share their leading time digits.
fn short(id: &str) -> &str {
    &id[id.len().saturating_sub(12)..]
}

fn clip(text: &str, max: usize) -> String {
    let single_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.chars().count() <= max {
        return single_line;
    }
    let clipped: String = single_line.chars().take(max).collect();
    format!("{clipped}…")
}

pub fn threads(threads: &[ThreadSummary]) -> String {
    let mut out = String::new();
    if threads.is_empty() {
        out.push_str("no threads yet; start one with `silt claude`\n");
        return out;
    }
    for thread in threads {
        let _ = writeln!(
            out,
            "{}  {:>4} msgs {:>5} edits  {}  ({})",
            short(&thread.id),
            thread.messages,
            thread.edits,
            thread.title.as_deref().unwrap_or("untitled"),
            thread.repo.display(),
        );
    }
    out
}

pub fn show(lines: &[ShowLine]) -> String {
    let mut out = String::new();
    for line in lines {
        let _ = match line {
            ShowLine::Entry { kind, text } => match kind.as_str() {
                "user" => writeln!(out, "\n> {}", text.trim()),
                "assistant" => writeln!(out, "{}", text.trim()),
                "thinking" => writeln!(out, "  (thinking) {}", clip(text, 160)),
                "tool_result" => writeln!(out, "    ↳ {}", clip(text, 160)),
                other => writeln!(out, "[{other}] {}", clip(text, 160)),
            },
            ShowLine::ToolCall { name, input, .. } => writeln!(out, "  ⚙ {name} {}", clip(input, 140)),
            ShowLine::Edits { tool_use_id, files } => {
                let who = if tool_use_id.is_some() { "    ✎" } else { "  ✎ outside tool calls:" };
                let files: Vec<String> = files
                    .iter()
                    .map(|file| format!("{} +{} -{}", file.path, file.inserted, file.deleted))
                    .collect();
                writeln!(out, "{who} {}", files.join(", "))
            }
            ShowLine::Compaction { summary } => match summary {
                Some(summary) => writeln!(out, "── context compacted: {} ──", clip(summary, 120)),
                None => writeln!(out, "── context compacted ──"),
            },
            ShowLine::Turn { mark } if mark == "PromptSubmitted" => writeln!(out, "\n────────"),
            ShowLine::Turn { .. } => Ok(()),
        };
    }
    out
}

pub fn search(hits: &[SearchResult]) -> String {
    let mut out = String::new();
    if hits.is_empty() {
        out.push_str("no matches\n");
    }
    for hit in hits {
        let _ = writeln!(out, "{}  {:<11} {}", short(&hit.thread), hit.kind, clip(&hit.snippet, 160));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipping_keeps_short_text_and_flattens_lines() {
        assert_eq!(clip("a\n  b", 10), "a b");
        assert_eq!(clip("abcdef", 3), "abc…");
    }
}
