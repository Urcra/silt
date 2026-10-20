use std::env;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

use exn::{Exn, ResultExt};

use crate::CliError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Milestone {
    pub id: String,
    pub title: String,
    pub status: String,
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TaskCounts {
    pub in_progress: usize,
    pub todo: usize,
    pub blocked: usize,
    pub done: usize,
}

pub fn parse_roadmap(text: &str) -> Vec<Milestone> {
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

pub fn parse_tasks(text: &str) -> TaskCounts {
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

pub fn render(milestones: &[Milestone], tasks: &TaskCounts) -> String {
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
    Some(Milestone {
        id: id.to_owned(),
        title: title.trim().to_owned(),
        status: status.to_owned(),
        done: 0,
        total: 0,
    })
}

pub fn print(root: Option<PathBuf>) -> Result<(), Exn<CliError>> {
    let root = match root {
        Some(root) => root,
        None => find_repo_root()?,
    };
    let roadmap = read(&root.join("ROADMAP.md"))?;
    let tasks = read(&root.join("TASKS.md"))?;
    print!("{}", render(&parse_roadmap(&roadmap), &parse_tasks(&tasks)));
    Ok(())
}

fn find_repo_root() -> Result<PathBuf, Exn<CliError>> {
    let cwd = env::current_dir().or_raise(|| CliError("cannot read current directory".into()))?;
    cwd.ancestors()
        .find(|dir| dir.join("ROADMAP.md").is_file())
        .map(Path::to_path_buf)
        .ok_or_else(|| Exn::new(CliError("no ROADMAP.md found in any parent directory".into())))
}

fn read(path: &Path) -> Result<String, Exn<CliError>> {
    fs::read_to_string(path).or_raise(|| CliError(format!("cannot read {}", path.display())))
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
                Milestone {
                    id: "M0".into(),
                    title: "Spike and skeleton".into(),
                    status: "active".into(),
                    done: 1,
                    total: 2,
                },
                Milestone {
                    id: "M1".into(),
                    title: "CRDT".into(),
                    status: "planned".into(),
                    done: 1,
                    total: 2,
                },
            ]
        );
    }

    #[test]
    fn tasks_are_counted_by_section() {
        assert_eq!(
            parse_tasks(TASKS),
            TaskCounts {
                in_progress: 1,
                todo: 2,
                blocked: 0,
                done: 1,
            }
        );
    }

    #[test]
    fn render_shows_active_milestone() {
        let out = render(&parse_roadmap(ROADMAP), &parse_tasks(TASKS));
        assert!(out.starts_with("Active: M0 Spike and skeleton (1/2 criteria)\n"));
        assert!(out.contains("1 in progress, 2 todo, 0 blocked, 1 done"));
    }
}
