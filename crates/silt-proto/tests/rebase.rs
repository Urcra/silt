//! An edit counted against an older read of a doc, moved onto the doc as
//! others since left it, always applies, keeps what they wrote and puts in
//! what the person typed (ADR 0077).

use proptest::prelude::*;
use silt_crdt::CharId;
use silt_proto::edited::{DocEdit, doc_ops, rebased};
use silt_types::{ActorId, DeltaId, Lamport};

fn id(actor: u128, at: u64) -> CharId {
    CharId::Delta(DeltaId::new(ActorId(actor), Lamport(at)))
}

fn text_of(chars: &[(char, CharId)]) -> String {
    chars.iter().map(|(char, _)| char).collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn a_late_edit_applies_and_keeps_others_text(
        read in "[a-d]{0,24}",
        theirs in prop::collection::vec((0usize..30, 0usize..4, "[XY]{0,3}"), 0..4),
        mine in (0usize..30, 0usize..6, "[mn]{0,4}"),
    ) {
        let seen: Vec<(char, CharId)> = read.chars().zip(0..).map(|(char, at)| (char, id(1, at))).collect();
        // Others' edits since: deletions and insertions, by their own ids.
        let mut now = seen.clone();
        for (round, (at, delete, insert)) in theirs.iter().enumerate() {
            let at = (*at).min(now.len());
            let delete = (*delete).min(now.len() - at);
            now.drain(at..at + delete);
            let fresh: Vec<(char, CharId)> = insert.chars().zip(0..).map(|(char, n)| (char, id(100 + round as u128, n))).collect();
            now.splice(at..at, fresh);
        }
        let theirs_now: Vec<CharId> = now.iter().map(|(_, id)| *id).filter(|id| !seen.iter().any(|(_, mine)| mine == id)).collect();
        let (at, delete, insert) = mine;
        let at = at.min(seen.len());
        let delete = delete.min(seen.len() - at);
        let edit = [DocEdit::Splice { pos: at, delete, insert: insert.clone() }];
        let moved = rebased(&seen, &now, &edit).expect("rebases");
        let ops = doc_ops(&text_of(&now), &moved);
        prop_assert!(ops.is_ok(), "applies: {:?}", ops);
        let mut after = now.clone();
        for edit in &moved {
            let DocEdit::Splice { pos, delete, insert } = edit else { unreachable!("rebased gives splices") };
            after.splice(*pos..*pos + *delete, insert.chars().map(|char| (char, id(9, 0))));
        }
        for theirs in &theirs_now {
            prop_assert!(after.iter().any(|(_, kept)| kept == theirs), "others' text stays");
        }
        prop_assert!(text_of(&after).contains(&insert), "what the person typed is in");
    }
}
