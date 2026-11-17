//! Rooms on the daemon (ADR 0081): the views open on this machine of a
//! page, relayed through `silt_proto::rooms::Hub`, and the thread's agent as
//! a peer that emits and reads what was said.

use silt_proto::rooms::{Deliveries, Heard, PeerInfo, RoomKey, Sent};
use silt_types::ThreadId;

use crate::core::{Core, CoreError};

impl Core {
    /// Adds a view of `key` as `peer`, if its artifact declares `room`.
    pub(crate) fn room_join(&mut self, key: RoomKey, peer: String, agent: bool) -> Result<Deliveries, CoreError> {
        let thread = self.find_thread(&key.thread)?;
        let (topics, level) = self.room_access(thread, &key.artifact)?;
        let info = PeerInfo { peer, user: self.page_me(thread), kind: if agent { "agent" } else { "person" }.into() };
        let key = RoomKey { thread: thread.to_string(), ..key };
        Ok(self.rooms.join(key, info, level, topics, crate::core::now_ms()))
    }

    pub(crate) fn room_send(&mut self, key: &RoomKey, peer: &str, sent: Sent) -> Deliveries {
        self.rooms.sent(key, peer, sent, crate::core::now_ms())
    }

    pub(crate) fn room_leave(&mut self, key: &RoomKey, peer: &str) -> Deliveries {
        self.rooms.leave(key, peer, crate::core::now_ms())
    }

    /// What the thread's agent says to a page's room, and what it hears: the
    /// last moments and who's present. The agent joins as it first speaks.
    pub(crate) fn room_agent(
        &mut self,
        thread: &str,
        artifact: &str,
        emit: Option<(String, serde_json::Value)>,
        presence: Option<serde_json::Value>,
    ) -> Result<(Deliveries, serde_json::Value), CoreError> {
        let thread_id = self.find_thread(thread)?;
        let key = RoomKey { thread: thread_id.to_string(), artifact: artifact.to_owned(), name: String::new() };
        let peer = format!("agent-{}", thread_id.short());
        let mut deliveries = Vec::new();
        if !self.rooms.present(&key, crate::core::now_ms()).iter().any(|present| present.info.peer == peer) {
            deliveries.extend(self.room_join(key.clone(), peer.clone(), true)?);
        }
        let said = emit
            .map(|(topic, data)| Sent::Emit { topic, data })
            .into_iter()
            .chain(presence.map(|state| Sent::Presence { state }));
        for sent in said {
            let sent = self.room_send(&key, &peer, sent);
            if let Some((_, Heard::Refused { reason })) = sent.iter().find(|(to, _)| *to == peer) {
                return Err(CoreError::Invalid(format!("the room refused it: {reason}")));
            }
            deliveries.extend(sent);
        }
        let kept: Vec<serde_json::Value> = self
            .rooms
            .kept(&key)
            .into_iter()
            .map(|(topic, data, from)| serde_json::json!({"topic": topic, "data": data, "from": from}))
            .collect();
        let peers = self.rooms.present(&key, crate::core::now_ms());
        Ok((deliveries, serde_json::json!({ "messages": kept, "peers": peers })))
    }

    /// The topics a room opens below `collaborator`, and this machine's
    /// person's level, if the artifact's latest version declares `room`.
    fn room_access(
        &self,
        thread: ThreadId,
        artifact: &str,
    ) -> Result<(std::collections::BTreeMap<String, silt_proto::page::Level>, silt_proto::page::Level), CoreError> {
        let id: silt_types::ArtifactId =
            artifact.parse().map_err(|_| CoreError::Invalid(format!("{artifact} isn't an artifact id")))?;
        let config = self.declared(thread, id, "room")?;
        Ok((silt_proto::rooms::topic_levels(&config), self.page_level(thread)))
    }
}
