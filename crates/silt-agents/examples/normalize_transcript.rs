//! Prints how many entries of each kind a Claude Code transcript holds.
//!
//! ```sh
//! cargo run -p silt-agents --example normalize_transcript -- <path.jsonl>
//! ```

use std::collections::BTreeMap;
use std::env;
use std::fs;

use silt_agents::{claude, complete_lines};

fn main() {
    let path = env::args().nth(1).expect("usage: normalize_transcript <path.jsonl>");
    let bytes = fs::read(&path).expect("transcript is readable");
    let (lines, _) = complete_lines(&bytes);
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut searchable = 0;
    for line in &lines {
        for entry in claude::normalize(line) {
            searchable += usize::from(entry.searchable_text().is_some());
            *kinds.entry(entry.kind().to_owned()).or_default() += 1;
        }
    }
    println!("{} lines, {searchable} searchable entries", lines.len());
    for (kind, count) in kinds {
        println!("{count:>6}  {kind}");
    }
}
