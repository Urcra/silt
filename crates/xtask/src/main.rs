//! Tasks for working on Silt rather than using it, kept out of the `silt`
//! binary:
//! - `cargo xtask status [--root <dir>]` shows milestone and task progress
//!   from ROADMAP.md and TASKS.md;
//! - `cargo xtask wasm-bindgen <wasm> <out-dir>` writes the browser
//!   replica's JavaScript bindings, as `wasm-bindgen --target web` would,
//!   with the version silt-wasm links, so nobody installs a matching CLI
//!   (ADR 0097).

use std::env;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Milestone {
    id: String,
    title: String,
    status: String,
    done: usize,
    total: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct TaskCounts {
    in_progress: usize,
    todo: usize,
    blocked: usize,
    done: usize,
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["status"] => status(None),
        ["status", "--root", root] => status(Some(PathBuf::from(root))),
        ["wasm-bindgen", wasm, out_dir] => wasm_bindgen(Path::new(wasm), Path::new(out_dir)),
        _ => Err("usage: cargo xtask status [--root <dir>]\n       cargo xtask wasm-bindgen <wasm> <out-dir>".to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}

fn status(root: Option<PathBuf>) -> Result<(), String> {
    let root = match root {
        Some(root) => root,
        None => find_repo_root()?,
    };
    let roadmap = read(&root.join("ROADMAP.md"))?;
    let tasks = read(&root.join("TASKS.md"))?;
    print!("{}", render(&parse_roadmap(&roadmap), &parse_tasks(&tasks)));
    Ok(())
}

/// The settings `wasm-bindgen --target web` runs with where they differ
/// from the library's own defaults.
fn wasm_bindgen(wasm: &Path, out_dir: &Path) -> Result<(), String> {
    let mut bindgen = wasm_bindgen_cli_support::Bindgen::new();
    bindgen.input_path(wasm).typescript(true).omit_default_module_path(false);
    bindgen
        .web(true)
        .and_then(|bindgen| bindgen.generate(out_dir))
        .map_err(|error| format!("wasm-bindgen on {}: {error:#}", wasm.display()))
}

fn find_repo_root() -> Result<PathBuf, String> {
    let cwd = env::current_dir().map_err(|error| format!("cannot read the current directory: {error}"))?;
    cwd.ancestors()
        .find(|dir| dir.join("ROADMAP.md").is_file())
        .map(Path::to_path_buf)
        .ok_or_else(|| "no ROADMAP.md found in any parent directory".to_owned())
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("cannot read {}: {error}", path.display()))
}

fn parse_roadmap(text: &str) -> Vec<Milestone> {
    let mut milestones: Vec<Milestone> = Vec::new();
    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            if let Some(milestone) = parse_milestone_heading(heading) {
                milestones.push(milestone);
            }
            continue;
        }
        let Some(current) = milestones.last_mut() else {
            continue;
        };
        match checkbox(line) {
            Some(Checkbox::Checked) => {
                current.done += 1;
                current.total += 1;
            }
            Some(Checkbox::Unchecked) => current.total += 1,
            None => {}
        }
    }
    milestones
}

fn parse_tasks(text: &str) -> TaskCounts {
    let mut counts = TaskCounts::default();
    let mut section: Option<&str> = None;
    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            section = Some(heading.trim());
            continue;
        }
        if checkbox(line).is_none() {
            continue;
        }
        match section {
            Some("In progress") => counts.in_progress += 1,
            Some("Todo") => counts.todo += 1,
            Some("Blocked") => counts.blocked += 1,
            Some("Done") => counts.done += 1,
            _ => {}
        }
    }
    counts
}

fn render(milestones: &[Milestone], tasks: &TaskCounts) -> String {
    let mut out = String::new();
    let active = milestones.iter().find(|m| m.status == "active");
    match active {
        Some(m) => writeln!(out, "Active: {} {} ({}/{} criteria)", m.id, m.title, m.done, m.total),
        None => writeln!(out, "Active: none"),
    }
    .expect("writing to a String cannot fail");
    writeln!(
        out,
        "Tasks:  {} in progress, {} todo, {} blocked, {} done",
        tasks.in_progress, tasks.todo, tasks.blocked, tasks.done
    )
    .expect("writing to a String cannot fail");
    out.push('\n');
    for m in milestones {
        writeln!(out, "{:<4} {:<8} {:>3}/{:<3} {}", m.id, m.status, m.done, m.total, m.title)
            .expect("writing to a String cannot fail");
    }
    out
}

enum Checkbox {
    Checked,
    Unchecked,
}

fn checkbox(line: &str) -> Option<Checkbox> {
    let item = line.trim_start().strip_prefix("- [")?;
    match item.chars().next()? {
        'x' | 'X' => Some(Checkbox::Checked),
        ' ' => Some(Checkbox::Unchecked),
        _ => None,
    }
}

fn parse_milestone_heading(heading: &str) -> Option<Milestone> {
    let (id, rest) = heading.split_once(": ")?;
    if !id.starts_with('M') || !id[1..].chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (title, status) = match rest.rsplit_once(" (`") {
        Some((title, status)) => (title, status.split('`').next().unwrap_or_default()),
        None => (rest, ""),
    };
    Some(Milestone { id: id.to_owned(), title: title.trim().to_owned(), status: status.to_owned(), done: 0, total: 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROADMAP: &str = "\
# Roadmap

Intro with a stray box:
- [x] not part of any milestone

## M0: Spike and skeleton (`active`)
- [x] repo
- [ ] probes

## M1: CRDT (`planned`, optional)
- [ ] convergence
  - [x] nested still counts

## Notes
";

    const TASKS: &str = "\
# Tasks

## In progress
- [ ] M0-14: dev status
  → next: tests

## Todo
- [ ] M0-15: a
- [ ] M0-16: b

## Blocked

## Done
- [x] M0-01: init
";

    #[test]
    fn roadmap_counts_criteria_per_milestone() {
        let milestones = parse_roadmap(ROADMAP);
        assert_eq!(
            milestones,
            vec![
                Milestone { id: "M0".into(), title: "Spike and skeleton".into(), status: "active".into(), done: 1, total: 2 },
                Milestone { id: "M1".into(), title: "CRDT".into(), status: "planned".into(), done: 1, total: 2 },
            ]
        );
    }

    #[test]
    fn tasks_are_counted_by_section() {
        assert_eq!(parse_tasks(TASKS), TaskCounts { in_progress: 1, todo: 2, blocked: 0, done: 1 });
    }

    #[test]
    fn render_shows_active_milestone() {
        let out = render(&parse_roadmap(ROADMAP), &parse_tasks(TASKS));
        assert!(out.starts_with("Active: M0 Spike and skeleton (1/2 criteria)\n"));
        assert!(out.contains("1 in progress, 2 todo, 0 blocked, 1 done"));
    }
}
