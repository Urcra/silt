use silt_types::{DeltaId, DeltaSpan, Frontier};

use crate::causal::{CausalGraph, VersionVector};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextOp {
    Insert { pos: usize, content: String },
    Delete { pos: usize, len: usize },
}

impl TextOp {
    /// How many ids the operation consumes: one per inserted or deleted char.
    pub fn units(&self) -> usize {
        match self {
            Self::Insert { content, .. } => content.chars().count(),
            Self::Delete { len, .. } => *len,
        }
    }
}

/// A positional edit, interpreted against the document as of `parents`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEvent {
    pub span: DeltaSpan,
    pub parents: Frontier,
    pub op: TextOp,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TextError {
    #[error("{op:?} is outside a document of length {len} at its parent version")]
    OutOfBounds { op: TextOp, len: usize },
    #[error("span of {span_len} ids does not match an operation of {units} units")]
    SpanLengthMismatch { span_len: u64, units: usize },
    #[error("operation is empty")]
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Item {
    id: DeltaId,
    origin_left: Option<DeltaId>,
    origin_right: Option<DeltaId>,
    deleted_by: Vec<DeltaId>,
    ch: char,
}

impl Item {
    fn exists_at(&self, vv: &VersionVector) -> bool {
        vv.contains(self.id)
    }

    fn visible_at(&self, vv: &VersionVector) -> bool {
        self.exists_at(vv) && !self.deleted_by.iter().any(|id| vv.contains(*id))
    }

    fn is_deleted(&self) -> bool {
        !self.deleted_by.is_empty()
    }
}

/// One text document's replica state: every character ever inserted, in
/// document order, with the delta that inserted it and the deltas that
/// deleted it.
///
/// Concurrent insertions are ordered with the YjsMod rules used by
/// diamond-types, so the two can be checked against each other.
#[derive(Debug, Clone, Default)]
pub struct TextDoc {
    items: Vec<Item>,
}

impl TextDoc {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(&self) -> String {
        self.items
            .iter()
            .filter(|item| !item.is_deleted())
            .map(|item| item.ch)
            .collect()
    }

    pub fn len(&self) -> usize {
        self.items.iter().filter(|item| !item.is_deleted()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn text_at(&self, graph: &CausalGraph, version: &Frontier) -> String {
        let vv = graph.version_vector(version);
        self.items
            .iter()
            .filter(|item| item.visible_at(&vv))
            .map(|item| item.ch)
            .collect()
    }

    /// The delta that inserted each visible character.
    pub fn attribution(&self) -> Vec<(char, DeltaId)> {
        self.items
            .iter()
            .filter(|item| !item.is_deleted())
            .map(|item| (item.ch, item.id))
            .collect()
    }

    /// Checks that `event` is well formed and fits the document as of its
    /// parents, which must already be in `graph`.
    pub fn check(&self, graph: &CausalGraph, event: &TextEvent) -> Result<(), TextError> {
        let units = event.op.units();
        if units == 0 {
            return Err(TextError::Empty);
        }
        if event.span.len != units as u64 {
            return Err(TextError::SpanLengthMismatch {
                span_len: event.span.len,
                units,
            });
        }
        let len = self.len_at(&graph.version_vector(&event.parents));
        let fits = match event.op {
            TextOp::Insert { pos, .. } => pos <= len,
            TextOp::Delete { pos, len: count } => pos + count <= len,
        };
        if !fits {
            return Err(TextError::OutOfBounds {
                op: event.op.clone(),
                len,
            });
        }
        Ok(())
    }

    /// Applies an event whose ids are already in `graph`. Every earlier event
    /// of this document that the event depends on must already be applied.
    pub fn apply(&mut self, graph: &CausalGraph, event: &TextEvent) -> Result<(), TextError> {
        if self.already_applied(event) {
            return Ok(());
        }
        self.check(graph, event)?;
        let vv = graph.version_vector(&event.parents);
        match &event.op {
            TextOp::Insert { pos, content } => {
                self.insert(&vv, event, *pos, content);
                Ok(())
            }
            TextOp::Delete { pos, len } => {
                self.delete(&vv, event, *pos, *len);
                Ok(())
            }
        }
    }

    fn already_applied(&self, event: &TextEvent) -> bool {
        let first = event.span.first();
        match event.op {
            TextOp::Insert { .. } => self.items.iter().any(|item| item.id == first),
            TextOp::Delete { .. } => self.items.iter().any(|item| item.deleted_by.contains(&first)),
        }
    }

    fn insert(&mut self, vv: &VersionVector, event: &TextEvent, pos: usize, content: &str) {
        let left_index = pos
            .checked_sub(1)
            .map(|n| self.nth_visible(vv, n).expect("checked insert position is in bounds"));
        let scan_from = left_index.map_or(0, |index| index + 1);
        let right_index = (scan_from..self.items.len()).find(|index| self.items[*index].exists_at(vv));
        let origin_left = left_index.map(|index| self.items[index].id);
        let origin_right = right_index.map(|index| self.items[index].id);

        let first = Item {
            id: event.span.first(),
            origin_left,
            origin_right,
            deleted_by: Vec::new(),
            ch: content.chars().next().expect("insert has at least one char"),
        };
        let mut dest = self.integration_point(&first, scan_from, right_index);
        let mut previous = first.id;
        self.items.insert(dest, first);
        for (offset, ch) in content.chars().enumerate().skip(1) {
            dest += 1;
            let id = event.span.id_at(offset as u64);
            self.items.insert(
                dest,
                Item {
                    id,
                    origin_left: Some(previous),
                    origin_right,
                    deleted_by: Vec::new(),
                    ch,
                },
            );
            previous = id;
        }
    }

    /// Where `item` goes among the items between its origins, following the
    /// YjsMod rules: concurrent siblings with the same origins are ordered by
    /// id, and subtrees of other siblings are skipped as a whole.
    fn integration_point(&self, item: &Item, scan_from: usize, right_index: Option<usize>) -> usize {
        let my_left = scan_from;
        let my_right = right_index.unwrap_or(self.items.len());
        let mut scan_start = scan_from;
        let mut scanning = false;
        let mut index = scan_from;
        while index < my_right {
            let other = &self.items[index];
            let other_left = self.position_after(other.origin_left);
            if other_left < my_left {
                break;
            }
            if other_left == my_left {
                if other.origin_right == item.origin_right {
                    if item.id < other.id {
                        break;
                    }
                    scanning = false;
                } else if self.position_before(other.origin_right) < my_right {
                    if !scanning {
                        scanning = true;
                        scan_start = index;
                    }
                } else {
                    scanning = false;
                }
            }
            index += 1;
        }
        if scanning { scan_start } else { index }
    }

    fn delete(&mut self, vv: &VersionVector, event: &TextEvent, pos: usize, len: usize) {
        let targets: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.visible_at(vv))
            .map(|(index, _)| index)
            .skip(pos)
            .take(len)
            .collect();
        for (offset, index) in targets.into_iter().enumerate() {
            self.items[index].deleted_by.push(event.span.id_at(offset as u64));
        }
    }

    fn nth_visible(&self, vv: &VersionVector, n: usize) -> Option<usize> {
        self.items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.visible_at(vv))
            .nth(n)
            .map(|(index, _)| index)
    }

    fn len_at(&self, vv: &VersionVector) -> usize {
        self.items.iter().filter(|item| item.visible_at(vv)).count()
    }

    fn index_of(&self, id: DeltaId) -> Option<usize> {
        self.items.iter().position(|item| item.id == id)
    }

    fn position_after(&self, id: Option<DeltaId>) -> usize {
        id.and_then(|id| self.index_of(id)).map_or(0, |index| index + 1)
    }

    fn position_before(&self, id: Option<DeltaId>) -> usize {
        id.and_then(|id| self.index_of(id)).unwrap_or(self.items.len())
    }
}
