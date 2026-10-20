//! Expected results come from diamond-types (M0-10 probe), with actor ids
//! ordered like its agent names.

use silt_crdt::{CausalError, CharId, ReplicaError, TextError, TextOp, TextReplica};
use silt_types::{ActorId, Frontier};

const BASE: ActorId = ActorId(9);
const A: ActorId = ActorId(1);
const B: ActorId = ActorId(2);
const C: ActorId = ActorId(3);

fn sync(from: &TextReplica, to: &mut TextReplica) {
    for event in from.events() {
        to.receive(event).expect("event applies");
    }
}

/// Replicas for `actors` that all start from `base` typed by BASE.
fn from_base(base: &str, actors: &[ActorId]) -> Vec<TextReplica> {
    let mut origin = TextReplica::new(BASE);
    if !base.is_empty() {
        origin.insert(0, base).expect("base insert");
    }
    actors
        .iter()
        .map(|actor| {
            let mut replica = TextReplica::new(*actor);
            sync(&origin, &mut replica);
            replica
        })
        .collect()
}

/// Merges every replica into every other one and checks they agree.
fn converge(replicas: &mut [TextReplica]) -> String {
    let snapshot: Vec<TextReplica> = replicas.to_vec();
    for replica in replicas.iter_mut() {
        for other in &snapshot {
            sync(other, replica);
        }
    }
    let text = replicas[0].text();
    for replica in replicas.iter() {
        assert_eq!(replica.text(), text, "replica {} diverged", replica.actor());
    }
    text
}

#[test]
fn local_typing_and_deleting() {
    let mut replica = TextReplica::new(A);
    replica.insert(0, "hello").expect("insert");
    replica.insert(5, " world").expect("insert");
    replica.delete(0, 1).expect("delete");
    replica.insert(0, "J").expect("insert");
    assert_eq!(replica.text(), "Jello world");
    assert_eq!(replica.doc().len(), 11);
}

#[test]
fn published_reactivity_example() {
    let mut replicas = from_base("TIVITY", &[A, B]);
    replicas[0].insert(0, "REA").expect("a");
    replicas[1].insert(0, "C").expect("b");
    assert_eq!(converge(&mut replicas), "REACTIVITY");
}

#[test]
fn swapping_actors_swaps_the_tie_break() {
    let mut replicas = from_base("TIVITY", &[A, B]);
    replicas[1].insert(0, "REA").expect("b");
    replicas[0].insert(0, "C").expect("a");
    assert_eq!(converge(&mut replicas), "CREATIVITY");
}

#[test]
fn delete_and_concurrent_insert_after_it() {
    let mut replicas = from_base("hello world", &[A, B]);
    replicas[0].delete(6, 5).expect("delete world");
    replicas[1].insert(6, "big ").expect("insert");
    assert_eq!(converge(&mut replicas), "hello big ");
}

#[test]
fn insert_inside_concurrently_deleted_range_survives() {
    let mut replicas = from_base("hello world", &[A, B]);
    replicas[0].delete(6, 5).expect("delete world");
    replicas[1].insert(8, "X").expect("insert");
    assert_eq!(converge(&mut replicas), "hello X");
}

#[test]
fn overlapping_concurrent_deletes() {
    let mut replicas = from_base("abcdef", &[A, B]);
    replicas[0].delete(1, 3).expect("bcd");
    replicas[1].delete(2, 3).expect("cde");
    assert_eq!(converge(&mut replicas), "af");
}

#[test]
fn three_way_concurrent_inserts_then_merge() {
    let mut replicas = from_base("XY", &[A, B, C]);
    replicas[2].insert(1, "3").expect("c");
    replicas[0].insert(1, "1").expect("a");
    replicas[1].insert(1, "2").expect("b");
    converge(&mut replicas);
    let end = replicas[0].doc().len();
    replicas[0].insert(end, "!").expect("merge");
    assert_eq!(converge(&mut replicas), "X123Y!");
}

#[test]
fn append_delete_all_prepend() {
    let mut replicas = from_base("xyz", &[A, B, C]);
    replicas[0].insert(3, "A").expect("a");
    replicas[1].delete(0, 3).expect("b");
    replicas[2].insert(0, "C").expect("c");
    converge(&mut replicas);
    let end = replicas[0].doc().len();
    replicas[0].insert(end, "AA").expect("a again");
    assert_eq!(converge(&mut replicas), "CAAA");
}

#[test]
fn concurrent_inserts_into_empty_document() {
    let mut replicas = from_base("", &[A, B]);
    replicas[1].insert(0, "bbb").expect("b");
    replicas[0].insert(0, "aaa").expect("a");
    assert_eq!(converge(&mut replicas), "aaabbb");
}

#[test]
fn backward_typing_does_not_interleave() {
    let mut replicas = from_base("_", &[A, B]);
    replicas[0].insert(0, "1").expect("a");
    replicas[0].insert(0, "2").expect("a");
    replicas[1].insert(0, "x").expect("b");
    replicas[1].insert(0, "y").expect("b");
    assert_eq!(converge(&mut replicas), "21yx_");
}

#[test]
fn forward_typing_does_not_interleave() {
    let mut replicas = from_base("_", &[A, B]);
    replicas[0].insert(0, "1").expect("a");
    replicas[0].insert(1, "2").expect("a");
    replicas[1].insert(0, "x").expect("b");
    replicas[1].insert(1, "y").expect("b");
    assert_eq!(converge(&mut replicas), "12xy_");
}

#[test]
fn delivery_order_does_not_matter() {
    let mut replicas = from_base("TIVITY", &[A, B]);
    replicas[0].insert(0, "REA").expect("a");
    replicas[1].insert(0, "C").expect("b");
    let mut forward = TextReplica::new(C);
    let mut backward = TextReplica::new(C);
    sync(&replicas[0], &mut forward);
    sync(&replicas[1], &mut forward);
    sync(&replicas[1], &mut backward);
    sync(&replicas[0], &mut backward);
    assert_eq!(forward.text(), "REACTIVITY");
    assert_eq!(backward.text(), "REACTIVITY");
}

#[test]
fn text_at_earlier_versions() {
    let mut replicas = from_base("TIVITY", &[A, B]);
    let base = replicas[0].frontier().clone();
    replicas[0].insert(0, "REA").expect("a");
    let a_only = replicas[0].frontier().clone();
    replicas[1].insert(0, "C").expect("b");
    converge(&mut replicas);
    let replica = &replicas[0];
    assert_eq!(replica.doc().text_at(replica.graph(), &base), "TIVITY");
    assert_eq!(replica.doc().text_at(replica.graph(), &a_only), "REATIVITY");
    assert_eq!(replica.doc().text_at(replica.graph(), &Frontier::root()), "");
}

#[test]
fn attribution_names_the_inserting_actor() {
    let mut replicas = from_base("ab", &[A]);
    replicas[0].insert(1, "X").expect("a");
    let actors: Vec<(char, ActorId)> = replicas[0]
        .doc()
        .attribution()
        .into_iter()
        .map(|(ch, id)| match id {
            CharId::Delta(id) => (ch, id.actor),
            CharId::Base { .. } => panic!("no base content in this test"),
        })
        .collect();
    assert_eq!(actors, vec![('a', BASE), ('X', A), ('b', BASE)]);
}

#[test]
fn redelivery_is_idempotent() {
    let mut replicas = from_base("abc", &[A, B]);
    replicas[0].insert(1, "X").expect("a");
    let snapshot = replicas[0].clone();
    sync(&snapshot, &mut replicas[1]);
    sync(&snapshot, &mut replicas[1]);
    assert_eq!(replicas[1].text(), "aXbc");
}

#[test]
fn rejects_events_with_unknown_parents_or_bad_positions() {
    let mut a = TextReplica::new(A);
    a.insert(0, "abc").expect("a");
    let second = a.insert(3, "d").expect("a");
    let mut b = TextReplica::new(B);
    assert!(matches!(
        b.receive(&second),
        Err(ReplicaError::Causal(CausalError::UnknownParent(_)))
    ));
    assert_eq!(b.text(), "");
    assert!(matches!(
        a.insert(10, "x"),
        Err(ReplicaError::Text(TextError::OutOfBounds { .. }))
    ));
    assert!(matches!(
        a.delete(2, 5),
        Err(ReplicaError::Text(TextError::OutOfBounds {
            op: TextOp::Delete { pos: 2, len: 5 },
            len: 4
        }))
    ));
    assert_eq!(a.text(), "abcd");
}

#[test]
fn unicode_positions_count_chars() {
    let mut replica = TextReplica::new(A);
    replica.insert(0, "héllo🎉").expect("insert");
    replica.delete(5, 1).expect("delete emoji");
    replica.insert(1, "→").expect("insert arrow");
    assert_eq!(replica.text(), "h→éllo");
}
