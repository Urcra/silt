//! An artifact's page data (ADR 0072): documents at paths, written by
//! `Op::Page` deltas and folded along a thread's lineage, so a fork
//! inherits them, a pull brings upstream's in, and a revert rewinds them.
//! Writes to one path apply in stamp order, so every replica agrees
//! whatever order the deltas arrived in.

use std::collections::BTreeMap;

use silt_types::ArtifactId;

use silt_crdt::Stamp;

use crate::delta::{Op, PageOp};
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

/// `artifact`'s documents, folded from `items`.
pub fn documents(items: &[HistoryItem<'_>], artifact: ArtifactId) -> Documents {
    let mut docs = Documents::new();
    for (_, op) in page_ops(items, artifact) {
        match op {
            PageOp::Set { path, value, .. } => {
                if let Ok(value) = serde_json::from_str(value) {
                    docs.insert(path.clone(), value);
                }
            }
            PageOp::Patched { path, fields, .. } => {
                let Ok(serde_json::Value::Object(fields)) = serde_json::from_str::<serde_json::Value>(fields) else {
                    continue;
                };
                let doc = docs.entry(path.clone()).or_insert_with(|| serde_json::json!({}));
                if !doc.is_object() {
                    *doc = serde_json::json!({});
                }
                if let Some(object) = doc.as_object_mut() {
                    for (field, value) in fields {
                        if value.is_null() {
                            object.remove(&field);
                        } else {
                            object.insert(field, value);
                        }
                    }
                }
            }
            PageOp::Deleted { path, .. } => {
                docs.remove(path);
            }
            PageOp::Asset { asset, blob, name, media_type, size, .. } => {
                let doc = serde_json::json!({"blob": blob.to_string(), "name": name, "media_type": media_type, "size": size});
                docs.insert(format!("_assets/{asset}"), doc);
            }
            PageOp::Text { .. } | PageOp::Lines { .. } | PageOp::Cell { .. } => {}
        }
    }
    docs
}

/// How large `docs` are together, as JSON.
pub fn size(docs: &Documents) -> usize {
    docs.iter().map(|(path, value)| path.len() + value.to_string().len()).sum()
}
