//! The server's room refuses deltas no replica could apply, and overlaps
//! with stored ones (ADR 0016).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use silt_crdt::{InodeKind, Parent};
use silt_log::{Author, Content, ConvOp, Delta, Family, Op};
use silt_sync::{PushError, Room};
use silt_types::{ActorId, DeltaSpan, Lamport, ThreadId};

const THREAD: ThreadId = ThreadId(1);

fn title(family: &mut Family, actor: ActorId, text: &str) -> Delta {
    let delta = family.delta(actor, THREAD, Author::User, 0, family.thread_head(THREAD), Op::Conv(ConvOp::Title(text.into())));
    family.apply(delta.clone(), &mut HashMap::new()).expect("applies");
    delta
}

#[test]
fn malformed_deltas_are_refused_quickly() {
    let mut family = Family::new();
    let mut room = Room::new(1);
    room.push(vec![title(&mut family, ActorId(1), "a")]).expect("pushes");

    let mut huge = family.delta(ActorId(2), THREAD, Author::User, 0, family.thread_head(THREAD), Op::Conv(ConvOp::Title("x".into())));
    huge.span.len = 1 << 62;
    let started = Instant::now();
    assert!(matches!(room.push(vec![huge]), Err(PushError::Invalid { .. })));
    assert!(started.elapsed() < Duration::from_secs(1));

    let mut late = family.delta(ActorId(2), THREAD, Author::User, 0, family.thread_head(THREAD), Op::Conv(ConvOp::Title("x".into())));
    late.span = DeltaSpan::new(ActorId(2), Lamport(u64::MAX), 1);
    assert!(matches!(room.push(vec![late]), Err(PushError::Invalid { .. })));

    let git = family.delta(
        ActorId(2),
        THREAD,
        Author::User,
        0,
        family.thread_head(THREAD),
        Op::Create {
            kind: InodeKind::Dir,
            parent: Parent::Root,
            name: ".git".into(),
            content: Content::Empty,
            executable: false,
        },
    );
    assert!(matches!(room.push(vec![git]), Err(PushError::Invalid { .. })));
    assert_eq!(room.seq(), 1, "nothing refused was stored");
}

#[test]
fn a_batch_refused_or_not_stored_leaves_the_room_as_it_was() {
    let mut family = Family::new();
    let mut room = Room::new(1);
    let first = title(&mut family, ActorId(1), "a");
    let second = title(&mut family, ActorId(1), "b");
    let mut broken = title(&mut family, ActorId(2), "c");
    broken.span.len = 1 << 62;
    assert!(room.push(vec![first.clone(), second.clone(), broken]).is_err());
    let abandoned = room.prepare(vec![first.clone()]).expect("prepares");
    assert_eq!(abandoned.fresh(), std::slice::from_ref(&first));
    drop(abandoned);
    assert!(!room.graph().contains(first.span.first()), "an abandoned batch is taken back");
    assert_eq!(room.push(vec![first, second]).expect("pushes again").accepted, 2);
}

#[test]
fn deltas_overlapping_stored_ones_are_conflicts() {
    let mut family = Family::new();
    let mut room = Room::new(1);
    let first = title(&mut family, ActorId(1), "a");
    room.push(vec![first.clone()]).expect("pushes");
    let mut overlapping = first.clone();
    overlapping.op = Op::Conv(ConvOp::Title("b".into()));
    assert!(matches!(room.push(vec![overlapping]), Err(PushError::Conflict(_))));
    let mut mid = first;
    mid.span = DeltaSpan::new(ActorId(1), Lamport(0), 1);
    mid.parents = silt_types::Frontier::single(silt_types::DeltaId::new(ActorId(1), Lamport(0)));
    assert!(room.push(vec![mid]).is_err());
    assert_eq!(room.seq(), 1);
}
