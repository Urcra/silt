use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BinaryHeap, HashSet};

use silt_types::{ActorId, DeltaId, DeltaSpan, Frontier, Lamport};

/// The highest lamport each actor has contributed to a version.
///
/// Because every actor's deltas form a chain (each one has the previous as an
/// ancestor), this exactly describes the set of ancestors of a version.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VersionVector(BTreeMap<ActorId, Lamport>);

impl VersionVector {
    pub fn contains(&self, id: DeltaId) -> bool {
        self.0.get(&id.actor).is_some_and(|max| id.lamport <= *max)
    }

    pub fn get(&self, actor: ActorId) -> Option<Lamport> {
        self.0.get(&actor).copied()
    }

    /// The common ancestors of two versions, such as a merge base.
    pub fn intersection(&self, other: &Self) -> Self {
        Self(
            self.0
                .iter()
                .filter_map(|(actor, max)| other.get(*actor).map(|other| (*actor, (*max).min(other))))
                .collect(),
        )
    }

    fn include(&mut self, id: DeltaId) {
        let max = self.0.entry(id.actor).or_insert(id.lamport);
        if *max < id.lamport {
            *max = id.lamport;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CausalError {
    #[error("delta span is empty")]
    EmptySpan,
    #[error("parent {0} is not in the graph")]
    UnknownParent(DeltaId),
    #[error("{id} does not come after its parents on the lamport clock")]
    LamportNotAfterParents { id: DeltaId },
    #[error("{id} does not descend from {previous}, the actor's previous delta")]
    BrokenActorChain { id: DeltaId, previous: DeltaId },
    #[error("{id} overlaps deltas already in the graph with different history")]
    ConflictingSpan { id: DeltaId },
    #[error("{id} claims {len} ids, more than a delta may have")]
    SpanTooLong { id: DeltaId, len: u64 },
    #[error("{id} lies outside the lamport clock or too far ahead of it")]
    LamportOutOfRange { id: DeltaId },
}

/// Most ids one delta may claim; more than any text edit Silt makes.
pub const MAX_SPAN_LEN: u64 = 1 << 32;
/// How far ahead of everything known a delta may start. Deltas from threads
/// this replica can't see push the clock ahead legitimately, but a peer must
/// not be able to use the clock up.
pub const MAX_CLOCK_SKEW: u64 = 1 << 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Added {
    New(DeltaSpan),
    AlreadyKnown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub span: DeltaSpan,
    pub parents: Frontier,
}

/// Every delta's identity and parents, stored as run-length-encoded runs in a
/// topological order.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CausalGraph {
    runs: Vec<Run>,
    by_actor: BTreeMap<ActorId, Vec<usize>>,
    frontier: Frontier,
    max_lamport: Option<Lamport>,
}

impl CausalGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn frontier(&self) -> &Frontier {
        &self.frontier
    }

    /// Runs in an order where every delta comes after its ancestors.
    pub fn runs(&self) -> &[Run] {
        &self.runs
    }

    /// The lamport a new local delta should start at.
    pub fn next_lamport(&self) -> Lamport {
        self.max_lamport.map_or(Lamport(0), Lamport::next)
    }

    pub fn contains(&self, id: DeltaId) -> bool {
        self.find(id).is_some()
    }

    pub fn parents_of(&self, id: DeltaId) -> Option<Frontier> {
        let (run_index, offset) = self.find(id)?;
        let run = &self.runs[run_index];
        Some(if offset == 0 {
            run.parents.clone()
        } else {
            Frontier::single(run.span.id_at(offset - 1))
        })
    }

    /// Records a span of deltas whose first id has `parents`; each later id in
    /// the span has the id before it as its only parent.
    pub fn add(&mut self, span: DeltaSpan, parents: &Frontier) -> Result<Added, CausalError> {
        if span.len == 0 {
            return Err(CausalError::EmptySpan);
        }
        if span.len > MAX_SPAN_LEN {
            return Err(CausalError::SpanTooLong {
                id: span.first(),
                len: span.len,
            });
        }
        if !span.is_in_range() || span.start.0 > self.next_lamport().0.saturating_add(MAX_CLOCK_SKEW) {
            return Err(CausalError::LamportOutOfRange { id: span.first() });
        }
        let Some((span, parents)) = self.unknown_suffix(span, parents)? else {
            return Ok(Added::AlreadyKnown);
        };
        self.validate(span, &parents)?;
        self.append(span, parents);
        Ok(Added::New(span))
    }

    pub fn version_vector(&self, frontier: &Frontier) -> VersionVector {
        let mut vv = VersionVector::default();
        let mut visited = vec![false; self.runs.len()];
        let mut stack: Vec<DeltaId> = frontier.ids().to_vec();
        while let Some(id) = stack.pop() {
            let Some((run_index, _)) = self.find(id) else {
                continue;
            };
            vv.include(id);
            if !visited[run_index] {
                visited[run_index] = true;
                stack.extend_from_slice(self.runs[run_index].parents.ids());
            }
        }
        vv
    }

    /// Whether `id` is `frontier` itself or one of its ancestors. Lamports
    /// grow along every path, so only deltas at least as new as `id` are
    /// visited, which keeps checks against recent ids cheap.
    pub fn is_ancestor(&self, id: DeltaId, frontier: &Frontier) -> bool {
        let mut visited = HashSet::new();
        let mut stack: Vec<DeltaId> = frontier.ids().to_vec();
        while let Some(at) = stack.pop() {
            if at.lamport < id.lamport {
                continue;
            }
            let Some((run_index, _)) = self.find(at) else {
                continue;
            };
            let span = self.runs[run_index].span;
            if span.actor == id.actor && span.start <= id.lamport && id.lamport <= at.lamport {
                return true;
            }
            if visited.insert(run_index) {
                stack.extend(self.runs[run_index].parents.ids());
            }
        }
        false
    }

    /// Spans that are in `to` but not in `from`, in topological order.
    pub fn diff(&self, from: &VersionVector, to: &VersionVector) -> Vec<DeltaSpan> {
        let mut spans = Vec::new();
        for run in &self.runs {
            let actor = run.span.actor;
            let Some(upper) = to.get(actor) else {
                continue;
            };
            let start = match from.get(actor) {
                Some(lower) => run.span.start.max(lower.next()),
                None => run.span.start,
            };
            let end = run.span.end().min(upper.next());
            if start < end {
                spans.push(DeltaSpan::new(actor, start, end.0 - start.0));
            }
        }
        spans
    }

    /// Spans in `a` but not `b`, and in `b` but not `a`. Walks back from
    /// both versions only until what is left is history they share, so it
    /// costs what they differ by, not the whole history.
    pub fn diff_frontiers(&self, a: &Frontier, b: &Frontier) -> (Vec<DeltaSpan>, Vec<DeltaSpan>) {
        const IN_A: u8 = 1;
        const IN_B: u8 = 2;
        const SHARED: u8 = IN_A | IN_B;
        let mut queue: BinaryHeap<(Lamport, ActorId, u8)> = BinaryHeap::new();
        queue.extend(a.ids().iter().map(|id| (id.lamport, id.actor, IN_A)));
        queue.extend(b.ids().iter().map(|id| (id.lamport, id.actor, IN_B)));
        let (mut only_a, mut only_b) = (Vec::new(), Vec::new());
        while queue.iter().any(|(_, _, side)| *side != SHARED) {
            let Some((lamport, actor, mut side)) = queue.pop() else {
                break;
            };
            while let Some(&(next, next_actor, next_side)) = queue.peek()
                && (next, next_actor) == (lamport, actor)
            {
                side |= next_side;
                queue.pop();
            }
            let Some((run_index, _)) = self.find(DeltaId::new(actor, lamport)) else {
                continue;
            };
            let run = &self.runs[run_index];
            // Only down to the next queued id: a delta reached later may name
            // a parent inside this run, below which both sides then share.
            let lowest = match queue.peek() {
                Some(&(next, _, _)) => run.span.start.max(Lamport(next.0.saturating_add(1)).min(lamport)),
                None => run.span.start,
            };
            let span = DeltaSpan::new(actor, lowest, lamport.0 - lowest.0 + 1);
            match side {
                IN_A => only_a.push(span),
                IN_B => only_b.push(span),
                _ => {}
            }
            if lowest > run.span.start {
                queue.push((Lamport(lowest.0 - 1), actor, side));
            } else {
                queue.extend(run.parents.ids().iter().map(|parent| (parent.lamport, parent.actor, side)));
            }
        }
        (only_a, only_b)
    }

    /// Removes ids that are ancestors of other ids in the set.
    pub fn minimize(&self, ids: impl IntoIterator<Item = DeltaId>) -> Frontier {
        let ids: Frontier = ids.into_iter().collect();
        let dominated = |id: &DeltaId| {
            ids.ids()
                .iter()
                .filter(|other| *other != id)
                .any(|other| self.is_ancestor(*id, &Frontier::single(*other)))
        };
        ids.ids().iter().filter(|id| !dominated(id)).copied().collect()
    }

    fn find(&self, id: DeltaId) -> Option<(usize, u64)> {
        let runs = self.by_actor.get(&id.actor)?;
        let position = runs.partition_point(|index| self.runs[*index].span.start <= id.lamport);
        let run_index = *runs.get(position.checked_sub(1)?)?;
        let span = self.runs[run_index].span;
        span.contains(id).then(|| (run_index, id.lamport.0 - span.start.0))
    }

    /// Whether any id of `actor` in `[start, end)` is known. The actor's
    /// runs are in lamport order, so one search finds the only candidate.
    fn overlaps(&self, actor: ActorId, start: Lamport, end: Lamport) -> bool {
        let Some(runs) = self.by_actor.get(&actor) else {
            return false;
        };
        let before_end = runs.partition_point(|index| self.runs[*index].span.start < end);
        before_end
            .checked_sub(1)
            .is_some_and(|position| self.runs[runs[position]].span.end() > start)
    }

    /// The part of `span` not in the graph yet, with its parents. A span
    /// that starts at a known id must repeat that id's parents: the same id
    /// with a different history is a conflict, not a duplicate.
    fn unknown_suffix(
        &self,
        span: DeltaSpan,
        parents: &Frontier,
    ) -> Result<Option<(DeltaSpan, Frontier)>, CausalError> {
        let Some((run_index, _)) = self.find(span.first()) else {
            if self.overlaps(span.actor, span.start, span.end()) {
                return Err(CausalError::ConflictingSpan { id: span.first() });
            }
            return Ok(Some((span, parents.clone())));
        };
        if self.parents_of(span.first()).as_ref() != Some(parents) {
            return Err(CausalError::ConflictingSpan { id: span.first() });
        }
        let known_end = self.runs[run_index].span.end();
        if known_end >= span.end() {
            return Ok(None);
        }
        let remainder = DeltaSpan::new(span.actor, known_end, span.end().0 - known_end.0);
        if self.overlaps(remainder.actor, remainder.start, remainder.end()) {
            return Err(CausalError::ConflictingSpan { id: remainder.first() });
        }
        let previous = DeltaId::new(span.actor, Lamport(known_end.0 - 1));
        Ok(Some((remainder, Frontier::single(previous))))
    }

    fn validate(&self, span: DeltaSpan, parents: &Frontier) -> Result<(), CausalError> {
        for parent in parents.ids() {
            if !self.contains(*parent) {
                return Err(CausalError::UnknownParent(*parent));
            }
            if parent.lamport >= span.start {
                return Err(CausalError::LamportNotAfterParents { id: span.first() });
            }
        }
        if let Some(previous) = self.last_of_actor(span.actor) {
            let descends = previous.lamport < span.start && self.is_ancestor(previous, parents);
            if !descends {
                return Err(CausalError::BrokenActorChain {
                    id: span.first(),
                    previous,
                });
            }
        }
        Ok(())
    }

    fn last_of_actor(&self, actor: ActorId) -> Option<DeltaId> {
        let run_index = *self.by_actor.get(&actor)?.last()?;
        Some(self.runs[run_index].span.last())
    }

    fn append(&mut self, span: DeltaSpan, parents: Frontier) {
        for parent in parents.ids() {
            self.frontier.remove(parent);
        }
        self.frontier.insert(span.last());
        self.max_lamport = Some(self.max_lamport.map_or(span.last().lamport, |max| {
            max.max(span.last().lamport)
        }));

        let extends_previous = self.last_of_actor(span.actor).is_some_and(|previous| {
            previous.lamport.next() == span.start && parents == Frontier::single(previous)
        });
        if extends_previous {
            let run_index = *self
                .by_actor
                .get(&span.actor)
                .and_then(|runs| runs.last())
                .expect("actor with a previous delta has a run");
            self.runs[run_index].span.len += span.len;
            return;
        }
        self.by_actor
            .entry(span.actor)
            .or_default()
            .push(self.runs.len());
        self.runs.push(Run { span, parents });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(actor: u128, lamport: u64) -> DeltaId {
        DeltaId::new(ActorId(actor), Lamport(lamport))
    }

    fn span(actor: u128, start: u64, len: u64) -> DeltaSpan {
        DeltaSpan::new(ActorId(actor), Lamport(start), len)
    }

    fn frontier(ids: &[DeltaId]) -> Frontier {
        ids.iter().copied().collect()
    }

    /// base (a:0..6) then concurrent a:6..9 and b:6..7, then a merge by a.
    fn diamond() -> CausalGraph {
        let mut graph = CausalGraph::new();
        graph.add(span(1, 0, 6), &Frontier::root()).expect("base");
        graph.add(span(1, 6, 3), &frontier(&[id(1, 5)])).expect("a");
        graph.add(span(2, 6, 1), &frontier(&[id(1, 5)])).expect("b");
        graph
            .add(span(1, 9, 1), &frontier(&[id(1, 8), id(2, 6)]))
            .expect("merge");
        graph
    }

    #[test]
    fn contiguous_local_typing_extends_one_run() {
        let mut graph = CausalGraph::new();
        graph.add(span(1, 0, 3), &Frontier::root()).expect("first");
        graph.add(span(1, 3, 2), &frontier(&[id(1, 2)])).expect("second");
        assert_eq!(graph.runs().len(), 1);
        assert_eq!(graph.runs()[0].span, span(1, 0, 5));
        assert_eq!(graph.frontier(), &frontier(&[id(1, 4)]));
        assert_eq!(graph.next_lamport(), Lamport(5));
    }

    #[test]
    fn frontier_tracks_concurrent_heads_and_merges() {
        let mut graph = CausalGraph::new();
        graph.add(span(1, 0, 6), &Frontier::root()).expect("base");
        graph.add(span(1, 6, 3), &frontier(&[id(1, 5)])).expect("a");
        graph.add(span(2, 6, 1), &frontier(&[id(1, 5)])).expect("b");
        assert_eq!(graph.frontier(), &frontier(&[id(1, 8), id(2, 6)]));
        graph
            .add(span(1, 9, 1), &frontier(&[id(1, 8), id(2, 6)]))
            .expect("merge");
        assert_eq!(graph.frontier(), &frontier(&[id(1, 9)]));
    }

    #[test]
    fn parents_within_and_at_start_of_run() {
        let graph = diamond();
        assert_eq!(graph.parents_of(id(1, 0)), Some(Frontier::root()));
        assert_eq!(graph.parents_of(id(1, 7)), Some(frontier(&[id(1, 6)])));
        assert_eq!(graph.parents_of(id(2, 6)), Some(frontier(&[id(1, 5)])));
        assert_eq!(graph.parents_of(id(3, 0)), None);
    }

    #[test]
    fn intersection_is_the_common_history() {
        let mut graph = CausalGraph::new();
        graph.add(span(1, 0, 3), &Frontier::root()).expect("base");
        graph.add(span(2, 3, 2), &frontier(&[id(1, 2)])).expect("one side");
        graph.add(span(3, 3, 4), &frontier(&[id(1, 1)])).expect("other side");
        let common = graph
            .version_vector(&frontier(&[id(2, 4)]))
            .intersection(&graph.version_vector(&frontier(&[id(3, 6)])));
        assert!(common.contains(id(1, 1)));
        assert!(!common.contains(id(1, 2)));
        assert!(!common.contains(id(2, 3)) && !common.contains(id(3, 3)));
    }

    #[test]
    fn version_vector_describes_ancestors() {
        let graph = diamond();
        let vv = graph.version_vector(&frontier(&[id(2, 6)]));
        assert!(vv.contains(id(1, 5)));
        assert!(vv.contains(id(2, 6)));
        assert!(!vv.contains(id(1, 6)));
        assert!(graph.is_ancestor(id(1, 3), &frontier(&[id(1, 8)])));
        assert!(!graph.is_ancestor(id(2, 6), &frontier(&[id(1, 8)])));
        assert!(graph.is_ancestor(id(2, 6), &frontier(&[id(1, 9)])));
    }

    #[test]
    fn version_vector_follows_parents_of_earlier_runs_of_the_same_actor() {
        let mut graph = CausalGraph::new();
        graph.add(span(1, 0, 2), &Frontier::root()).expect("a");
        graph.add(span(2, 2, 1), &frontier(&[id(1, 1)])).expect("b after a");
        graph.add(span(3, 0, 1), &Frontier::root()).expect("c");
        graph
            .add(span(2, 3, 1), &frontier(&[id(2, 2), id(3, 0)]))
            .expect("b merges c");
        let vv = graph.version_vector(&frontier(&[id(2, 3)]));
        assert!(vv.contains(id(1, 1)), "reached only through b's earlier run");
        assert!(vv.contains(id(3, 0)));
    }

    #[test]
    fn diff_lists_missing_spans_in_topological_order() {
        let graph = diamond();
        let from = graph.version_vector(&frontier(&[id(2, 6)]));
        let to = graph.version_vector(graph.frontier());
        assert_eq!(graph.diff(&from, &to), vec![span(1, 6, 3), span(1, 9, 1)]);
        let base = graph.version_vector(&frontier(&[id(1, 5)]));
        let a_side = graph.version_vector(&frontier(&[id(1, 8)]));
        assert_eq!(graph.diff(&base, &a_side), vec![span(1, 6, 3)]);
    }

    fn ids_of(spans: &[DeltaSpan]) -> HashSet<DeltaId> {
        spans
            .iter()
            .flat_map(|span| (span.start.0..span.end().0).map(|lamport| id(span.actor.0, lamport)))
            .collect()
    }

    #[test]
    fn diff_frontiers_agrees_with_version_vectors() {
        let mut graph = diamond();
        graph.add(span(3, 7, 2), &frontier(&[id(1, 6), id(2, 6)])).expect("c");
        graph.add(span(2, 10, 1), &frontier(&[id(1, 9), id(3, 8)])).expect("b merges");
        graph.add(span(4, 7, 1), &frontier(&[id(1, 3)])).expect("d from inside the base run");
        let versions = [
            Frontier::root(),
            frontier(&[id(1, 3)]),
            frontier(&[id(1, 7)]),
            frontier(&[id(2, 6)]),
            frontier(&[id(1, 8), id(2, 6)]),
            frontier(&[id(3, 8)]),
            frontier(&[id(4, 7)]),
            frontier(&[id(2, 10), id(4, 7)]),
            graph.frontier().clone(),
        ];
        for a in &versions {
            for b in &versions {
                let (only_a, only_b) = graph.diff_frontiers(a, b);
                let (vv_a, vv_b) = (graph.version_vector(a), graph.version_vector(b));
                assert_eq!(ids_of(&only_a), ids_of(&graph.diff(&vv_b, &vv_a)), "{a:?} minus {b:?}");
                assert_eq!(ids_of(&only_b), ids_of(&graph.diff(&vv_a, &vv_b)), "{b:?} minus {a:?}");
            }
        }
    }

    #[test]
    fn readding_known_deltas_is_idempotent() {
        let mut graph = diamond();
        assert_eq!(
            graph.add(span(1, 6, 3), &frontier(&[id(1, 5)])),
            Ok(Added::AlreadyKnown)
        );
        assert_eq!(graph.add(span(1, 2, 2), &frontier(&[id(1, 1)])), Ok(Added::AlreadyKnown));
    }

    #[test]
    fn partially_known_span_adds_only_the_new_suffix() {
        let mut graph = CausalGraph::new();
        graph.add(span(1, 0, 3), &Frontier::root()).expect("first");
        assert_eq!(
            graph.add(span(1, 0, 5), &Frontier::root()),
            Ok(Added::New(span(1, 3, 2)))
        );
        assert_eq!(graph.frontier(), &frontier(&[id(1, 4)]));
    }

    #[test]
    fn rejects_invalid_history() {
        let mut graph = diamond();
        assert_eq!(
            graph.add(span(3, 20, 1), &frontier(&[id(4, 1)])),
            Err(CausalError::UnknownParent(id(4, 1)))
        );
        assert_eq!(
            graph.add(span(3, 5, 1), &frontier(&[id(1, 5)])),
            Err(CausalError::LamportNotAfterParents { id: id(3, 5) })
        );
        assert_eq!(
            graph.add(span(2, 20, 1), &frontier(&[id(1, 8)])),
            Err(CausalError::BrokenActorChain {
                id: id(2, 20),
                previous: id(2, 6)
            })
        );
        assert_eq!(graph.add(span(1, 0, 0), &Frontier::root()), Err(CausalError::EmptySpan));
    }

    #[test]
    fn minimize_drops_ancestors() {
        let graph = diamond();
        assert_eq!(
            graph.minimize([id(1, 3), id(1, 8), id(2, 6)]),
            frontier(&[id(1, 8), id(2, 6)])
        );
        assert_eq!(graph.minimize([id(1, 8), id(2, 6), id(1, 9)]), frontier(&[id(1, 9)]));
    }
}
