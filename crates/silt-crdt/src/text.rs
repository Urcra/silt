use serde::{Deserialize, Serialize};
use silt_types::{DeltaId, DeltaSpan, Frontier};

use crate::causal::{CausalGraph, VersionVector};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextEvent {
    pub span: DeltaSpan,
    pub parents: Frontier,
    pub op: TextOp,
}

/// Identity of one character.
///
/// Characters typed into the document carry the id of the delta that
/// inserted them. Characters that came from a git blob are named by the delta
/// that introduced the blob plus their offset in it, so every replica derives
/// the same ids without spending lamports on unedited content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CharId {
    Base { source: DeltaId, offset: u32 },
    Delta(DeltaId),
}

impl CharId {
    fn exists_at(self, vv: &VersionVector) -> bool {
        match self {
            Self::Base { source, .. } => vv.contains(source),
            Self::Delta(id) => vv.contains(id),
        }
    }
}

/// Which side of a position an anchor sticks to when text is inserted there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Bias {
    /// Stays after the character to its left.
    Left,
    /// Stays before the character to its right.
    Right,
}

/// A position that survives edits: it follows the character it is attached
/// to, or the start or end of the document when there is none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Anchor {
    pub char_id: Option<CharId>,
    pub bias: Bias,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedAnchor {
    pub pos: usize,
    /// The character the anchor was attached to is not visible any more.
    pub outdated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TextError {
    #[error("{op:?} is outside a document of length {len} at its parent version")]
    OutOfBounds { op: TextOp, len: usize },
    #[error("span of {span_len} ids does not match an operation of {units} units")]
    SpanLengthMismatch { span_len: u64, units: usize },
    #[error("operation is empty")]
    Empty,
    #[error("anchor position {pos} is outside a document of length {len}")]
    AnchorOutOfBounds { pos: usize, len: usize },
    #[error("anchor refers to character {0:?} that is not in this document")]
    UnknownAnchor(CharId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Item {
    id: CharId,
    origin_left: Option<CharId>,
    origin_right: Option<CharId>,
    deleted_by: Vec<DeltaId>,
    ch: char,
}

impl Item {
    fn exists_at(&self, vv: &VersionVector) -> bool {
        self.id.exists_at(vv)
    }

    fn visible_at(&self, vv: &VersionVector) -> bool {
        self.exists_at(vv) && !self.deleted_by.iter().any(|id| vv.contains(*id))
    }

    fn is_deleted(&self) -> bool {
        !self.deleted_by.is_empty()
    }

    fn is_visible_now(&self) -> bool {
        !self.is_deleted()
    }
}

/// One text document's replica state: every character ever inserted, in
/// document order, with the delta that inserted it and the deltas that
/// deleted it.
///
/// Concurrent insertions are ordered with the YjsMod rules used by
/// diamond-types, so the two can be checked against each other.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TextDoc {
    items: Vec<Item>,
}

impl TextDoc {
    pub fn new() -> Self {
        Self::default()
    }

    /// A document whose initial content is a git blob introduced by `source`.
    /// The content is visible at every version that includes `source`.
    pub fn from_base(source: DeltaId, content: &str) -> Self {
        let mut previous = None;
        let items = content
            .chars()
            .enumerate()
            .map(|(offset, ch)| {
                let id = CharId::Base {
                    source,
                    offset: u32::try_from(offset).expect("base blobs are smaller than 4 GiB of chars"),
                };
                let item = Item {
                    id,
                    origin_left: previous,
                    origin_right: None,
                    deleted_by: Vec::new(),
                    ch,
                };
                previous = Some(id);
                item
            })
            .collect();
        Self { items }
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

    /// An anchor at `pos` in the current text.
    pub fn anchor(&self, pos: usize, bias: Bias) -> Result<Anchor, TextError> {
        let visible: Vec<&Item> = self.items.iter().filter(|item| !item.is_deleted()).collect();
        if pos > visible.len() {
            return Err(TextError::AnchorOutOfBounds {
                pos,
                len: visible.len(),
            });
        }
        let char_id = match bias {
            Bias::Left => pos.checked_sub(1).map(|index| visible[index].id),
            Bias::Right => visible.get(pos).map(|item| item.id),
        };
        Ok(Anchor { char_id, bias })
    }

    /// Where `anchor` is in the current text.
    pub fn resolve(&self, anchor: &Anchor) -> Result<ResolvedAnchor, TextError> {
        self.resolve_with(anchor, Item::is_visible_now)
    }

    /// Where `anchor` is in the text as of `version`.
    pub fn resolve_at(
        &self,
        graph: &CausalGraph,
        version: &Frontier,
        anchor: &Anchor,
    ) -> Result<ResolvedAnchor, TextError> {
        let vv = graph.version_vector(version);
        self.resolve_with(anchor, |item| item.visible_at(&vv))
    }

    fn resolve_with(
        &self,
        anchor: &Anchor,
        visible: impl Fn(&Item) -> bool,
    ) -> Result<ResolvedAnchor, TextError> {
        let Some(char_id) = anchor.char_id else {
            let pos = match anchor.bias {
                Bias::Left => 0,
                Bias::Right => self.items.iter().filter(|item| visible(item)).count(),
            };
            return Ok(ResolvedAnchor { pos, outdated: false });
        };
        let index = self.index_of(char_id).ok_or(TextError::UnknownAnchor(char_id))?;
        let before = self.items[..index].iter().filter(|item| visible(item)).count();
        let target_visible = visible(&self.items[index]);
        let pos = match anchor.bias {
            Bias::Left if target_visible => before + 1,
            Bias::Left | Bias::Right => before,
        };
        Ok(ResolvedAnchor {
            pos,
            outdated: !target_visible,
        })
    }

    /// The delta that inserted each visible character.
    pub fn attribution(&self) -> Vec<(char, CharId)> {
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
            TextOp::Insert { .. } => self.items.iter().any(|item| item.id == CharId::Delta(first)),
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
            id: CharId::Delta(event.span.first()),
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
            let id = CharId::Delta(event.span.id_at(offset as u64));
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

    fn index_of(&self, id: CharId) -> Option<usize> {
        self.items.iter().position(|item| item.id == id)
    }

    fn position_after(&self, id: Option<CharId>) -> usize {
        id.and_then(|id| self.index_of(id)).map_or(0, |index| index + 1)
    }

    fn position_before(&self, id: Option<CharId>) -> usize {
        id.and_then(|id| self.index_of(id)).unwrap_or(self.items.len())
    }
}
