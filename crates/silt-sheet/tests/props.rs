//! Whatever a peer types into a sheet's cells, evaluating it ends, gives
//! the same values every time, and never panics.

use proptest::prelude::*;
use silt_sheet::{Sheet, evaluate};

/// Cell text from the pieces formulas are made of, nested and repeated.
fn cell() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        Just("(".to_owned()),
        Just(")".to_owned()),
        Just("-".to_owned()),
        Just("+".to_owned()),
        Just("^".to_owned()),
        Just("&".to_owned()),
        Just(",".to_owned()),
        Just(":".to_owned()),
        Just("SUM(".to_owned()),
        Just("IF(".to_owned()),
        Just("CONCAT(".to_owned()),
        Just("DATE(".to_owned()),
        Just("ROUND(".to_owned()),
        Just("\"x\"".to_owned()),
        Just("1e308".to_owned()),
        Just("A1".to_owned()),
        Just("B2".to_owned()),
        Just("C1:C3".to_owned()),
        Just("ZZ9999".to_owned()),
        (0u32..1000).prop_map(|n| n.to_string()),
    ];
    prop_oneof![
        (0u32..100).prop_map(|n| n.to_string()),
        "[a-z ]{0,8}",
        prop::collection::vec(piece, 0..40).prop_map(|pieces| format!("={}", pieces.concat())),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn any_sheet_evaluates_alike_and_ends(cells in prop::collection::vec(prop::collection::vec(cell(), 3), 1..4)) {
        let sheet = Sheet { cells: &cells, today: 46_000.0 };
        let once = evaluate(&sheet);
        let again = evaluate(&sheet);
        prop_assert_eq!(&once, &again);
        prop_assert_eq!(once.len(), cells.len());
    }

    #[test]
    fn nesting_of_any_depth_is_an_answer(depth in 0usize..5000) {
        let cells = vec![vec![format!("={}1{}", "(".repeat(depth), ")".repeat(depth))]];
        let sheet = Sheet { cells: &cells, today: 46_000.0 };
        prop_assert_eq!(evaluate(&sheet).len(), 1);
    }
}
