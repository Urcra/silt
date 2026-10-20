use std::cmp::Ordering;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use silt_sumtree::{Bias as SeekBias, ContextLessSummary, Dimension, Dimensions, Item, SumTree};
use silt_types::{DeltaId, DeltaSpan, Frontier};
use smallvec::SmallVec;

use crate::causal::{CausalGraph, VersionVector};
use crate::locator::Locator;

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
    fn offset(self, by: u32) -> Self {
        match self {
            Self::Base { source, offset } => Self::Base {
                source,
                offset: offset + by,
            },
            Self::Delta(id) => Self::Delta(DeltaId::new(id.actor, id.lamport.offset(u64::from(by)))),
        }
    }

    /// How far `self` is past `start` when both belong to the same run of ids.
    fn distance_from(self, start: Self) -> Option<u32> {
        match (start, self) {
            (Self::Base { source: a, offset: from }, Self::Base { source: b, offset: to }) if a == b => {
                to.checked_sub(from)
            }
            (Self::Delta(from), Self::Delta(to)) if from.actor == to.actor => {
                u32::try_from(to.lamport.0.checked_sub(from.lamport.0)?).ok()
            }
            _ => None,
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


const NOT_INSERTED: u32 = 0;
const VISIBLE: u32 = 1;

/// The characters of one insertion (an insert event or a base blob), and the
/// fragments they are currently split into.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Insertion {
    chars: Vec<char>,
    origin_left: Option<CharId>,
    origin_right: Option<CharId>,
    /// Offset within the insertion where each fragment starts.
    splits: BTreeMap<u32, Locator>,
}

/// A run of consecutive characters from one insertion that share their state.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Fragment {
    locator: Locator,
    insertion: CharId,
    offset: u32,
    len: u32,
    /// State in the version currently being merged into: not yet inserted,
    /// visible, or deleted by `prepare - 1` deletes.
    prepare: u32,
    /// The delete that removed this fragment's first character, one entry
    /// per deleting event; the character at offset `k` was removed by `id + k`.
    deleted_by: SmallVec<[DeltaId; 1]>,
}

impl Fragment {
    fn first_char(&self) -> CharId {
        self.insertion.offset(self.offset)
    }

    fn is_deleted(&self) -> bool {
        !self.deleted_by.is_empty()
    }

    /// How many of the fragment's characters are visible at `vv`. Typing
    /// extends fragments across insert events, so the characters that exist
    /// at an earlier version are a prefix; deletes always cover whole
    /// fragments.
    fn visible_len_at(&self, vv: &VersionVector) -> usize {
        if self.deleted_by.iter().any(|id| vv.contains(*id)) {
            return 0;
        }
        match self.first_char() {
            CharId::Base { source, .. } => {
                if vv.contains(source) {
                    self.len()
                } else {
                    0
                }
            }
            CharId::Delta(first) => vv.get(first.actor).map_or(0, |max| {
                let present = max.0.saturating_add(1).saturating_sub(first.lamport.0);
                usize::try_from(present).unwrap_or(usize::MAX).min(self.len())
            }),
        }
    }

    fn len(&self) -> usize {
        self.len as usize
    }
}

#[derive(Debug, Clone, Default)]
struct FragmentSummary {
    max_locator: Locator,
    visible: usize,
    prepare_visible: usize,
}

impl ContextLessSummary for FragmentSummary {
    fn zero() -> Self {
        Self::default()
    }

    fn add_summary(&mut self, summary: &Self) {
        self.max_locator.clone_from(&summary.max_locator);
        self.visible += summary.visible;
        self.prepare_visible += summary.prepare_visible;
    }
}

impl Item for Fragment {
    type Summary = FragmentSummary;

    fn summary(&self, (): ()) -> FragmentSummary {
        FragmentSummary {
            max_locator: self.locator.clone(),
            visible: if self.is_deleted() { 0 } else { self.len() },
            prepare_visible: if self.prepare == VISIBLE { self.len() } else { 0 },
        }
    }
}

impl Dimension<'_, FragmentSummary> for Locator {
    fn zero((): ()) -> Self {
        Self::default()
    }

    fn add_summary(&mut self, summary: &FragmentSummary, (): ()) {
        self.clone_from(&summary.max_locator);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
struct VisibleLen(usize);

impl Dimension<'_, FragmentSummary> for VisibleLen {
    fn zero((): ()) -> Self {
        Self::default()
    }

    fn add_summary(&mut self, summary: &FragmentSummary, (): ()) {
        self.0 += summary.visible;
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
struct PrepareLen(usize);

impl Dimension<'_, FragmentSummary> for PrepareLen {
    fn zero((): ()) -> Self {
        Self::default()
    }

    fn add_summary(&mut self, summary: &FragmentSummary, (): ()) {
        self.0 += summary.prepare_visible;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum AppliedKind {
    Insert { start: CharId, len: u32 },
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Applied {
    id: DeltaId,
    kind: AppliedKind,
}

/// One text document's replica state, following Eg-walker.
///
/// Every character ever inserted is kept, in document order, in run-length
/// fragments of a persistent B-tree ordered by locators. Besides its final
/// state, each fragment tracks its state in a *prepare* version. To apply an
/// event, the prepare version is moved to the event's parents by retreating
/// and advancing events, so the event's positions can be read directly from
/// the tree. Sequential edits never move it. Concurrent insertions at the
/// same place are ordered with the YjsMod rules used by diamond-types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(from = "TextDocState", into = "TextDocState")]
pub struct TextDoc {
    fragments: SumTree<Fragment>,
    insertions: BTreeMap<CharId, Insertion>,
    deletes: BTreeMap<DeltaId, Vec<(CharId, u32)>>,
    applied: Vec<Applied>,
    prepare: Frontier,
}

#[derive(Serialize, Deserialize)]
struct TextDocState {
    fragments: Vec<Fragment>,
    insertions: BTreeMap<CharId, Insertion>,
    deletes: BTreeMap<DeltaId, Vec<(CharId, u32)>>,
    applied: Vec<Applied>,
    prepare: Frontier,
}

impl From<TextDocState> for TextDoc {
    fn from(state: TextDocState) -> Self {
        Self {
            fragments: SumTree::from_iter(state.fragments, ()),
            insertions: state.insertions,
            deletes: state.deletes,
            applied: state.applied,
            prepare: state.prepare,
        }
    }
}

impl From<TextDoc> for TextDocState {
    fn from(doc: TextDoc) -> Self {
        Self {
            fragments: doc.fragments.iter().cloned().collect(),
            insertions: doc.insertions,
            deletes: doc.deletes,
            applied: doc.applied,
            prepare: doc.prepare,
        }
    }
}

impl Default for TextDoc {
    fn default() -> Self {
        Self {
            fragments: SumTree::new(()),
            insertions: BTreeMap::new(),
            deletes: BTreeMap::new(),
            applied: Vec::new(),
            prepare: Frontier::root(),
        }
    }
}

impl TextDoc {
    pub fn new() -> Self {
        Self::default()
    }

    /// A document whose initial content is a git blob introduced by `source`.
    /// The content is visible at every version that includes `source`.
    pub fn from_base(source: DeltaId, content: &str) -> Self {
        let mut doc = Self::new();
        let key = CharId::Base { source, offset: 0 };
        let chars: Vec<char> = content.chars().collect();
        let mut splits = BTreeMap::new();
        if !chars.is_empty() {
            let locator = Locator::between(&Locator::min(), &Locator::max());
            splits.insert(0, locator.clone());
            doc.fragments.push(
                Fragment {
                    locator,
                    insertion: key,
                    offset: 0,
                    len: run_len(chars.len()),
                    prepare: VISIBLE,
                    deleted_by: SmallVec::new(),
                },
                (),
            );
        }
        doc.insertions.insert(
            key,
            Insertion {
                chars,
                origin_left: None,
                origin_right: None,
                splits,
            },
        );
        doc.applied.push(Applied {
            id: source,
            kind: AppliedKind::Insert {
                start: key,
                len: run_len(content.chars().count()),
            },
        });
        doc.prepare = Frontier::single(source);
        doc
    }

    pub fn text(&self) -> String {
        let mut text = String::with_capacity(self.len());
        for fragment in self.fragments.iter().filter(|fragment| !fragment.is_deleted()) {
            text.extend(self.chars_of(fragment));
        }
        text
    }

    pub fn len(&self) -> usize {
        self.fragments.summary().visible
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn text_at(&self, graph: &CausalGraph, version: &Frontier) -> String {
        let vv = graph.version_vector(version);
        let mut text = String::new();
        for fragment in self.fragments.iter() {
            text.extend(&self.chars_of(fragment)[..fragment.visible_len_at(&vv)]);
        }
        text
    }

    /// An anchor at `pos` in the current text.
    pub fn anchor(&self, pos: usize, bias: Bias) -> Result<Anchor, TextError> {
        let len = self.len();
        if pos > len {
            return Err(TextError::AnchorOutOfBounds { pos, len });
        }
        let char_id = match bias {
            Bias::Left if pos == 0 => None,
            Bias::Left => Some(self.visible_char(pos - 1)),
            Bias::Right if pos == len => None,
            Bias::Right => Some(self.visible_char(pos)),
        };
        Ok(Anchor { char_id, bias })
    }

    /// Where `anchor` is in the current text.
    pub fn resolve(&self, anchor: &Anchor) -> Result<ResolvedAnchor, TextError> {
        let Some(char_id) = anchor.char_id else {
            let pos = match anchor.bias {
                Bias::Left => 0,
                Bias::Right => self.len(),
            };
            return Ok(ResolvedAnchor { pos, outdated: false });
        };
        let (locator, offset) =
            locate(&self.insertions, char_id).ok_or(TextError::UnknownAnchor(char_id))?;
        let mut cursor = self.fragments.cursor::<Dimensions<Locator, VisibleLen>>(());
        cursor.seek(&locator, SeekBias::Left);
        let fragment = cursor.item().expect("located fragment is in the tree");
        let before = cursor.start().1.0;
        Ok(place(before, offset as usize, !fragment.is_deleted(), anchor.bias))
    }

    /// Where `anchor` is in the text as of `version`.
    pub fn resolve_at(
        &self,
        graph: &CausalGraph,
        version: &Frontier,
        anchor: &Anchor,
    ) -> Result<ResolvedAnchor, TextError> {
        let vv = graph.version_vector(version);
        let Some(char_id) = anchor.char_id else {
            let pos = match anchor.bias {
                Bias::Left => 0,
                Bias::Right => self.fragments.iter().map(|f| f.visible_len_at(&vv)).sum(),
            };
            return Ok(ResolvedAnchor { pos, outdated: false });
        };
        let (locator, offset) =
            locate(&self.insertions, char_id).ok_or(TextError::UnknownAnchor(char_id))?;
        let mut before = 0;
        for fragment in self.fragments.iter() {
            let visible = fragment.visible_len_at(&vv);
            if fragment.locator == locator {
                let offset = offset as usize;
                return Ok(place(before, offset, offset < visible, anchor.bias));
            }
            before += visible;
        }
        Err(TextError::UnknownAnchor(char_id))
    }

    /// The character id of each visible character, in order.
    pub fn attribution(&self) -> Vec<(char, CharId)> {
        let mut attribution = Vec::with_capacity(self.len());
        for fragment in self.fragments.iter().filter(|fragment| !fragment.is_deleted()) {
            let first = fragment.first_char();
            attribution.extend(
                self.chars_of(fragment)
                    .iter()
                    .zip(0..)
                    .map(|(ch, offset)| (*ch, first.offset(offset))),
            );
        }
        attribution
    }

    /// Checks that `event` is well formed and fits the document as of its
    /// parents, which must already be in `graph`.
    pub fn check(&mut self, graph: &CausalGraph, event: &TextEvent) -> Result<(), TextError> {
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
        self.move_prepare_to(graph, &event.parents);
        let len = self.fragments.summary().prepare_visible;
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
        let kind = match &event.op {
            TextOp::Insert { pos, content } => {
                self.insert(event, *pos, content);
                AppliedKind::Insert {
                    start: CharId::Delta(event.span.first()),
                    len: run_len(event.op.units()),
                }
            }
            TextOp::Delete { pos, len } => {
                self.delete(event, *pos, *len);
                AppliedKind::Delete
            }
        };
        self.applied.push(Applied {
            id: event.span.first(),
            kind,
        });
        self.prepare = Frontier::single(event.span.last());
        Ok(())
    }

    fn already_applied(&self, event: &TextEvent) -> bool {
        let first = event.span.first();
        match event.op {
            TextOp::Insert { .. } => insertion_of(&self.insertions, CharId::Delta(first)).is_some(),
            TextOp::Delete { .. } => self.deletes.contains_key(&first),
        }
    }

    fn chars_of(&self, fragment: &Fragment) -> &[char] {
        let start = fragment.offset as usize;
        &self.insertions[&fragment.insertion].chars[start..start + fragment.len()]
    }

    fn visible_char(&self, index: usize) -> CharId {
        let mut cursor = self.fragments.cursor::<VisibleLen>(());
        cursor.seek(&VisibleLen(index), SeekBias::Right);
        let fragment = cursor.item().expect("index is within the visible text");
        fragment.first_char().offset(run_len(index - cursor.start().0))
    }

    /// Retreats and advances events until the prepare state is the document
    /// as of `target`.
    fn move_prepare_to(&mut self, graph: &CausalGraph, target: &Frontier) {
        if *target == self.prepare {
            return;
        }
        let in_prepare = graph.version_vector(&self.prepare);
        let in_target = graph.version_vector(target);
        let applied = std::mem::take(&mut self.applied);
        for event in applied.iter().rev() {
            if in_prepare.contains(event.id) && !in_target.contains(event.id) {
                self.shift_prepare(event, -1);
            }
        }
        for event in &applied {
            if !in_prepare.contains(event.id) && in_target.contains(event.id) {
                self.shift_prepare(event, 1);
            }
        }
        self.applied = applied;
        self.prepare = target.clone();
    }

    fn shift_prepare(&mut self, event: &Applied, step: i32) {
        let ranges: Vec<(CharId, u32)> = match event.kind {
            AppliedKind::Insert { start, len } => vec![(start, len)],
            AppliedKind::Delete => self.deletes[&event.id].clone(),
        };
        for (start, len) in ranges {
            self.update_range(start, len, |fragment| {
                fragment.prepare = fragment
                    .prepare
                    .checked_add_signed(step)
                    .expect("prepare state never goes below not-inserted");
            });
        }
    }

    /// Applies `update` to the characters `start..start + len` of one
    /// insertion, splitting fragments at the range's edges.
    fn update_range(&mut self, start: CharId, len: u32, update: impl Fn(&mut Fragment)) {
        let (key, mut offset) = insertion_of(&self.insertions, start).expect("range start exists");
        let end = offset + len;
        while offset < end {
            let (fragment_start, locator) = self.insertions[&key]
                .splits
                .range(..=offset)
                .next_back()
                .map(|(start, locator)| (*start, locator.clone()))
                .expect("every offset is inside a fragment");
            let mut cursor = self.fragments.cursor::<Locator>(());
            let mut fragments = cursor.slice(&locator, SeekBias::Left);
            let mut fragment = cursor.item().expect("fragment with locator exists").clone();
            cursor.next();
            let next = cursor.item().map_or_else(Locator::max, |next| next.locator.clone());
            if fragment_start < offset {
                let right = split(&mut self.insertions, &mut fragment, offset - fragment_start, &next);
                fragments.push(fragment, ());
                fragment = right;
            }
            let keep = end.min(fragment.offset + fragment.len) - fragment.offset;
            let tail = (keep < fragment.len).then(|| split(&mut self.insertions, &mut fragment, keep, &next));
            update(&mut fragment);
            offset = fragment.offset + fragment.len;
            fragments.push(fragment, ());
            if let Some(tail) = tail {
                fragments.push(tail, ());
            }
            fragments.append(cursor.suffix(), ());
            drop(cursor);
            self.fragments = fragments;
        }
    }

    fn insert(&mut self, event: &TextEvent, pos: usize, content: &str) {
        let my_id = CharId::Delta(event.span.first());
        if let Some((locator, key)) = self.typing_target(pos, my_id) {
            let added = run_len(event.op.units());
            self.fragments
                .update_item(|summary| summary.max_locator >= locator, |fragment| fragment.len += added, ());
            self.insertions
                .get_mut(&key)
                .expect("fragment's insertion exists")
                .chars
                .extend(content.chars());
            return;
        }
        if self.insert_in_place(event, pos, content) {
            return;
        }
        let mut cursor = self.fragments.cursor::<PrepareLen>(());
        let mut fragments = cursor.slice(&PrepareLen(pos), SeekBias::Left);

        let mut pending: Option<Fragment> = None;
        let mut left: Option<Fragment> = None;
        if pos > 0 {
            let mut fragment = cursor.item().expect("insert position is in bounds").clone();
            let keep = run_len(pos - cursor.start().0);
            cursor.next();
            if keep < fragment.len {
                let next = cursor.item().map_or_else(Locator::max, |next| next.locator.clone());
                pending = Some(split(&mut self.insertions, &mut fragment, keep, &next));
            }
            left = Some(fragment);
        }
        let origin_left = left.as_ref().map(|fragment| fragment.first_char().offset(fragment.len - 1));
        let left_was_split = pending.is_some();

        let mut region: Vec<Fragment> = Vec::new();
        let right_fragment = loop {
            let next = match pending.take() {
                Some(fragment) => fragment,
                None => match cursor.item() {
                    Some(fragment) => {
                        cursor.next();
                        fragment.clone()
                    }
                    None => break None,
                },
            };
            if next.prepare == NOT_INSERTED {
                region.push(next);
            } else {
                break Some(next);
            }
        };
        let origin_right = right_fragment.as_ref().map(Fragment::first_char);

        let chars: Vec<char> = content.chars().collect();
        if region.is_empty()
            && !left_was_split
            && let Some(fragment) = left.as_mut()
            && self.continues_insertion(fragment, my_id, origin_right)
        {
            fragment.len += run_len(chars.len());
            self.insertions
                .get_mut(&fragment.insertion)
                .expect("fragment's insertion exists")
                .chars
                .extend(chars);
            fragments.extend(left, ());
            fragments.extend(right_fragment, ());
            fragments.append(cursor.suffix(), ());
            drop(cursor);
            self.fragments = fragments;
            return;
        }
        fragments.extend(left, ());

        let dest = integration_point(&self.insertions, &region, my_id, origin_left, origin_right);
        let previous = match dest.checked_sub(1) {
            Some(index) => region[index].locator.clone(),
            None => fragments.last().map_or_else(Locator::min, |last| last.locator.clone()),
        };
        let next = region
            .get(dest)
            .or(right_fragment.as_ref())
            .map(|fragment| fragment.locator.clone())
            .or_else(|| cursor.item().map(|fragment| fragment.locator.clone()))
            .unwrap_or_else(Locator::max);
        let locator = Locator::between(&previous, &next);

        let inserted = Fragment {
            locator: locator.clone(),
            insertion: my_id,
            offset: 0,
            len: run_len(chars.len()),
            prepare: VISIBLE,
            deleted_by: SmallVec::new(),
        };
        let after_inserted = region.split_off(dest);
        fragments.extend(region, ());
        fragments.push(inserted, ());
        fragments.extend(after_inserted, ());
        if let Some(fragment) = right_fragment {
            fragments.push(fragment, ());
        }
        fragments.append(cursor.suffix(), ());
        drop(cursor);
        self.fragments = fragments;
        self.insertions.insert(
            my_id,
            Insertion {
                chars,
                origin_left,
                origin_right,
                splits: BTreeMap::from([(0, locator)]),
            },
        );
    }

    /// The fragment that an insertion of `id` at `pos` simply continues, if
    /// any: it ends exactly at `pos`, nothing concurrent follows it, and
    /// `continues_insertion` holds. Found without modifying the tree.
    fn typing_target(&self, pos: usize, id: CharId) -> Option<(Locator, CharId)> {
        if pos == 0 {
            return None;
        }
        let mut cursor = self.fragments.cursor::<PrepareLen>(());
        cursor.seek(&PrepareLen(pos), SeekBias::Left);
        let fragment = cursor.item()?;
        if cursor.start().0 + fragment.len() != pos {
            return None;
        }
        cursor.next();
        let origin_right = match cursor.item() {
            Some(next) if next.prepare == NOT_INSERTED => return None,
            Some(next) => Some(next.first_char()),
            None => None,
        };
        self.continues_insertion(fragment, id, origin_right)
            .then(|| (fragment.locator.clone(), fragment.insertion))
    }

    /// Whether a new insertion `id` right after `fragment` is just more typing
    /// of the same insertion: the next id of the same actor, with the same
    /// right origin. Per-character semantics are then identical to extending
    /// the insertion, which keeps sequential typing in one fragment.
    fn continues_insertion(&self, fragment: &Fragment, id: CharId, origin_right: Option<CharId>) -> bool {
        let insertion = &self.insertions[&fragment.insertion];
        let insertion_len = run_len(insertion.chars.len());
        fragment.offset + fragment.len == insertion_len
            && fragment.prepare == VISIBLE
            && !fragment.is_deleted()
            && insertion.origin_right == origin_right
            && fragment.insertion.offset(insertion_len) == id
    }

    /// Inserts without rebuilding the tree when nothing concurrent sits at
    /// the insertion point and the fragment's leaf has room.
    fn insert_in_place(&mut self, event: &TextEvent, pos: usize, content: &str) -> bool {
        if pos == 0 {
            return false;
        }
        let (fragment, keep, origin_right, next) = {
            let mut cursor = self.fragments.cursor::<PrepareLen>(());
            cursor.seek(&PrepareLen(pos), SeekBias::Left);
            let Some(fragment) = cursor.item() else {
                return false;
            };
            let keep = run_len(pos - cursor.start().0);
            let fragment = fragment.clone();
            cursor.next();
            let following = cursor.item();
            let origin_right = if keep < fragment.len {
                Some(fragment.first_char().offset(keep))
            } else {
                match following {
                    Some(next) if next.prepare == NOT_INSERTED => return false,
                    Some(next) => Some(next.first_char()),
                    None => None,
                }
            };
            let next = following.map_or_else(Locator::max, |next| next.locator.clone());
            (fragment, keep, origin_right, next)
        };

        let my_id = CharId::Delta(event.span.first());
        let chars: Vec<char> = content.chars().collect();
        let origin_left = fragment.first_char().offset(keep - 1);
        let target = fragment.locator.clone();
        let insertions = &mut self.insertions;
        let mut inserted_locator = None;
        let spliced = self.fragments.splice_item(
            |summary| summary.max_locator >= target,
            3,
            |mut left| {
                let right = (keep < left.len).then(|| split(insertions, &mut left, keep, &next));
                let upper = right.as_ref().map_or(&next, |right| &right.locator);
                let locator = Locator::between(&left.locator, upper);
                inserted_locator = Some(locator.clone());
                let inserted = Fragment {
                    locator,
                    insertion: my_id,
                    offset: 0,
                    len: run_len(chars.len()),
                    prepare: VISIBLE,
                    deleted_by: SmallVec::new(),
                };
                [Some(left), Some(inserted), right].into_iter().flatten()
            },
            (),
        );
        let Some(locator) = inserted_locator.filter(|_| spliced) else {
            return false;
        };
        self.insertions.insert(
            my_id,
            Insertion {
                chars,
                origin_left: Some(origin_left),
                origin_right,
                splits: BTreeMap::from([(0, locator)]),
            },
        );
        true
    }

    /// Deletes without rebuilding the tree when the range lies inside one
    /// visible fragment and the fragment's leaf has room for the split.
    fn delete_in_place(&mut self, event: &TextEvent, pos: usize, len: usize) -> bool {
        let (fragment, before, next) = {
            let mut cursor = self.fragments.cursor::<PrepareLen>(());
            cursor.seek(&PrepareLen(pos), SeekBias::Right);
            let Some(fragment) = cursor.item() else {
                return false;
            };
            let start = cursor.start().0;
            if fragment.prepare != VISIBLE || start + fragment.len() < pos + len {
                return false;
            }
            let fragment = fragment.clone();
            cursor.next();
            let next = cursor.item().map_or_else(Locator::max, |next| next.locator.clone());
            (fragment, run_len(pos - start), next)
        };

        let count = run_len(len);
        let deleter = event.span.first();
        let target = fragment.locator.clone();
        let insertions = &mut self.insertions;
        let mut deleted_run = None;
        let spliced = self.fragments.splice_item(
            |summary| summary.max_locator >= target,
            3,
            |mut first| {
                let (left, mut middle) = if before > 0 {
                    let middle = split(insertions, &mut first, before, &next);
                    (Some(first), middle)
                } else {
                    (None, first)
                };
                let right = (middle.len > count).then(|| split(insertions, &mut middle, count, &next));
                middle.prepare += 1;
                middle.deleted_by.push(deleter);
                deleted_run = Some((middle.first_char(), count));
                [left, Some(middle), right].into_iter().flatten()
            },
            (),
        );
        match deleted_run.filter(|_| spliced) {
            Some(run) => {
                self.deletes.insert(deleter, vec![run]);
                true
            }
            None => false,
        }
    }

    fn delete(&mut self, event: &TextEvent, pos: usize, len: usize) {
        if self.delete_in_place(event, pos, len) {
            return;
        }
        let mut cursor = self.fragments.cursor::<PrepareLen>(());
        let mut fragments = cursor.slice(&PrepareLen(pos), SeekBias::Right);
        let mut pending: Option<Fragment> = None;
        if let Some(fragment) = cursor.item()
            && cursor.start().0 < pos
        {
            let mut fragment = fragment.clone();
            let keep = run_len(pos - cursor.start().0);
            cursor.next();
            let next = cursor.item().map_or_else(Locator::max, |next| next.locator.clone());
            pending = Some(split(&mut self.insertions, &mut fragment, keep, &next));
            fragments.push(fragment, ());
        }

        let mut targets = Vec::new();
        let mut deleted: u32 = 0;
        let mut remaining = run_len(len);
        while remaining > 0 {
            let mut fragment = match pending.take() {
                Some(fragment) => fragment,
                None => {
                    let fragment = cursor.item().expect("delete range is in bounds").clone();
                    cursor.next();
                    fragment
                }
            };
            if fragment.prepare != VISIBLE {
                fragments.push(fragment, ());
                continue;
            }
            if fragment.len > remaining {
                let next = cursor.item().map_or_else(Locator::max, |next| next.locator.clone());
                pending = Some(split(&mut self.insertions, &mut fragment, remaining, &next));
            }
            fragment.prepare += 1;
            fragment
                .deleted_by
                .push(DeltaId::new(event.span.actor, event.span.start.offset(u64::from(deleted))));
            targets.push((fragment.first_char(), fragment.len));
            deleted += fragment.len;
            remaining -= fragment.len;
            fragments.push(fragment, ());
        }
        if let Some(fragment) = pending {
            fragments.push(fragment, ());
        }
        fragments.append(cursor.suffix(), ());
        drop(cursor);
        self.fragments = fragments;
        self.deletes.insert(event.span.first(), targets);
    }
}

fn run_len(len: usize) -> u32 {
    u32::try_from(len).expect("runs are shorter than 4 GiB of chars")
}

fn place(before: usize, offset: usize, target_visible: bool, bias: Bias) -> ResolvedAnchor {
    let pos = match bias {
        _ if !target_visible => before,
        Bias::Left => before + offset + 1,
        Bias::Right => before + offset,
    };
    ResolvedAnchor {
        pos,
        outdated: !target_visible,
    }
}

/// Splits `fragment` so it keeps its first `keep` chars, returning the rest
/// with a locator between the fragment's and `next`.
fn split(
    insertions: &mut BTreeMap<CharId, Insertion>,
    fragment: &mut Fragment,
    keep: u32,
    next: &Locator,
) -> Fragment {
    debug_assert!(keep > 0 && keep < fragment.len);
    let locator = Locator::between(&fragment.locator, next);
    let right = Fragment {
        locator: locator.clone(),
        insertion: fragment.insertion,
        offset: fragment.offset + keep,
        len: fragment.len - keep,
        prepare: fragment.prepare,
        deleted_by: fragment
            .deleted_by
            .iter()
            .map(|id| DeltaId::new(id.actor, id.lamport.offset(u64::from(keep))))
            .collect(),
    };
    fragment.len = keep;
    insertions
        .get_mut(&fragment.insertion)
        .expect("fragment's insertion exists")
        .splits
        .insert(right.offset, locator);
    right
}

/// The insertion containing `id`, and `id`'s offset within it.
fn insertion_of(insertions: &BTreeMap<CharId, Insertion>, id: CharId) -> Option<(CharId, u32)> {
    let (key, insertion) = insertions.range(..=id).next_back()?;
    let offset = id.distance_from(*key)?;
    ((offset as usize) < insertion.chars.len()).then_some((*key, offset))
}

/// The locator of the fragment containing `id`, and `id`'s offset within it.
fn locate(insertions: &BTreeMap<CharId, Insertion>, id: CharId) -> Option<(Locator, u32)> {
    let (key, offset) = insertion_of(insertions, id)?;
    let (start, locator) = insertions[&key].splits.range(..=offset).next_back()?;
    Some((locator.clone(), offset - start))
}

fn compare_positions(insertions: &BTreeMap<CharId, Insertion>, a: CharId, b: CharId) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }
    let a = locate(insertions, a).expect("origin is in the document");
    let b = locate(insertions, b).expect("origin is in the document");
    a.cmp(&b)
}

/// Where a new insertion goes among `region`, the fragments between its
/// origins, following the YjsMod rules. Only the first character of a
/// fragment can have an origin outside the fragment, so the decision is made
/// per fragment.
fn integration_point(
    insertions: &BTreeMap<CharId, Insertion>,
    region: &[Fragment],
    my_id: CharId,
    my_left: Option<CharId>,
    my_right: Option<CharId>,
) -> usize {
    let left_order = |other: Option<CharId>| match (other, my_left) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(other), Some(mine)) => compare_positions(insertions, other, mine),
    };
    let right_order = |other: Option<CharId>| match (other, my_right) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(other), Some(mine)) => compare_positions(insertions, other, mine),
    };

    let mut scan_start = 0;
    let mut scanning = false;
    let mut index = 0;
    while index < region.len() {
        let other = &region[index];
        let insertion = &insertions[&other.insertion];
        let other_left = if other.offset == 0 {
            insertion.origin_left
        } else {
            Some(other.insertion.offset(other.offset - 1))
        };
        match left_order(other_left) {
            Ordering::Less => break,
            Ordering::Greater => {}
            Ordering::Equal => {
                if insertion.origin_right == my_right {
                    if my_id < other.first_char() {
                        break;
                    }
                    scanning = false;
                } else if right_order(insertion.origin_right) == Ordering::Less {
                    if !scanning {
                        scanning = true;
                        scan_start = index;
                    }
                } else {
                    scanning = false;
                }
            }
        }
        index += 1;
    }
    if scanning { scan_start } else { index }
}
