//! What docs, sheets and page data cost to fold, in process: each is folded
//! along the thread's history whenever it's read (ADRs 0072, 0077).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use silt_log::{Applied, Author, Axis, Family, Op, PageOp};
use silt_types::{ActorId, ArtifactId, ThreadId};

use crate::budgets::{DOC_REPLAY, Measured, PAGE_FOLD, SHEET_EVAL};

const THREAD: ThreadId = ThreadId(1);
const ACTOR: ActorId = ActorId(1);
/// Page documents written, over this many paths.
const PAGE_WRITES: usize = 10_000;
const PAGE_PATHS: usize = 1_000;
/// Words typed into a doc, one edit each.
const DOC_EDITS: usize = 20_000;
const SHEET_ROWS: usize = 1_000;
const SHEET_COLUMNS: usize = 10;

fn artifact() -> ArtifactId {
    "0123456789ab".parse().expect("an artifact id")
}

fn write(family: &mut Family, op: PageOp) {
    let head = family.thread_head(THREAD);
    let delta = family.delta(ACTOR, THREAD, Author::User, 0, head, Op::Page(op));
    let applied = family.apply(delta, &mut HashMap::new());
    debug_assert_eq!(applied, Ok(Applied::New));
}

/// The fastest of three runs of `run`.
fn fastest(mut run: impl FnMut()) -> Measured {
    let best = (0..3)
        .map(|_| {
            let start = Instant::now();
            run();
            start.elapsed()
        })
        .min()
        .unwrap_or(Duration::ZERO);
    Measured::Time(best)
}

pub fn measure() -> Vec<(&'static str, Measured)> {
    let artifact = artifact();
    let mut pages = Family::new();
    for at in 0..PAGE_WRITES {
        let path = format!("votes/v{}", at % PAGE_PATHS);
        write(&mut pages, PageOp::Set { artifact, path, value: format!("{{\"pick\":{at}}}") });
    }
    let page_fold = fastest(|| assert_eq!(pages.page_documents(THREAD, artifact).len(), PAGE_PATHS));

    let mut doc = Family::new();
    let mut length = 0;
    for at in 0..DOC_EDITS {
        let content = format!("word{} ", at % 97);
        let pos = length;
        length += content.chars().count();
        write(&mut doc, PageOp::Text { artifact, op: silt_crdt::TextOp::Insert { pos, content } });
    }
    let doc_replay = fastest(|| assert_eq!(doc.doc_text(THREAD, artifact).chars().count(), length));

    let mut sheet = Family::new();
    let lines = |axis, count: usize| PageOp::Lines {
        artifact,
        sheet: 0,
        axis,
        op: silt_crdt::TextOp::Insert { pos: 0, content: "x".repeat(count) },
    };
    write(&mut sheet, lines(Axis::Rows, SHEET_ROWS));
    write(&mut sheet, lines(Axis::Columns, SHEET_COLUMNS));
    let cells = sheet.sheet_cells(THREAD, artifact, 0);
    let mut set = std::collections::BTreeMap::new();
    for row in 1..=SHEET_ROWS {
        for (at, column) in ('A'..='J').enumerate() {
            let raw = if column == 'J' { format!("=SUM(A{row}:I{row})") } else { (row * SHEET_COLUMNS + at).to_string() };
            set.insert(format!("{column}{row}"), raw);
        }
    }
    for op in silt_proto::edited::cell_ops(artifact, 0, &cells, &set).expect("every cell is in the sheet") {
        write(&mut sheet, op);
    }
    let sheet_eval = fastest(|| {
        let view = silt_proto::edited::sheet_view(&sheet.sheet_cells(THREAD, artifact, 0), 0.0);
        assert_eq!(view.values.len(), SHEET_ROWS);
    });
    vec![(PAGE_FOLD, page_fold), (DOC_REPLAY, doc_replay), (SHEET_EVAL, sheet_eval)]
}
