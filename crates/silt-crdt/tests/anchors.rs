use proptest::prelude::*;
use silt_crdt::{Anchor, Bias, ResolvedAnchor, TextError, TextReplica};
use silt_types::ActorId;

const A: ActorId = ActorId(1);
const B: ActorId = ActorId(2);

fn resolved(pos: usize) -> ResolvedAnchor {
    ResolvedAnchor { pos, outdated: false }
}

#[test]
fn anchors_follow_their_character_through_edits() {
    let mut replica = TextReplica::new(A);
    replica.insert(0, "hello world").expect("insert");
    let before_w = replica.doc().anchor(6, Bias::Right).expect("anchor");
    let after_o = replica.doc().anchor(5, Bias::Left).expect("anchor");
    replica.insert(0, ">> ").expect("insert");
    replica.insert(8, ",").expect("insert");
    assert_eq!(replica.text(), ">> hello, world");
    assert_eq!(replica.doc().resolve(&before_w), Ok(resolved(10)));
    assert_eq!(replica.doc().resolve(&after_o), Ok(resolved(8)));
}

#[test]
fn bias_decides_which_side_of_an_insertion_the_anchor_lands() {
    let mut replica = TextReplica::new(A);
    replica.insert(0, "ab").expect("insert");
    let left = replica.doc().anchor(1, Bias::Left).expect("anchor");
    let right = replica.doc().anchor(1, Bias::Right).expect("anchor");
    replica.insert(1, "XYZ").expect("insert");
    assert_eq!(replica.doc().resolve(&left), Ok(resolved(1)));
    assert_eq!(replica.doc().resolve(&right), Ok(resolved(4)));
}

#[test]
fn deleted_target_is_outdated_but_still_placed() {
    let mut replica = TextReplica::new(A);
    replica.insert(0, "abcdef").expect("insert");
    let version = replica.frontier().clone();
    let on_d = replica.doc().anchor(3, Bias::Right).expect("anchor");
    replica.delete(2, 3).expect("delete cde");
    assert_eq!(replica.text(), "abf");
    assert_eq!(
        replica.doc().resolve(&on_d),
        Ok(ResolvedAnchor {
            pos: 2,
            outdated: true
        })
    );
    assert_eq!(
        replica.doc().resolve_at(replica.graph(), &version, &on_d),
        Ok(resolved(3))
    );
}

#[test]
fn document_edges() {
    let mut replica = TextReplica::new(A);
    replica.insert(0, "abc").expect("insert");
    let start = replica.doc().anchor(0, Bias::Left).expect("anchor");
    let end = replica.doc().anchor(3, Bias::Right).expect("anchor");
    assert_eq!(start, Anchor { char_id: None, bias: Bias::Left });
    assert_eq!(end, Anchor { char_id: None, bias: Bias::Right });
    replica.insert(0, "<").expect("insert");
    replica.insert(4, ">").expect("insert");
    assert_eq!(replica.doc().resolve(&start), Ok(resolved(0)));
    assert_eq!(replica.doc().resolve(&end), Ok(resolved(5)));
    assert!(matches!(
        replica.doc().anchor(9, Bias::Left),
        Err(TextError::AnchorOutOfBounds { pos: 9, len: 5 })
    ));
}

#[test]
fn anchors_survive_concurrent_edits_from_other_replicas() {
    let mut a = TextReplica::new(A);
    a.insert(0, "fn main() {}").expect("insert");
    let mut b = TextReplica::new(B);
    for event in a.events() {
        b.receive(event).expect("sync");
    }
    let brace = a.doc().anchor(10, Bias::Right).expect("anchor on {");
    a.insert(0, "pub ").expect("a edits");
    b.insert(9, " -> u8").expect("b edits");
    for event in b.events().to_vec() {
        a.receive(&event).expect("sync");
    }
    assert_eq!(a.text(), "pub fn main() -> u8 {}");
    assert_eq!(a.doc().resolve(&brace), Ok(resolved(20)));
}

#[derive(Debug, Clone)]
enum Edit {
    Insert { replica: bool, pos: u16, text: String },
    Delete { replica: bool, pos: u16, len: u8 },
}

fn edit() -> impl Strategy<Value = Edit> {
    prop_oneof![
        (any::<bool>(), any::<u16>(), "[a-z]{1,3}")
            .prop_map(|(replica, pos, text)| Edit::Insert { replica, pos, text }),
        (any::<bool>(), any::<u16>(), 1u8..3)
            .prop_map(|(replica, pos, len)| Edit::Delete { replica, pos, len }),
    ]
}

proptest! {
    #[test]
    fn anchored_character_is_found_wherever_it_moves(
        initial in "[a-z]{1,12}",
        target in any::<u16>(),
        bias in prop_oneof![Just(Bias::Left), Just(Bias::Right)],
        edits in prop::collection::vec(edit(), 0..30),
    ) {
        let mut a = TextReplica::new(A);
        a.insert(0, &initial).expect("initial");
        let mut b = TextReplica::new(B);
        for event in a.events() {
            b.receive(event).expect("sync");
        }
        let original = a.frontier().clone();
        let len = a.doc().len();
        let pos = match bias {
            Bias::Left => usize::from(target) % len + 1,
            Bias::Right => usize::from(target) % len,
        };
        let anchor = a.doc().anchor(pos, bias).expect("anchor");
        let char_id = anchor.char_id.expect("anchored to a character");

        for edit in &edits {
            let (replica, op_pos, apply_delete, text, del_len) = match edit {
                Edit::Insert { replica, pos, text } => (*replica, *pos, false, text.as_str(), 0),
                Edit::Delete { replica, pos, len } => (*replica, *pos, true, "", *len),
            };
            let replica = if replica { &mut a } else { &mut b };
            let doc_len = replica.doc().len();
            if apply_delete {
                if doc_len == 0 {
                    continue;
                }
                let start = usize::from(op_pos) % doc_len;
                let count = usize::from(del_len).min(doc_len - start);
                replica.delete(start, count).expect("delete");
            } else {
                replica.insert(usize::from(op_pos) % (doc_len + 1), text).expect("insert");
            }
        }
        for event in b.events().to_vec() {
            a.receive(&event).expect("sync");
        }

        let resolved = a.doc().resolve(&anchor).expect("resolves");
        let attribution = a.doc().attribution();
        match attribution.iter().position(|(_, id)| *id == char_id) {
            Some(index) => {
                prop_assert!(!resolved.outdated);
                let expected = match bias {
                    Bias::Left => index + 1,
                    Bias::Right => index,
                };
                prop_assert_eq!(resolved.pos, expected);
            }
            None => prop_assert!(resolved.outdated),
        }
        prop_assert_eq!(
            a.doc().resolve_at(a.graph(), &original, &anchor).expect("resolves").pos,
            pos
        );
    }
}

#[test]
fn anchors_made_at_an_old_version_follow_their_character() {
    let mut replica = TextReplica::new(A);
    replica.insert(0, "line one\nline two\n").expect("insert");
    let old = replica.frontier().clone();
    replica.insert(0, "new first\n").expect("insert");
    let at_old = replica
        .doc()
        .anchor_at(replica.graph(), &old, 9, Bias::Right)
        .expect("anchor at the old version");
    assert_eq!(at_old, replica.doc().anchor(19, Bias::Right).expect("same character now"));
    assert_eq!(replica.doc().resolve(&at_old), Ok(resolved(19)));
    assert!(matches!(
        replica.doc().anchor_at(replica.graph(), &old, 99, Bias::Left),
        Err(TextError::AnchorOutOfBounds { .. })
    ));
}
