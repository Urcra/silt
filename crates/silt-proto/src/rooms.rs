//! Rooms (ADR 0081): moments between the people viewing a page now. Sans
//! I/O: the daemon and the server feed it what peers send and deliver what
//! it answers, so both relay alike. Nothing in a room persists.

use std::collections::{BTreeMap, HashMap, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::page::Level;

/// The most a moment's data may be, as JSON.
pub const MESSAGE_BYTES: usize = 16 * 1024;
/// The most a peer's presence may be, as JSON.
pub const PRESENCE_BYTES: usize = 4 * 1024;
/// The most moments a peer sends a second.
pub const PER_SECOND: usize = 30;
/// How long presence lasts without a refresh, in milliseconds.
pub const PRESENCE_MS: i64 = 15_000;
/// The moments a room keeps for the thread's agent.
pub const KEPT: usize = 100;

/// A room: one artifact's viewers, or a sub-room of them by name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RoomKey {
    pub thread: String,
    pub artifact: String,
    #[serde(default)]
    pub name: String,
}

/// Who a peer is, as others see them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PeerInfo {
    /// The peer's connection, unique in the room.
    pub peer: String,
    /// The person's page-data id (ADR 0072).
    pub user: String,
    /// `person` or `agent`.
    pub kind: String,
}

/// What a peer sends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Sent {
    Emit { topic: String, data: Value },
    Presence { state: Value },
}

/// What a peer hears.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Heard {
    Moment { topic: String, data: Value, from: PeerInfo },
    Peers { peers: Vec<PeerPresence> },
    Refused { reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeerPresence {
    #[serde(flatten)]
    pub info: PeerInfo,
    pub state: Value,
}

struct Peer {
    info: PeerInfo,
    level: Level,
    state: Value,
    seen_ms: i64,
    sent: VecDeque<i64>,
}

#[derive(Default)]
struct Room {
    peers: BTreeMap<String, Peer>,
    /// The levels topics need, beyond the default `collaborator`.
    topics: BTreeMap<String, Level>,
    kept: VecDeque<(String, Value, PeerInfo)>,
}

/// Every room a daemon or a server relays, in memory.
#[derive(Default)]
pub struct Hub {
    rooms: HashMap<RoomKey, Room>,
}

/// Deliveries the caller makes: each peer and what it hears.
pub type Deliveries = Vec<(String, Heard)>;

/// The levels a `room` declaration opens topics to: `{topics: {move: "comment"}}`.
pub fn topic_levels(config: &Value) -> BTreeMap<String, Level> {
    config
        .get("topics")
        .and_then(Value::as_object)
        .map(|topics| topics.iter().filter_map(|(topic, level)| Some((topic.clone(), Level::parse(level.as_str()?)?))).collect())
        .unwrap_or_default()
}

impl Hub {
    /// Adds `info` to `key` at `level`, under the topics the version
    /// declares; everyone in it hears the new list of peers.
    pub fn join(
        &mut self,
        key: RoomKey,
        info: PeerInfo,
        level: Level,
        topics: BTreeMap<String, Level>,
        now_ms: i64,
    ) -> Deliveries {
        let room = self.rooms.entry(key.clone()).or_default();
        room.topics = topics;
        let peer = info.peer.clone();
        room.peers.insert(peer, Peer { info, level, state: Value::Null, seen_ms: now_ms, sent: VecDeque::new() });
        self.peers_of(&key, now_ms)
    }

    /// Takes `peer` out of `key`; a room with no one left goes.
    pub fn leave(&mut self, key: &RoomKey, peer: &str, now_ms: i64) -> Deliveries {
        let Some(room) = self.rooms.get_mut(key) else { return Vec::new() };
        room.peers.remove(peer);
        if room.peers.is_empty() {
            self.rooms.remove(key);
            return Vec::new();
        }
        self.peers_of(key, now_ms)
    }

    /// What `peer` sent to `key`, and who hears what of it.
    pub fn sent(&mut self, key: &RoomKey, peer: &str, sent: Sent, now_ms: i64) -> Deliveries {
        let refuse = |reason: &str| vec![(peer.to_owned(), Heard::Refused { reason: reason.to_owned() })];
        let Some(room) = self.rooms.get_mut(key) else { return refuse("not in that room") };
        let Some(from) = room.peers.get_mut(peer) else { return refuse("not in that room") };
        from.seen_ms = now_ms;
        match sent {
            Sent::Presence { state } => {
                if state.to_string().len() > PRESENCE_BYTES {
                    return refuse("too_large");
                }
                from.state = state;
                self.peers_of(key, now_ms)
            }
            Sent::Emit { topic, data } => {
                if topic.is_empty() || topic.len() > 64 || data.to_string().len() > MESSAGE_BYTES {
                    return refuse("too_large");
                }
                from.sent.retain(|at| now_ms - at < 1000);
                if from.sent.len() >= PER_SECOND {
                    return refuse("rate_limited");
                }
                let needs = room.topics.get(&topic).copied().unwrap_or(Level::Collaborator);
                if from.level < needs {
                    return refuse("not_granted");
                }
                from.sent.push_back(now_ms);
                let info = from.info.clone();
                room.kept.push_back((topic.clone(), data.clone(), info.clone()));
                while room.kept.len() > KEPT {
                    room.kept.pop_front();
                }
                room.peers
                    .keys()
                    .filter(|other| *other != peer)
                    .map(|other| (other.clone(), Heard::Moment { topic: topic.clone(), data: data.clone(), from: info.clone() }))
                    .collect()
            }
        }
    }

    /// The moments `key` kept, oldest first, for the thread's agent.
    pub fn kept(&self, key: &RoomKey) -> Vec<(String, Value, PeerInfo)> {
        self.rooms.get(key).map(|room| room.kept.iter().cloned().collect()).unwrap_or_default()
    }

    /// The peers present in `key`, presence that wasn't refreshed in time left out.
    pub fn present(&self, key: &RoomKey, now_ms: i64) -> Vec<PeerPresence> {
        self.rooms
            .get(key)
            .map(|room| {
                room.peers
                    .values()
                    .map(|peer| PeerPresence {
                        info: peer.info.clone(),
                        state: if now_ms - peer.seen_ms > PRESENCE_MS { Value::Null } else { peer.state.clone() },
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn peers_of(&self, key: &RoomKey, now_ms: i64) -> Deliveries {
        let peers = self.present(key, now_ms);
        self.rooms
            .get(key)
            .map(|room| room.peers.keys().map(|peer| (peer.clone(), Heard::Peers { peers: peers.clone() })).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key() -> RoomKey {
        RoomKey { thread: "t".into(), artifact: "a".into(), name: String::new() }
    }

    fn peer(name: &str) -> PeerInfo {
        PeerInfo { peer: name.into(), user: format!("u-{name}"), kind: "person".into() }
    }

    #[test]
    fn moments_reach_the_others_and_presence_reaches_everyone() {
        let mut hub = Hub::default();
        hub.join(key(), peer("ann"), Level::Owner, BTreeMap::new(), 0);
        let joined = hub.join(key(), peer("bob"), Level::Collaborator, BTreeMap::new(), 0);
        assert_eq!(joined.len(), 2, "both hear who's here");
        let heard = hub.sent(&key(), "ann", Sent::Emit { topic: "move".into(), data: json!({"x": 1}) }, 1);
        assert_eq!(heard.len(), 1);
        assert_eq!(heard[0].0, "bob");
        let presence = hub.sent(&key(), "bob", Sent::Presence { state: json!({"cursor": [3, 4]}) }, 2);
        assert_eq!(presence.len(), 2);
        assert_eq!(hub.present(&key(), 2 + PRESENCE_MS + 1).iter().filter(|peer| !peer.state.is_null()).count(), 0, "it lapses");
        assert_eq!(hub.kept(&key()).len(), 1, "kept for the agent");
        hub.leave(&key(), "ann", 3);
        hub.leave(&key(), "bob", 3);
        assert!(hub.kept(&key()).is_empty(), "nothing outlives the room");
    }

    #[test]
    fn topics_need_their_level_and_moments_are_limited() {
        let mut hub = Hub::default();
        let topics = topic_levels(&json!({"topics": {"cursor": "comment"}}));
        hub.join(key(), peer("carol"), Level::Comment, topics, 0);
        let refused = hub.sent(&key(), "carol", Sent::Emit { topic: "move".into(), data: json!(1) }, 0);
        assert!(matches!(&refused[0].1, Heard::Refused { reason } if reason == "not_granted"));
        let opened = hub.sent(&key(), "carol", Sent::Emit { topic: "cursor".into(), data: json!(1) }, 0);
        assert!(opened.is_empty(), "no one else to hear it, and not refused");
        let big = json!("x".repeat(MESSAGE_BYTES));
        assert!(matches!(
            &hub.sent(&key(), "carol", Sent::Emit { topic: "cursor".into(), data: big }, 0)[0].1,
            Heard::Refused { .. }
        ));
        for _ in 0..PER_SECOND - 1 {
            hub.sent(&key(), "carol", Sent::Emit { topic: "cursor".into(), data: json!(1) }, 10);
        }
        let too_many = hub.sent(&key(), "carol", Sent::Emit { topic: "cursor".into(), data: json!(1) }, 10);
        assert!(matches!(&too_many[0].1, Heard::Refused { reason } if reason == "rate_limited"));
    }
}
