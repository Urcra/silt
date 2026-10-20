//! Deterministic network simulation (ADR 0009): clients edit a shared file
//! while requests are reordered, responses are lost and the server is rolled
//! back to older snapshots. Once the network is quiet, every client must
//! agree and the server must hold every delta.

use std::collections::HashMap;

use proptest::prelude::*;
use silt_crdt::{InodeId, InodeKind, Parent, TextOp};
use silt_log::{Applied, Author, Content, Delta, Family, FileView, Op};
use silt_sync::{ClientSync, Effect, Outcome, Pull, Room};
use silt_types::{ActorId, ThreadId};

const THREAD: ThreadId = ThreadId(1);

struct Client {
    actor: ActorId,
    family: Family,
    log: Vec<Delta>,
    sync: ClientSync,
}

impl Client {
    fn new(index: usize) -> Self {
        Self {
            actor: ActorId(index as u128 + 1),
            family: Family::new(),
            log: Vec::new(),
            sync: ClientSync::default(),
        }
    }

    fn write(&mut self, op: Op) {
        let delta = self.family.delta(self.actor, THREAD, Author::User, 0, self.family.thread_head(THREAD), op);
        self.family.apply(delta.clone(), &mut HashMap::new()).expect("local write applies");
        self.log.push(delta);
    }

    fn file(&self) -> Option<InodeId> {
        self.family.tree().paths().get("a.txt").copied()
    }

    fn text(&self) -> String {
        match self.file().and_then(|inode| self.family.file(inode)) {
            Some(FileView::Text(text)) => text,
            _ => String::new(),
        }
    }

    fn apply(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Apply { deltas, .. } => {
                    for (_, delta) in deltas {
                        let at = self.log.len() as u64;
                        let applied = self.family.apply(delta.clone(), &mut HashMap::new()).expect("server deltas apply");
                        if applied == Applied::New {
                            self.log.push(delta);
                            self.sync.appended_from_server(at, 1);
                        }
                    }
                }
                Effect::Diverged(id) => panic!("no actor is reused, yet {id} diverged"),
            }
        }
    }
}

enum Request {
    Push(usize, Vec<Delta>),
    Pull(usize, Pull),
}

#[derive(Debug, Clone)]
enum Action {
    Edit { client: usize, pos: u16, text: String },
    Send { client: usize },
    Deliver { pick: u16, lose_response: bool },
    Snapshot,
    Rollback,
}

fn action(clients: usize) -> impl Strategy<Value = Action> {
    prop_oneof![
        3 => (0..clients, any::<u16>(), "[a-d]{1,3}").prop_map(|(client, pos, text)| Action::Edit { client, pos, text }),
        4 => (0..clients).prop_map(|client| Action::Send { client }),
        5 => (any::<u16>(), prop::bool::weighted(0.2)).prop_map(|(pick, lose_response)| Action::Deliver { pick, lose_response }),
        1 => Just(Action::Snapshot),
        1 => Just(Action::Rollback),
    ]
}

struct Network {
    clients: Vec<Client>,
    room: Room,
    snapshot: Room,
    generations: u64,
    queue: Vec<Request>,
}

impl Network {
    fn send(&mut self, index: usize) {
        let client = &mut self.clients[index];
        if let Some(push) = client.sync.next_push(client.log.len() as u64) {
            let batch = client.log[push.from as usize..push.to as usize].to_vec();
            self.queue.push(Request::Push(index, batch));
        }
        if let Some(pull) = client.sync.next_pull() {
            self.queue.push(Request::Pull(index, pull));
        }
    }

    fn deliver(&mut self, pick: usize, lose_response: bool) {
        if self.queue.is_empty() {
            return;
        }
        match self.queue.remove(pick % self.queue.len()) {
            Request::Push(index, batch) => {
                let generation = (!lose_response).then(|| self.room.generation());
                let outcome = match self.room.push(batch) {
                    Ok(_) if lose_response => Outcome::Failed,
                    Ok(_) => Outcome::Pushed,
                    Err(silt_sync::PushError::MissingParents(_)) => Outcome::MissingParents,
                    Err(silt_sync::PushError::Conflict(id)) => Outcome::Conflict(id),
                    Err(error) => panic!("unexpected push error: {error}"),
                };
                if std::env::var_os("SIM_TRACE").is_some() {
                    eprintln!("push from {index} gen {generation:?} -> {outcome:?}; room seq {}", self.room.seq());
                }
                let effects = self.clients[index].sync.on_push(generation, outcome);
                self.clients[index].apply(effects);
            }
            Request::Pull(index, pull) => {
                let generation = (!lose_response).then(|| self.room.generation());
                let outcome = match self.room.pull(pull.after, pull.last, 7) {
                    Ok(_) if lose_response => Outcome::Failed,
                    Ok(deltas) => Outcome::Pulled(deltas),
                    Err(_) => Outcome::UnknownCursor,
                };
                if std::env::var_os("SIM_TRACE").is_some() {
                    eprintln!("pull by {index} after {} gen {generation:?} -> {:?}", pull.after, match &outcome { Outcome::Pulled(d) => format!("{} deltas", d.len()), other => format!("{other:?}") });
                }
                let effects = self.clients[index].sync.on_pull(generation, outcome);
                self.clients[index].apply(effects);
            }
        }
    }

    /// Delivers everything reliably until a whole round changes nothing:
    /// every client is caught up and its pull, carrying its latest push
    /// marker, confirmed the server still has what it pushed.
    fn settle(&mut self) {
        let state = |network: &Self| -> Vec<(u64, u64, usize)> {
            network
                .clients
                .iter()
                .map(|client| (client.sync.pushed(), client.sync.pulled(), client.log.len()))
                .collect()
        };
        for _ in 0..10_000 {
            let before = state(self);
            for index in 0..self.clients.len() {
                self.send(index);
            }
            while !self.queue.is_empty() {
                self.deliver(0, false);
            }
            let caught_up = self.clients.iter().all(|client| {
                client.sync.pushed() >= client.log.len() as u64 && client.sync.pulled() >= self.room.seq()
            });
            if caught_up && state(self) == before {
                return;
            }
        }
        panic!("the network never settled");
    }
}

fn run(client_count: usize, actions: &[Action]) -> Result<(), TestCaseError> {
    let mut clients: Vec<Client> = (0..client_count).map(Client::new).collect();
    clients[0].write(Op::Create {
        kind: InodeKind::File,
        parent: Parent::Root,
        name: "a.txt".into(),
        content: Content::Empty,
        executable: false,
    });
    let mut network = Network {
        clients,
        room: Room::new(0),
        snapshot: Room::new(0),
        generations: 0,
        queue: Vec::new(),
    };
    for action in actions {
        match action {
            Action::Edit { client, pos, text } => {
                let client = &mut network.clients[*client];
                if let Some(inode) = client.file() {
                    let pos = usize::from(*pos) % (client.text().chars().count() + 1);
                    client.write(Op::Text {
                        inode,
                        op: TextOp::Insert {
                            pos,
                            content: text.clone(),
                        },
                    });
                }
            }
            Action::Send { client } => network.send(*client),
            Action::Deliver { pick, lose_response } => network.deliver(usize::from(*pick), *lose_response),
            Action::Snapshot => network.snapshot = network.room.clone(),
            Action::Rollback => {
                if std::env::var_os("SIM_TRACE").is_some() {
                    eprintln!("ROLLBACK to {} deltas", network.snapshot.seq());
                }
                network.generations += 1;
                network.room = network.snapshot.restarted(network.generations);
            }
        }
    }
    network.queue.clear();
    for client in &mut network.clients {
        let _ = client.sync.on_push(None, Outcome::Failed);
        let _ = client.sync.on_pull(None, Outcome::Failed);
    }
    network.settle();

    if std::env::var_os("SIM_DEBUG").is_some() {
        for (index, client) in network.clients.iter().enumerate() {
            for delta in network.clients.iter().flat_map(|other| &other.log) {
                if !client.family.graph().contains(delta.span.first()) {
                    eprintln!(
                        "client {index} lacks {:?}; server position {:?}; room seq {}; clients {:?}",
                        delta.span.first(),
                        network.room.position(delta.span.first()),
                        network.room.seq(),
                        network.clients.iter().map(|c| (c.actor, c.log.iter().map(|d| d.span.first().lamport.0 * 10 + d.span.actor.0 as u64).collect::<Vec<_>>(), c.sync.clone())).collect::<Vec<_>>()
                    );
                }
            }
        }
    }
    let text = network.clients[0].text();
    let frontier = network.clients[0].family.graph().frontier().clone();
    for client in &network.clients {
        prop_assert_eq!(&client.text(), &text);
        prop_assert_eq!(client.family.graph().frontier(), &frontier);
    }
    let total: usize = network.clients[0].log.len();
    prop_assert_eq!(network.room.deltas().len(), total, "the server holds every delta");
    Ok(())
}

proptest! {
    #[test]
    fn clients_converge_through_loss_reordering_and_rollback(
        (clients, actions) in (2usize..=5).prop_flat_map(|clients| {
            (Just(clients), prop::collection::vec(action(clients), 0..120))
        })
    ) {
        run(clients, &actions)?;
    }
}

/// A push sent before a reset completed after it and used to move the push
/// cursor past deltas that still had to be sent again.
#[test]
fn regression_stale_push_after_restart() {
    let ins = |client| Action::Edit { client, pos: 0, text: "a".into() };
    let del = |pick| Action::Deliver { pick, lose_response: false };
    let actions = vec![
        ins(0), ins(0), ins(0), ins(0), ins(0), ins(0),
        Action::Send { client: 0 }, ins(0), Action::Send { client: 1 }, ins(0), ins(0),
        del(45), del(0), ins(0), del(0), Action::Snapshot, ins(1),
        Action::Send { client: 0 }, del(10657), Action::Rollback, del(0),
        Action::Send { client: 1 }, del(13062), Action::Send { client: 1 }, del(14360), del(0),
        Action::Rollback, Action::Send { client: 1 }, del(395), del(0),
    ];
    run(2, &actions).expect("converges");
}
