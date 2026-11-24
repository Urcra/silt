//! Agents in one project told of each other, and their messages (ADR 0090).
//! The functional core decides who is present and how each agent is
//! described to another; the store's part, which both serving modes run, is
//! at the end.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use silt_store::{AgentMessage, AgentMessageKind, SessionActivity, Store};
use silt_types::ThreadId;

use crate::core::{Core, CoreError, now_ms};

/// How long after it was last heard from an agent that never said it ended
/// counts as present; past it, it most likely crashed.
pub(crate) const PRESENT_FOR_MS: i64 = 12 * 60 * 60 * 1000;
const DOING_CHARS: usize = 120;
/// The most files a peer is listed with.
const TOUCHES: i64 = 8;
pub(crate) const MESSAGE_CHARS: usize = 4000;

/// What a prompt asks for, as a peer is told: its first line, clipped.
pub(crate) fn doing_of(prompt: &str) -> Option<String> {
    let line = prompt.lines().map(str::trim).find(|line| !line.is_empty())?;
    Some(match line.char_indices().nth(DOING_CHARS) {
        Some((end, _)) => format!("{}…", &line[..end]),
        None => line.to_owned(),
    })
}

/// Whether the session's agent is still around. An agent the web UI runs
/// is present while it runs here; any other until it ends, or for
/// [`PRESENT_FOR_MS`] after it was last heard from.
pub(crate) fn present(session: &SessionActivity, acp_running: bool, now_ms: i64) -> bool {
    if session.agent == silt_agents::acp::AGENT {
        return acp_running;
    }
    session.ended_ms.is_none() && now_ms - session.seen_ms.unwrap_or(session.created_ms) < PRESENT_FOR_MS
}

/// Where a peer works, from where the agent told of it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Place {
    WorkingTree,
    /// A managed checkout, by its path in the repository.
    OwnCheckout(String),
    OtherClone(PathBuf),
}

impl Place {
    fn of(peer: &SessionActivity, mine: &Path) -> Self {
        if peer.checkout == mine {
            Self::WorkingTree
        } else if peer.managed {
            let relative = peer.checkout.strip_prefix(&peer.repo_root).unwrap_or(&peer.checkout);
            Self::OwnCheckout(relative.to_string_lossy().replace('\\', "/"))
        } else {
            Self::OtherClone(peer.checkout.clone())
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::WorkingTree => "in your working tree".to_owned(),
            Self::OwnCheckout(path) => format!("in its own checkout `{path}`"),
            Self::OtherClone(path) => format!("in another clone at {}", path.display()),
        }
    }
}

/// Another thread's agent present in the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Peer {
    pub(crate) thread: ThreadId,
    /// The agent's id, as its sessions are recorded.
    pub(crate) agent_id: String,
    pub(crate) session: String,
    pub(crate) place: Place,
    pub(crate) doing: Option<String>,
    pub(crate) files: Vec<String>,
    pub(crate) idle: bool,
    pub(crate) seen_ms: i64,
}

impl Peer {
    /// `thread 1a2b3c4d5e6f (claude, in your working tree)`.
    fn label(&self) -> String {
        format!("thread {} ({}, {})", self.thread.short(), agent_name(&self.agent_id), self.place.describe())
    }
}

/// The command a person knows the agent by.
fn agent_name(agent_id: &str) -> String {
    silt_agents::agent(agent_id).map_or_else(|| agent_id.to_owned(), |agent| agent.command().to_owned())
}

/// The newest session of `me`'s thread.
fn mine(activity: &[SessionActivity], me: ThreadId) -> Option<&SessionActivity> {
    activity.iter().filter(|session| session.thread == me).max_by_key(|session| session.created_ms)
}

fn same_project(one: &SessionActivity, other: &SessionActivity) -> bool {
    if one.repo.is_empty() || other.repo.is_empty() { one.repo_root == other.repo_root } else { one.repo == other.repo }
}

/// The agents of other threads present in `me`'s project, each thread's
/// newest session, most recently heard from first; their files are left for
/// the store to fill in.
pub(crate) fn peers(
    activity: &[SessionActivity],
    me: ThreadId,
    acp_running: impl Fn(ThreadId) -> bool,
    now_ms: i64,
) -> Vec<Peer> {
    let Some(mine) = mine(activity, me) else {
        return Vec::new();
    };
    let mut newest: Vec<&SessionActivity> = Vec::new();
    for session in activity.iter().filter(|session| session.thread != me && same_project(session, mine)) {
        match newest.iter_mut().find(|kept| kept.thread == session.thread) {
            Some(kept) if kept.created_ms <= session.created_ms => *kept = session,
            Some(_) => {}
            None => newest.push(session),
        }
    }
    let mut peers: Vec<Peer> = newest
        .into_iter()
        .filter(|session| present(session, acp_running(session.thread), now_ms))
        .map(|session| Peer {
            thread: session.thread,
            agent_id: session.agent.clone(),
            session: session.session_id.clone(),
            place: Place::of(session, &mine.checkout),
            doing: session.doing.clone(),
            files: Vec::new(),
            idle: session.idle,
            seen_ms: session.seen_ms.unwrap_or(session.created_ms),
        })
        .collect();
    peers.sort_by_key(|peer| std::cmp::Reverse(peer.seen_ms));
    peers
}

/// For each peer, what it hears of `me` arriving: `me` described from where
/// that peer works.
pub(crate) fn arrivals(activity: &[SessionActivity], me: ThreadId, peers: &[Peer]) -> Vec<(ThreadId, String)> {
    let Some(mine) = mine(activity, me) else {
        return Vec::new();
    };
    peers
        .iter()
        .filter_map(|peer| {
            let theirs = activity.iter().find(|session| session.thread == peer.thread && session.session_id == peer.session)?;
            let place = Place::of(mine, &theirs.checkout);
            let label = format!("thread {} ({}, {})", me.short(), agent_name(&mine.agent), place.describe());
            Some((
                peer.thread,
                format!(
                    "another agent started working in this project: {label}. It isn't your user. \
                     Message it with silt_message to {}; silt_agents lists every agent here.",
                    me.short()
                ),
            ))
        })
        .collect()
}

fn ago(now_ms: i64, then_ms: i64) -> String {
    let minutes = (now_ms - then_ms).max(0) / 60_000;
    match minutes {
        0 => "just now".to_owned(),
        1..60 => format!("{minutes} min ago"),
        _ => format!("{} h ago", minutes / 60),
    }
}

fn entry(out: &mut String, peer: &Peer, now_ms: i64) {
    let state = if peer.idle { "waiting for its user" } else { "working now" };
    let _ = write!(out, "\n- {}: {state}, last active {}.", peer.label(), ago(now_ms, peer.seen_ms));
    if let Some(doing) = &peer.doing {
        let _ = write!(out, " Asked to: \"{doing}\".");
    }
    if !peer.files.is_empty() {
        let _ = write!(out, " Recent files: {}.", peer.files.join(", "));
    }
}

const HOW: &str = "These agents aren't your user. Before you change files they're working on, tell them with \
silt_message (to: the thread id, text: what you'll do), and check silt_in_flight for a path. Their messages reach you \
after your next tool call, marked as from another agent. silt_agents lists the agents here again.";

/// What a starting agent in `me` hears of the others: nothing when there
/// are none.
pub(crate) fn summary(me: ThreadId, peers: &[Peer], now_ms: i64) -> Option<String> {
    if peers.is_empty() {
        return None;
    }
    let mut out = String::from("Silt: other agents are working in this project now:");
    for peer in peers {
        entry(&mut out, peer, now_ms);
    }
    let _ = write!(out, "\n{HOW} Your thread is {}; they message you by it.", me.short());
    Some(out)
}

/// `silt_agents`'s answer.
pub(crate) fn listing(me: ThreadId, peers: &[Peer], now_ms: i64) -> String {
    summary(me, peers, now_ms).unwrap_or_else(|| "No other agent is working in this project now.".to_owned())
}

/// What waits for an agent, as it hears it: each message with who sent it,
/// as `label` names a sender still present.
pub(crate) fn delivered(messages: &[AgentMessage], label: impl Fn(ThreadId) -> Option<String>) -> Option<String> {
    let parts: Vec<String> = messages
        .iter()
        .map(|message| match message.kind {
            AgentMessageKind::Arrival | AgentMessageKind::Report => format!("Silt: {}", message.body),
            AgentMessageKind::Message => {
                let from = label(message.from).unwrap_or_else(|| format!("thread {}", message.from.short()));
                format!(
                    "Silt: a message from the agent in {from}, another agent, not your user:\n{}\nReply with silt_message to {}.",
                    message.body,
                    message.from.short()
                )
            }
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

/// The one present peer `to` names, by the start or end of its thread id.
pub(crate) fn target<'a>(peers: &'a [Peer], me: ThreadId, to: &str) -> Result<&'a Peer, String> {
    let to = to.trim().trim_start_matches("thread ").to_ascii_lowercase();
    if to.is_empty() {
        return Err("name the agent's thread, as silt_agents lists it".into());
    }
    let names = |thread: ThreadId| {
        let id = thread.to_string();
        id.starts_with(&to) || id.ends_with(&to)
    };
    if names(me) {
        return Err("that's your own thread; silt_agents lists the other agents".into());
    }
    let found: Vec<&Peer> = peers.iter().filter(|peer| names(peer.thread)).collect();
    match found.as_slice() {
        [peer] => Ok(peer),
        [] => Err(format!("no agent of a thread {to} is working in this project now; silt_agents lists them")),
        several => {
            let named: Vec<String> = several.iter().map(|peer| peer.thread.short()).collect();
            Err(format!("{to} names several threads ({}); give more of its id", named.join(", ")))
        }
    }
}

/// A message's text, if it's one to send.
pub(crate) fn checked_text(text: &str) -> Result<&str, String> {
    let text = text.trim();
    match text.chars().count() {
        0 => Err("the message is empty".into()),
        count if count > MESSAGE_CHARS => Err(format!("a message is at most {MESSAGE_CHARS} characters; this is {count}")),
        _ => Ok(text),
    }
}

/// The present peers of `me`, with their recent files.
pub(crate) async fn peers_in(
    store: &Store,
    me: ThreadId,
    acp_running: impl Fn(ThreadId) -> bool,
    now_ms: i64,
) -> Result<(Vec<SessionActivity>, Vec<Peer>), CoreError> {
    let activity = store.session_activity().await?;
    let mut peers = peers(&activity, me, acp_running, now_ms);
    for peer in &mut peers {
        peer.files = store.recent_touches(&peer.agent_id, &peer.session, TOUCHES).await?;
    }
    Ok((activity, peers))
}

/// Sends `text` from `from`'s agent to the present peer `to` names.
pub(crate) async fn send_in(
    store: &Store,
    from: ThreadId,
    to: &str,
    text: &str,
    acp_running: impl Fn(ThreadId) -> bool,
) -> Result<String, CoreError> {
    let text = checked_text(text).map_err(CoreError::Invalid)?;
    let now = now_ms();
    let (_, peers) = peers_in(store, from, acp_running, now).await?;
    let peer = target(&peers, from, to).map_err(CoreError::Invalid)?;
    store.add_agent_message(peer.thread, from, AgentMessageKind::Message, text, now).await?;
    let when = if peer.idle { "with its next prompt, as it waits for its user" } else { "after its next tool call" };
    Ok(format!("Sent to {}; it gets the message {when}.", peer.label()))
}

/// What waits for `thread`'s agent from other agents, taken.
pub(crate) async fn messages_in(
    store: &Store,
    thread: ThreadId,
    acp_running: impl Fn(ThreadId) -> bool,
) -> Result<Option<String>, CoreError> {
    let messages = store.take_agent_messages(thread).await?;
    if messages.is_empty() {
        return Ok(None);
    }
    let (_, peers) = peers_in(store, thread, acp_running, now_ms()).await?;
    let label = |from: ThreadId| peers.iter().find(|peer| peer.thread == from).map(Peer::label);
    Ok(delivered(&messages, label))
}

/// The thread `selector` names, by the start or end of its id.
async fn thread_of(store: &Store, selector: &str) -> Result<ThreadId, CoreError> {
    let found: Vec<ThreadId> = store
        .threads()
        .await?
        .into_iter()
        .map(|thread| thread.id)
        .filter(|thread| {
            let id = thread.to_string();
            id.starts_with(selector) || id.ends_with(selector)
        })
        .collect();
    match found.as_slice() {
        [thread] => Ok(*thread),
        [] => Err(CoreError::Invalid(format!("no thread matches {selector}"))),
        _ => Err(CoreError::Invalid(format!("{selector} matches several threads"))),
    }
}

/// `silt_agents` without a daemon: the store in `data_dir` says.
pub async fn agents_here(data_dir: &Path, thread: &str) -> Result<String, CoreError> {
    let store = Store::open(data_dir).await?;
    let now = now_ms();
    // Agents the web UI runs need the daemon, so none runs.
    let me = thread_of(&store, thread).await?;
    let (_, peers) = peers_in(&store, me, |_| false, now).await?;
    store.close().await;
    Ok(listing(me, &peers, now))
}

/// `silt_message` without a daemon.
pub async fn send(data_dir: &Path, from: &str, to: &str, text: &str) -> Result<String, CoreError> {
    let store = Store::open(data_dir).await?;
    let sent = match thread_of(&store, from).await {
        Ok(from) => send_in(&store, from, to, text, |_| false).await,
        Err(error) => Err(error),
    };
    store.close().await;
    sent
}

impl Core {
    fn acp_running(&self) -> impl Fn(ThreadId) -> bool + '_ {
        |thread| self.acp.contains_key(&thread)
    }

    /// What a starting agent hears of the others, and on `startup` or
    /// `resume`, they hear of it.
    pub(crate) async fn greet(&self, thread: ThreadId, announce: bool) -> Result<Option<String>, CoreError> {
        let now = now_ms();
        let (activity, peers) = peers_in(&self.store, thread, self.acp_running(), now).await?;
        if announce {
            for (to, body) in arrivals(&activity, thread, &peers) {
                self.store.add_agent_message(to, thread, AgentMessageKind::Arrival, &body, now).await?;
            }
        }
        Ok(summary(thread, &peers, now))
    }

    pub(crate) async fn agents(&self, thread: ThreadId) -> Result<String, CoreError> {
        let now = now_ms();
        let (_, peers) = peers_in(&self.store, thread, self.acp_running(), now).await?;
        Ok(listing(thread, &peers, now))
    }

    pub(crate) async fn message(&self, from: ThreadId, to: &str, text: &str) -> Result<String, CoreError> {
        send_in(&self.store, from, to, text, self.acp_running()).await
    }

    pub(crate) async fn take_messages(&self, thread: ThreadId) -> Result<Option<String>, CoreError> {
        messages_in(&self.store, thread, self.acp_running()).await
    }

    /// Records what each of the thread's live sessions was asked to do.
    pub(crate) async fn note_doing(&self, thread: ThreadId, prompt: &str) -> Result<(), CoreError> {
        let Some(doing) = doing_of(prompt) else {
            return Ok(());
        };
        for session in self.sessions.iter().filter(|session| session.record.thread == thread && session.live) {
            self.store.session_doing(&session.record.agent, &session.record.session_id, &doing).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: ThreadId = ThreadId(0x1111_0000_0000_0000_0000_aaaa_aaaa_aaaa);
    const BESIDE: ThreadId = ThreadId(0x2222_0000_0000_0000_0000_bbbb_bbbb_bbbb);
    const ISOLATED: ThreadId = ThreadId(0x3333_0000_0000_0000_0000_cccc_cccc_cccc);
    const NOW: i64 = 100 * 60 * 60 * 1000;

    fn session(thread: ThreadId, id: &str, checkout: &str, managed: bool) -> SessionActivity {
        SessionActivity {
            agent: "claude-code".into(),
            session_id: id.into(),
            thread,
            repo: "github.com/u/app".into(),
            repo_root: PathBuf::from("/repo"),
            checkout: PathBuf::from(checkout),
            managed,
            created_ms: NOW - 1000,
            seen_ms: Some(NOW - 1000),
            ended_ms: None,
            idle: false,
            doing: None,
        }
    }

    fn world() -> Vec<SessionActivity> {
        vec![
            session(ME, "mine", "/repo", false),
            SessionActivity { doing: Some("add a farewell".into()), ..session(BESIDE, "beside", "/repo", false) },
            SessionActivity { idle: true, ..session(ISOLATED, "isolated", "/repo/.silt/checkouts/c3", true) },
            SessionActivity { repo: "github.com/u/other".into(), ..session(ThreadId(9), "elsewhere", "/other", false) },
        ]
    }

    #[test]
    fn present_means_not_ended_and_heard_from_lately_or_running_here() {
        let fresh = session(BESIDE, "s", "/repo", false);
        assert!(present(&fresh, false, NOW));
        assert!(!present(&SessionActivity { ended_ms: Some(NOW - 1), ..fresh.clone() }, false, NOW));
        assert!(!present(&SessionActivity { seen_ms: Some(NOW - PRESENT_FOR_MS), ..fresh.clone() }, false, NOW), "a crash");
        let acp = SessionActivity { agent: silt_agents::acp::AGENT.into(), ..fresh };
        assert!(present(&acp, true, NOW) && !present(&acp, false, NOW), "the web UI's agent while it runs here");
    }

    #[test]
    fn peers_are_the_projects_other_threads_said_from_where_each_works() {
        let found = peers(&world(), ME, |_| false, NOW);
        let places: Vec<(ThreadId, Place)> = found.iter().map(|peer| (peer.thread, peer.place.clone())).collect();
        assert_eq!(places, [(BESIDE, Place::WorkingTree), (ISOLATED, Place::OwnCheckout(".silt/checkouts/c3".into()))]);
        let text = summary(ME, &found, NOW).expect("there are peers");
        assert!(text.contains(&format!("thread {} (claude, in your working tree): working now", BESIDE.short())), "{text}");
        assert!(text.contains("Asked to: \"add a farewell\""), "{text}");
        assert!(text.contains("(claude, in its own checkout `.silt/checkouts/c3`): waiting for its user"), "{text}");
        assert!(text.contains("silt_message") && text.contains("aren't your user"), "{text}");
        let alone = [session(ME, "mine", "/repo", false)];
        assert_eq!(summary(ME, &peers(&alone, ME, |_| false, NOW), NOW), None, "nothing to say alone");
        assert!(text.ends_with(&format!("Your thread is {}; they message you by it.", ME.short())), "{text}");
        let newer = SessionActivity { created_ms: NOW, session_id: "again".into(), ..session(BESIDE, "", "/elsewhere", false) };
        let again = peers(&[world(), vec![newer]].concat(), ME, |_| false, NOW);
        assert_eq!(again[0].place, Place::OtherClone("/elsewhere".into()), "a thread's newest session counts");
    }

    #[test]
    fn an_arrival_is_said_from_where_each_peer_works() {
        let world = world();
        let found = peers(&world, ME, |_| false, NOW);
        let heard = arrivals(&world, ME, &found);
        assert_eq!(heard.len(), 2);
        assert!(heard[0].1.contains(&format!("thread {} (claude, in your working tree)", ME.short())), "{}", heard[0].1);
        assert!(heard[1].1.contains("(claude, in another clone at /repo)"), "{}", heard[1].1);
    }

    #[test]
    fn a_message_goes_to_exactly_one_present_peer() {
        let found = peers(&world(), ME, |_| false, NOW);
        assert_eq!(target(&found, ME, &BESIDE.short()).map(|peer| peer.thread), Ok(BESIDE));
        assert_eq!(target(&found, ME, &format!("thread {}", BESIDE)).map(|peer| peer.thread), Ok(BESIDE));
        assert!(target(&found, ME, &ME.short()).is_err_and(|error| error.contains("your own")));
        assert!(target(&found, ME, "000000").is_err_and(|error| error.contains("no agent")));
        assert!(target(&found, ME, "").is_err());
        let twins = [found[0].clone(), Peer { thread: ThreadId(0x2222_0000_0000_0000_0000_dddd_dddd_dddd), ..found[0].clone() }];
        assert!(target(&twins, ME, "2222").is_err_and(|error| error.contains("several")));
        assert_eq!(checked_text("  hi "), Ok("hi"));
        assert!(checked_text(" ").is_err() && checked_text(&"x".repeat(MESSAGE_CHARS + 1)).is_err());
    }

    #[test]
    fn messages_are_delivered_as_another_agents() {
        let messages = [
            AgentMessage { from: BESIDE, kind: AgentMessageKind::Arrival, body: "another agent started".into(), created_ms: 1 },
            AgentMessage { from: BESIDE, kind: AgentMessageKind::Message, body: "I'm on main.rs".into(), created_ms: 2 },
        ];
        let label = |_| Some(format!("thread {} (claude, in your working tree)", BESIDE.short()));
        let text = delivered(&messages, label).expect("something to say");
        assert!(text.starts_with("Silt: another agent started\n\nSilt: a message from the agent in thread"), "{text}");
        assert!(text.contains("another agent, not your user:\nI'm on main.rs\nReply with silt_message to"), "{text}");
        assert_eq!(delivered(&[], |_| None), None);
        assert_eq!(doing_of("\n  Add a farewell line\nmore"), Some("Add a farewell line".into()));
        assert_eq!(doing_of(&"é".repeat(130)).map(|doing| doing.chars().count()), Some(DOING_CHARS + 1));
        assert_eq!(doing_of("  \n "), None);
    }
}
