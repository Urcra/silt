//! An artifact's page data (ADR 0072): documents at paths, written by
//! `Op::Page` deltas and folded along a thread's lineage, so a fork
//! inherits them, a pull brings upstream's in, and a revert rewinds them.
//! Writes to one path apply in stamp order, so every replica agrees
//! whatever order the deltas arrived in.

use std::collections::BTreeMap;

use silt_types::ArtifactId;

use silt_crdt::{CausalGraph, CharId, Stamp, TextDoc, TextEvent, TextOp};

use crate::delta::{Axis, Op, PageOp};
use crate::family::HistoryItem;

/// The longest document, as JSON.
pub const DOC_BYTES: usize = 64 * 1024;
/// The most documents one artifact holds.
pub const DOCS: usize = 10_000;
/// The most one artifact's documents hold together, as JSON.
pub const PAGE_BYTES: usize = 16 << 20;
/// The longest text a sheet's cell holds.
pub const CELL_BYTES: usize = 8 * 1024;

/// A document path: 2 to 8 segments of `[A-Za-z0-9_-]{1,64}`, an even
/// count, as documents sit at even depths and collections at odd ones.
pub fn is_document_path(path: &str) -> bool {
    let segments: Vec<&str> = path.split('/').collect();
    (2..=8).contains(&segments.len()) && segments.len().is_multiple_of(2) && segments.iter().all(|segment| is_segment(segment))
}

/// A collection path: 1 to 7 segments, an odd count.
pub fn is_collection_path(path: &str) -> bool {
    let segments: Vec<&str> = path.split('/').collect();
    (1..=7).contains(&segments.len()) && !segments.len().is_multiple_of(2) && segments.iter().all(|segment| is_segment(segment))
}

fn is_segment(segment: &str) -> bool {
    (1..=64).contains(&segment.len()) && segment.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Whether `text` is a JSON value within a document's size.
pub fn is_document(text: &str) -> bool {
    text.len() <= DOC_BYTES && serde_json::from_str::<serde_json::Value>(text).is_ok()
}

/// Whether `text` is a JSON object within a document's size.
pub fn is_fields(text: &str) -> bool {
    text.len() <= DOC_BYTES && serde_json::from_str::<serde_json::Value>(text).is_ok_and(|value| value.is_object())
}

/// An artifact's documents, by path.
pub type Documents = BTreeMap<String, serde_json::Value>;

/// The page ops for `artifact` among `items`, a thread's history with its
/// page ops, as they stand: rewound ones left out.
fn page_ops<'a>(items: &'a [HistoryItem<'a>], artifact: ArtifactId) -> Vec<(Stamp, &'a PageOp)> {
    let mut ops: Vec<(Stamp, &PageOp)> = items
        .iter()
        .filter(|item| !item.rewound)
        .filter_map(|item| match &item.delta.op {
            Op::Page(op) if op.artifact() == artifact => Some((Stamp::from(item.delta.span.first()), op)),
            _ => None,
        })
        .collect();
    ops.sort_by_key(|(stamp, _)| *stamp);
    ops
}

/// `artifact`'s documents, folded from `items`. A write that would take a
/// document past [`DOC_BYTES`], or the artifact past [`DOCS`] or
/// [`PAGE_BYTES`], is left out, as a patch or a peer may write past what
/// each op's own size shows; every replica leaves out the same ones.
pub fn documents(items: &[HistoryItem<'_>], artifact: ArtifactId) -> Documents {
    let mut docs = Documents::new();
    let mut total = 0;
    for (_, op) in page_ops(items, artifact) {
        let (path, next) = match op {
            PageOp::Set { path, value, .. } => {
                let Ok(value) = serde_json::from_str(value) else { continue };
                (path.clone(), Some(value))
            }
            PageOp::Patched { path, fields, .. } => {
                let Ok(serde_json::Value::Object(fields)) = serde_json::from_str::<serde_json::Value>(fields) else {
                    continue;
                };
                let mut doc = docs.get(path).filter(|doc| doc.is_object()).cloned().unwrap_or_else(|| serde_json::json!({}));
                if let Some(object) = doc.as_object_mut() {
                    for (field, value) in fields {
                        if value.is_null() {
                            object.remove(&field);
                        } else {
                            object.insert(field, value);
                        }
                    }
                }
                (path.clone(), Some(doc))
            }
            PageOp::Deleted { path, .. } => (path.clone(), None),
            PageOp::Asset { asset, blob, name, media_type, size, .. } => {
                let doc = serde_json::json!({"blob": blob.to_string(), "name": name, "media_type": media_type, "size": size});
                (format!("_assets/{asset}"), Some(doc))
            }
            PageOp::Text { .. } | PageOp::Lines { .. } | PageOp::Cell { .. } => continue,
        };
        let before = docs.get(&path).map_or(0, |doc| path.len() + doc.to_string().len());
        match next {
            None => {
                if docs.remove(&path).is_some() {
                    total -= before;
                }
            }
            Some(doc) => {
                let length = doc.to_string().len();
                let after = total - before + path.len() + length;
                if length <= DOC_BYTES && after <= PAGE_BYTES && (before > 0 || docs.len() < DOCS) {
                    docs.insert(path, doc);
                    total = after;
                }
            }
        }
    }
    docs
}

/// The text CRDT of a doc's text, or of one axis of one sheet, folded from
/// `items`: the operations that change it, in the history's order, each at
/// the version its delta was made at (ADR 0077).
fn sequence<'a>(items: &[HistoryItem<'a>], graph: &CausalGraph, wanted: impl Fn(&PageOp) -> Option<&TextOp>) -> TextDoc {
    let mut doc = TextDoc::new();
    for item in items.iter().filter(|item| !item.rewound) {
        let Op::Page(op) = &item.delta.op else { continue };
        let Some(text_op) = wanted(op) else { continue };
        let event = TextEvent { span: item.delta.span, parents: item.delta.parents.clone(), op: text_op.clone() };
        // An operation that doesn't fit is a peer's mistake, left out on every replica alike.
        let _ = doc.apply(graph, &event);
    }
    doc
}

/// A doc's text as it stands along `items`.
pub fn doc_text(items: &[HistoryItem<'_>], graph: &CausalGraph, artifact: ArtifactId) -> String {
    doc_crdt(items, graph, artifact).text()
}

/// A doc's text CRDT along `items`, which comments on it anchor in.
pub fn doc_crdt(items: &[HistoryItem<'_>], graph: &CausalGraph, artifact: ArtifactId) -> TextDoc {
    sequence(items, graph, |op| match op {
        PageOp::Text { artifact: of, op } if *of == artifact => Some(op),
        _ => None,
    })
}

/// How many sheets `artifact` has along `items`: one past the last any
/// operation names, and one at least, as a new sheet artifact has.
pub fn sheet_count(items: &[HistoryItem<'_>], artifact: ArtifactId) -> u16 {
    items
        .iter()
        .filter(|item| !item.rewound)
        .filter_map(|item| match &item.delta.op {
            Op::Page(PageOp::Lines { artifact: of, sheet, .. } | PageOp::Cell { artifact: of, sheet, .. }) if *of == artifact => {
                Some(*sheet)
            }
            _ => None,
        })
        .max()
        .map_or(1, |last| last + 1)
}

/// A sheet as it stands: its rows' and columns' ids in order, and each
/// cell's typed text by its row's and column's ids.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SheetCells {
    pub rows: Vec<CharId>,
    pub columns: Vec<CharId>,
    pub cells: BTreeMap<(CharId, CharId), String>,
}

/// Sheet `sheet` of `artifact` along `items`: rows and columns from their
/// sequences, and each cell's last write in stamp order.
pub fn sheet_cells(items: &[HistoryItem<'_>], graph: &CausalGraph, artifact: ArtifactId, sheet: u16) -> SheetCells {
    let axis = |wanted: Axis| {
        sequence(items, graph, |op| match op {
            PageOp::Lines { artifact: of, sheet: on, axis, op } if *of == artifact && *on == sheet && *axis == wanted => Some(op),
            _ => None,
        })
        .attribution()
        .into_iter()
        .map(|(_, id)| id)
        .collect::<Vec<_>>()
    };
    let mut cells = BTreeMap::new();
    for (_, op) in page_ops(items, artifact) {
        if let PageOp::Cell { sheet: on, row, column, raw, .. } = op
            && *on == sheet
        {
            if raw.is_empty() {
                cells.remove(&(*row, *column));
            } else {
                cells.insert((*row, *column), raw.clone());
            }
        }
    }
    // A peer can insert rows and columns past what a sheet holds; every
    // replica keeps the same first ones, so they all draw the same grid.
    let mut columns = axis(Axis::Columns);
    columns.truncate(SHEET_COLUMNS);
    let mut rows = axis(Axis::Rows);
    rows.truncate(SHEET_ROWS.min(SHEET_CELLS / columns.len().max(1)));
    SheetCells { rows, columns, cells }
}

/// The most rows, columns and cells a folded sheet keeps, as `silt-sheet`
/// computes and ADR 0077 bounds a sheet.
pub const SHEET_ROWS: usize = 10_000;
pub const SHEET_COLUMNS: usize = 200;
pub const SHEET_CELLS: usize = 100_000;

/// How large `docs` are together, as JSON.
pub fn size(docs: &Documents) -> usize {
    docs.iter().map(|(path, value)| path.len() + value.to_string().len()).sum()
}
