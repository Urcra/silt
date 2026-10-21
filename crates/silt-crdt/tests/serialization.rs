use silt_crdt::{CausalGraph, FileTree, InodeKind, Parent, TextDoc, TextEvent, TextReplica, TreeEvent, TreeOp};
use silt_types::{ActorId, DeltaId, Lamport};

fn round_trip<T: serde::Serialize + serde::de::DeserializeOwned>(value: &T) -> T {
    let bytes = postcard::to_allocvec(value).expect("serializes");
    postcard::from_bytes(&bytes).expect("deserializes")
}

fn edited_replicas() -> (TextReplica, TextReplica) {
    let mut a = TextReplica::new(ActorId(1));
    a.insert(0, "héllo wörld").expect("insert");
    let mut b = TextReplica::new(ActorId(2));
    for event in a.events() {
        b.receive(event).expect("sync");
    }
    a.delete(0, 1).expect("delete");
    b.insert(5, ",").expect("insert");
    (a, b)
}

#[test]
fn events_survive_the_wire() {
    let (a, b) = edited_replicas();
    let mut copy = TextReplica::new(ActorId(3));
    for event in a.events().iter().chain(b.events()) {
        let event: TextEvent = round_trip(event);
        copy.receive(&event).expect("applies");
    }
    let mut merged = a.clone();
    for event in b.events() {
        merged.receive(event).expect("applies");
    }
    assert_eq!(copy.text(), merged.text());
}

#[test]
fn snapshots_resume_where_they_left_off() {
    let (a, b) = edited_replicas();
    let graph: CausalGraph = round_trip(a.graph());
    let doc: TextDoc = round_trip(a.doc());
    assert_eq!(doc.text(), a.text());
    assert_eq!(graph.frontier(), a.graph().frontier());

    let mut graph = graph;
    let mut doc = doc;
    for event in b.events() {
        if graph.contains(event.span.first()) {
            continue;
        }
        graph.add(event.span, &event.parents).expect("causal");
        doc.apply(&graph, event).expect("applies");
    }
    let mut merged = a.clone();
    for event in b.events() {
        merged.receive(event).expect("applies");
    }
    assert_eq!(doc.text(), merged.text());
}

#[test]
fn tree_events_and_snapshots_round_trip() {
    let id = |lamport| DeltaId::new(ActorId(1), Lamport(lamport));
    let events = [
        TreeEvent { id: id(0), op: TreeOp::Create { kind: InodeKind::Dir, parent: Parent::Root, name: "src".into() } },
        TreeEvent {
            id: id(1),
            op: TreeOp::Create { kind: InodeKind::File, parent: Parent::Dir(silt_crdt::InodeId(id(0))), name: "lib.rs".into() },
        },
    ];
    let mut tree = FileTree::new();
    for event in &events {
        tree.apply(&round_trip(event)).expect("applies");
    }
    let restored: FileTree = round_trip(&tree);
    assert_eq!(restored.paths(), tree.paths());
}
