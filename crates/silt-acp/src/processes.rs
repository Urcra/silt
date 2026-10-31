//! What an agent runs, among the machine's processes: ACP can't ask an agent
//! to stop one command it left in the background, so Silt ends it itself.

use std::collections::{HashMap, HashSet, VecDeque};

use silt_os::Process;

/// Every process `root` started, and what they started, parents first.
pub(crate) fn under(processes: &[Process], root: u32) -> Vec<u32> {
    let children = children(processes);
    let mut found = Vec::new();
    let mut seen = HashSet::from([root]);
    let mut queue = VecDeque::from([root]);
    while let Some(pid) = queue.pop_front() {
        for child in children.get(&pid).into_iter().flatten() {
            if seen.insert(child.pid) {
                found.push(child.pid);
                queue.push_back(child.pid);
            }
        }
    }
    found
}

/// The processes running `command` for the agent `root`, with all they
/// started: the topmost under it whose command line holds the command, the
/// `nth` oldest of them where it runs more than once.
pub(crate) fn running(processes: &[Process], root: u32, command: &str, nth: usize) -> Vec<u32> {
    let wanted = comparable(command);
    if wanted.is_empty() {
        return Vec::new();
    }
    let children = children(processes);
    let mut matches: Vec<&Process> = Vec::new();
    let mut seen = HashSet::from([root]);
    let mut queue = VecDeque::from([root]);
    while let Some(pid) = queue.pop_front() {
        for child in children.get(&pid).into_iter().flatten() {
            if !seen.insert(child.pid) {
                continue;
            }
            if comparable(&child.command.join(" ")).contains(&wanted) {
                matches.push(child);
            } else {
                queue.push_back(child.pid);
            }
        }
    }
    matches.sort_by_key(|process| (process.started, process.pid));
    let Some(chosen) = matches.get(nth).or(matches.first()) else {
        return Vec::new();
    };
    std::iter::once(chosen.pid).chain(under(processes, chosen.pid)).collect()
}

fn children(processes: &[Process]) -> HashMap<u32, Vec<&Process>> {
    let mut children: HashMap<u32, Vec<&Process>> = HashMap::new();
    for process in processes {
        if let Some(parent) = process.parent.filter(|parent| *parent != process.pid) {
            children.entry(parent).or_default().push(process);
        }
    }
    children
}

/// A command as compared: the agent's shell quotes it again, and may join
/// its arguments differently, so quotes, escapes and spaces don't count.
fn comparable(command: &str) -> String {
    command.chars().filter(|char| !char.is_whitespace() && !matches!(char, '\'' | '"' | '\\')).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: u32, parent: u32, command: &str, started: u64) -> Process {
        Process { pid, parent: Some(parent), command: command.split(' ').map(str::to_owned).collect(), started }
    }

    fn table() -> Vec<Process> {
        vec![
            process(10, 1, "node claude-agent-acp", 100),
            process(11, 10, "claude --output-format stream-json", 101),
            process(12, 11, "/bin/bash -c -l eval 'npm run dev -- --port '\\''5173'\\''' < /dev/null", 110),
            process(13, 12, "node vite --port 5173", 111),
            process(14, 13, "esbuild --service", 112),
            process(15, 11, "/bin/bash -c -l eval 'sleep 600' < /dev/null", 120),
            process(16, 11, "/bin/bash -c -l eval 'sleep 600' < /dev/null", 130),
            process(20, 1, "/bin/bash -c npm run dev -- --port 5173", 90),
        ]
    }

    #[test]
    fn a_command_is_found_under_its_agent_however_its_shell_quoted_it() {
        let processes = table();
        assert_eq!(running(&processes, 10, "npm run dev -- --port '5173'", 0), [12, 13, 14]);
        assert_eq!(running(&processes, 10, "npm run dev -- --port 5173", 0), [12, 13, 14], "another agent's isn't");
        assert_eq!(running(&processes, 10, "cargo watch", 0), Vec::<u32>::new());
        assert_eq!(running(&processes, 10, "  ", 0), Vec::<u32>::new());
    }

    #[test]
    fn the_same_command_twice_is_told_apart_by_age() {
        let processes = table();
        assert_eq!(running(&processes, 10, "sleep 600", 0), [15]);
        assert_eq!(running(&processes, 10, "sleep 600", 1), [16]);
        assert_eq!(running(&processes, 10, "sleep 600", 5), [15], "the oldest when the count is off");
    }

    #[test]
    fn everything_an_agent_started_is_under_it() {
        let mut processes = table();
        // A pid reused as its own ancestor doesn't loop.
        processes.push(process(10, 14, "ghost", 1));
        assert_eq!(under(&processes, 10), [11, 12, 15, 16, 13, 14]);
        assert_eq!(under(&processes, 99), Vec::<u32>::new());
    }
}
