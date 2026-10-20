//! Thread work is based on a git blob. When upstream moves on, its diff is
//! applied as events whose parents are the import point, so the CRDT merge
//! rebases the thread's work onto the new commit. For edits that don't
//! overlap, the result must equal a three-way merge.

use proptest::prelude::*;
use silt_crdt::{CharId, TextReplica};
use silt_types::{ActorId, DeltaId, Frontier, Lamport};

const IMPORTER: ActorId = ActorId(100);
const UPSTREAM: ActorId = ActorId(50);
const THREAD: ActorId = ActorId(60);

fn import() -> DeltaId {
    DeltaId::new(IMPORTER, Lamport(0))
}

fn replace_line(replica: &mut TextReplica, line: usize, replacement: &str) {
    let text = replica.text();
    let start: usize = text.split_inclusive('\n').take(line).map(|l| l.chars().count()).sum();
    let old_len = text.split_inclusive('\n').nth(line).expect("line exists").trim_end_matches('\n').chars().count();
    if old_len > 0 {
        replica.delete(start, old_len).expect("delete line");
    }
    replica.insert(start, replacement).expect("insert line");
}

fn merge(into: &mut TextReplica, from: &TextReplica) {
    for event in from.events() {
        into.receive(event).expect("event applies");
    }
}

#[test]
fn base_content_is_visible_from_the_import_on() {
    let replica = TextReplica::from_base(THREAD, import(), "fn main() {}\n").expect("replica");
    assert_eq!(replica.text(), "fn main() {}\n");
    assert_eq!(replica.doc().text_at(replica.graph(), &Frontier::single(import())), "fn main() {}\n");
    assert_eq!(replica.doc().text_at(replica.graph(), &Frontier::root()), "");
    assert!(matches!(
        replica.doc().attribution()[0].1,
        CharId::Base { source, offset: 0 } if source == import()
    ));
}

#[test]
fn upstream_advance_rebases_thread_work() {
    let base = "fn a() {}\nfn b() {}\n";
    let mut upstream = TextReplica::from_base(UPSTREAM, import(), base).expect("replica");
    let mut thread = TextReplica::from_base(THREAD, import(), base).expect("replica");
    replace_line(&mut upstream, 0, "fn alpha() {}");
    replace_line(&mut thread, 1, "fn beta() {}");
    merge(&mut thread, &upstream);
    merge(&mut upstream, &thread);
    assert_eq!(thread.text(), "fn alpha() {}\nfn beta() {}\n");
    assert_eq!(upstream.text(), thread.text());
}

proptest! {
    #[test]
    fn non_overlapping_edits_merge_like_a_three_way_merge(
        lines in prop::collection::vec("[a-z ]{0,8}", 2..12),
        upstream_edits in prop::collection::vec((any::<u8>(), "[A-Z]{1,6}"), 0..6),
        thread_edits in prop::collection::vec((any::<u8>(), "[0-9]{1,6}"), 0..6),
    ) {
        let base: String = lines.iter().map(|line| format!("{line}\n")).collect();
        let mut upstream = TextReplica::from_base(UPSTREAM, import(), &base).expect("replica");
        let mut thread = TextReplica::from_base(THREAD, import(), &base).expect("replica");
        let mut expected = lines.clone();

        for (pick, replacement) in &upstream_edits {
            let line = (usize::from(*pick) % lines.len().div_ceil(2)) * 2;
            replace_line(&mut upstream, line, replacement);
            expected[line] = replacement.clone();
        }
        for (pick, replacement) in &thread_edits {
            let line = (usize::from(*pick) % (lines.len() / 2)) * 2 + 1;
            replace_line(&mut thread, line, replacement);
            expected[line] = replacement.clone();
        }

        merge(&mut thread, &upstream);
        let expected: String = expected.iter().map(|line| format!("{line}\n")).collect();
        prop_assert_eq!(thread.text(), expected);
    }
}
