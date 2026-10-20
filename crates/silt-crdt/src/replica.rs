use silt_types::{ActorId, DeltaId, DeltaSpan, Frontier};

use crate::causal::{Added, CausalError, CausalGraph};
use crate::text::{TextDoc, TextError, TextEvent, TextOp};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReplicaError {
    #[error(transparent)]
    Causal(#[from] CausalError),
    #[error(transparent)]
    Text(#[from] TextError),
}

/// A replica of a single text document: its own causal graph, the document
/// state and every event it knows, in the order it applied them.
#[derive(Debug, Clone)]
pub struct TextReplica {
    actor: ActorId,
    graph: CausalGraph,
    doc: TextDoc,
    events: Vec<TextEvent>,
}

impl TextReplica {
    pub fn new(actor: ActorId) -> Self {
        Self {
            actor,
            graph: CausalGraph::new(),
            doc: TextDoc::new(),
            events: Vec::new(),
        }
    }

    /// A replica whose document starts as the git blob `content`, introduced
    /// by the delta `source` (for example the import of a commit). Every
    /// replica built from the same `source` and `content` agrees on the ids of
    /// the base characters.
    pub fn from_base(actor: ActorId, source: DeltaId, content: &str) -> Result<Self, ReplicaError> {
        let mut graph = CausalGraph::new();
        graph.add(DeltaSpan::new(source.actor, source.lamport, 1), &Frontier::root())?;
        Ok(Self {
            actor,
            graph,
            doc: TextDoc::from_base(source, content),
            events: Vec::new(),
        })
    }

    pub fn actor(&self) -> ActorId {
        self.actor
    }

    pub fn text(&self) -> String {
        self.doc.text()
    }

    pub fn graph(&self) -> &CausalGraph {
        &self.graph
    }

    pub fn doc(&self) -> &TextDoc {
        &self.doc
    }

    pub fn frontier(&self) -> &Frontier {
        self.graph.frontier()
    }

    /// Events in an order where each comes after the events it depends on.
    pub fn events(&self) -> &[TextEvent] {
        &self.events
    }

    pub fn insert(&mut self, pos: usize, content: &str) -> Result<TextEvent, ReplicaError> {
        self.local(TextOp::Insert {
            pos,
            content: content.to_owned(),
        })
    }

    pub fn delete(&mut self, pos: usize, len: usize) -> Result<TextEvent, ReplicaError> {
        self.local(TextOp::Delete { pos, len })
    }

    /// Applies an event from another replica. Re-delivering a known event is a
    /// no-op; an event whose parents are unknown is rejected untouched.
    pub fn receive(&mut self, event: &TextEvent) -> Result<(), ReplicaError> {
        if self.graph.contains(event.span.first()) {
            return Ok(());
        }
        if let Some(parent) = event.parents.ids().iter().find(|id| !self.graph.contains(**id)) {
            return Err(CausalError::UnknownParent(*parent).into());
        }
        self.doc.check(&self.graph, event)?;
        match self.graph.add(event.span, &event.parents)? {
            Added::New(_) => {
                self.doc.apply(&self.graph, event)?;
                self.events.push(event.clone());
            }
            Added::AlreadyKnown => {}
        }
        Ok(())
    }

    fn local(&mut self, op: TextOp) -> Result<TextEvent, ReplicaError> {
        let span = DeltaSpan::new(self.actor, self.graph.next_lamport(), op.units() as u64);
        let event = TextEvent {
            span,
            parents: self.graph.frontier().clone(),
            op,
        };
        self.receive(&event)?;
        Ok(event)
    }
}
