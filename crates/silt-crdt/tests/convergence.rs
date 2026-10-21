//! Randomised multi-replica schedules: replicas edit concurrently and exchange
//! partial, repeated and reordered prefixes of each other's logs. At the end
//! every replica must agree, agree with diamond-types fed the same events, and
//! still reproduce every earlier text it showed via `text_at`.
//!
//! Run the acceptance-sized suite with `PROPTEST_CASES=10000`.

use std::collections::HashMap;

use diamond_types::list::ListOpLog;
use diamond_types::list::operation::TextOperation;
use proptest::prelude::*;
use silt_crdt::{TextEvent, TextOp, TextReplica};
use silt_types::{ActorId, DeltaId, Frontier};

#[derive(Debug, Clone)]
enum Action {
    Insert { replica: usize, pos: u16, text: String },
    Delete { replica: usize, pos: u16, len: u8 },
    Deliver { from: usize, to: usize, fraction: u8 },
}

fn action(replicas: usize) -> impl Strategy<Value = Action> {
    prop_oneof![
        3 => (0..replicas, any::<u16>(), "[a-e]{1,4}")
            .prop_map(|(replica, pos, text)| Action::Insert { replica, pos, text }),
        2 => (0..replicas, any::<u16>(), 1u8..4)
            .prop_map(|(replica, pos, len)| Action::Delete { replica, pos, len }),
        2 => (0..replicas, 0..replicas, any::<u8>())
            .prop_map(|(from, to, fraction)| Action::Deliver { from, to, fraction }),
    ]
}

fn scenario() -> impl Strategy<Value = (usize, Vec<Action>)> {
    (2usize..=5).prop_flat_map(|replicas| (Just(replicas), prop::collection::vec(action(replicas), 1..60)))
}

fn deliver_prefix(from: &TextReplica, to: &mut TextReplica, count: usize) {
    for event in from.events().iter().take(count) {
        to.receive(event).expect("prefix of a topological order applies");
    }
}

/// Feeds events, in the given topological order, to diamond-types.
fn diamond_types_text(events: &[TextEvent]) -> String {
    let mut oplog = ListOpLog::new();
    let mut lvs: HashMap<DeltaId, usize> = HashMap::new();
    for event in events {
        let agent = oplog.get_or_create_agent_id(&format!("{:032x}", event.span.actor.0));
        let mut parents: Vec<usize> = event.parents.ids().iter().map(|id| lvs[id]).collect();
        parents.sort_unstable();
        parents.dedup();
        let op = match &event.op {
            TextOp::Insert { pos, content } => TextOperation::new_insert(*pos, content),
            TextOp::Delete { pos, len } => TextOperation::new_delete(*pos..*pos + *len),
        };
        let range = oplog.add_operations_remote(agent, &parents, event.span.start.0 as usize, &[op]);
        assert_eq!((range.end - range.start) as u64, event.span.len, "diamond-types accepted every unit");
        for offset in 0..event.span.len {
            lvs.insert(event.span.id_at(offset), range.start + offset as usize);
        }
    }
    oplog.checkout_tip().content().to_string()
}

fn run(replica_count: usize, actions: &[Action]) {
    let mut replicas: Vec<TextReplica> = (0..replica_count).map(|index| TextReplica::new(ActorId(index as u128 + 1))).collect();
    let mut seen: Vec<(Frontier, String)> = Vec::new();

    for action in actions {
        match action {
            Action::Insert { replica, pos, text } => {
                let replica = &mut replicas[*replica];
                let pos = usize::from(*pos) % (replica.doc().len() + 1);
                replica.insert(pos, text).expect("local insert");
            }
            Action::Delete { replica, pos, len } => {
                let replica = &mut replicas[*replica];
                let doc_len = replica.doc().len();
                if doc_len == 0 {
                    continue;
                }
                let pos = usize::from(*pos) % doc_len;
                let len = usize::from(*len).min(doc_len - pos);
                replica.delete(pos, len).expect("local delete");
            }
            Action::Deliver { from, to, fraction } => {
                if from == to {
                    continue;
                }
                let source = replicas[*from].clone();
                let count = source.events().len() * usize::from(*fraction) / 255;
                deliver_prefix(&source, &mut replicas[*to], count);
            }
        }
        for replica in &replicas {
            seen.push((replica.frontier().clone(), replica.text()));
        }
    }

    let snapshot = replicas.clone();
    for replica in &mut replicas {
        for other in snapshot.iter().rev() {
            deliver_prefix(other, replica, other.events().len());
        }
    }

    let text = replicas[0].text();
    for replica in &replicas {
        assert_eq!(replica.text(), text, "replica {} diverged", replica.actor());
        assert_eq!(
            diamond_types_text(replica.events()),
            text,
            "diamond-types disagrees when fed replica {}'s order",
            replica.actor()
        );
    }

    let reference = &replicas[0];
    for (frontier, expected) in &seen {
        assert_eq!(
            &reference.doc().text_at(reference.graph(), frontier),
            expected,
            "text_at({:?})",
            frontier.ids().iter().map(|id| (id.actor.0, id.lamport.0)).collect::<Vec<_>>()
        );
    }
}

proptest! {
    #[test]
    fn replicas_converge_and_match_diamond_types((replicas, actions) in scenario()) {
        run(replicas, &actions);
    }
}

/// Found by the property test: ancestors reached only through an actor's
/// earlier run were missing from version vectors.
#[test]
fn regression_ancestor_through_earlier_run() {
    let ins = |replica, text: &str| Action::Insert { replica, pos: 0, text: text.into() };
    let actions = vec![
        ins(0, "aa"),
        ins(0, "a"),
        ins(2, "a"),
        Action::Deliver { from: 0, to: 1, fraction: 128 },
        ins(2, "a"),
        ins(1, "a"),
        Action::Deliver { from: 2, to: 1, fraction: 128 },
        ins(1, "e"),
    ];
    run(3, &actions);
}
