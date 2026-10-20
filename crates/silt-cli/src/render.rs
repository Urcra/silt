use std::fmt::Write;

use silt_daemon::protocol::{
    CommentLocation, CommentView, HistoryPage, InFlight, MrDetails, MrSummary, SearchResult, ShowLine, ThreadSummary};

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
        let fork = thread
            .forked_from
            .as_deref()
            .map(|from| format!(" fork of {}", short(from)))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "{}  {:<6} {:>4} msgs {:>5} edits  {}  ({}){fork}",
            short(&thread.id),
            thread.location,
            thread.messages,
            thread.edits,
            thread.title.as_deref().unwrap_or("untitled"),
            thread.repo,
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
            ShowLine::Segment { label } => writeln!(out, "\n═══ {label} ═══"),
            ShowLine::Event { text } => writeln!(out, "\n⇄ {text}"),
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
        let who = match (&hit.owner, &hit.status) {
            (Some(owner), Some(status)) => format!("{owner}, {status}  "),
            _ => String::new(),
        };
        let _ = writeln!(out, "{}  {who}{:<11} {}", short(&hit.thread), hit.kind, clip(&hit.snippet, 160));
    }
    out
}

pub fn history(page: &HistoryPage) -> String {
    let mut out = format!("thread {}: {} entries\n", page.thread, page.total);
    for entry in &page.entries {
        let origin = match entry.origin.as_str() {
            "own" => String::new(),
            other => format!(" ({other} from {})", short(&entry.thread)),
        };
        let _ = writeln!(out, "[{}] {}{origin}: {}", entry.index, entry.kind, clip(entry.text.trim(), 200));
    }
    out
}

pub fn comment_location(location: &CommentLocation) -> String {
    match location {
        CommentLocation::Lines {
            path,
            start,
            end,
            outdated,
        } => {
            let lines = if start == end { format!("{start}") } else { format!("{start}-{end}") };
            let note = if *outdated { " (outdated: the lines were deleted)" } else { "" };
            format!("{path}:{lines}{note}")
        }
        CommentLocation::Entry { index, excerpt } => format!("entry {index} ({})", clip(excerpt, 60)),
        CommentLocation::Unknown => "an unknown place".into(),
    }
}

pub fn comments(comments: &[CommentView]) -> String {
    let mut out = String::new();
    if comments.is_empty() {
        out.push_str("no comments\n");
    }
    for comment in comments {
        let resolved = if comment.resolved { " [resolved]" } else { "" };
        let _ = writeln!(
            out,
            "{}  {}  {}{resolved}\n    {}",
            comment.id,
            comment.by,
            comment_location(&comment.location),
            comment.text
        );
    }
    out
}

pub fn in_flight(found: &[InFlight]) -> String {
    let mut out = String::new();
    if found.is_empty() {
        out.push_str("no other thread is changing this\n");
    }
    for item in found {
        let owner = if item.owner.is_empty() { "you" } else { item.owner.as_str() };
        let _ = writeln!(out, "{}  {owner:<10} {}", short(&item.thread), item.files.join(", "));
    }
    out
}

pub fn merge_requests(requests: &[MrSummary]) -> String {
    let mut out = String::new();
    if requests.is_empty() {
        out.push_str("no merge requests\n");
    }
    for request in requests {
        let _ = writeln!(
            out,
            "{}  {} → {}  {}  ({})",
            request.id,
            short(&request.fork),
            short(&request.target),
            request.title,
            request.status
        );
    }
    out
}

pub fn merge_request(details: &MrDetails) -> String {
    let request = &details.mr;
    let mut out = format!(
        "merge request {} ({})\n{}\nfrom fork {} into {}\n\n{}\n",
        request.id,
        request.status,
        request.title,
        request.fork,
        request.target,
        request.summary.trim()
    );
    if !details.conflicts.is_empty() {
        out.push_str("\nconflict hints (the merge keeps both sides; check these):\n");
        for conflict in &details.conflicts {
            let _ = writeln!(out, "  ! {conflict}");
        }
    }
    out.push_str("\n── changes ──\n");
    out.push_str(&details.diff);
    out.push_str("\n── the fork's conversation ──\n");
    out.push_str(&show(&details.conversation));
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
