//! Who made an edit the watcher noticed: the functional core of
//! attribution, from what the daemon knows of each session at that moment.

use std::time::{Duration, Instant};

use silt_agents::Capture;
use silt_log::{Author, SessionRef};

/// Marks read this close to an edit, before or after, were written around
/// it: transcripts trail the edits. So a tool or turn read as finished
/// still owns the edits that arrive this long after; longer than the
/// transcript lag plus a tick of tailing.
pub(crate) const MARK_SETTLE: Duration = Duration::from_millis(1000);

/// What one session working at the checkout's path says about who is
/// editing: a session of the checkout's own thread, or of another thread
/// sharing the clone (ADR 0089).
#[derive(Debug, Clone)]
pub(crate) struct SessionState<'a> {
    pub(crate) agent: &'a str,
    pub(crate) session: &'a str,
    /// Whether it's a session of the checkout's own thread.
    pub(crate) own: bool,
    pub(crate) capture: Capture,
    pub(crate) live: bool,
    pub(crate) in_turn: bool,
    pub(crate) open_tool: Option<&'a str>,
    pub(crate) closed_tool: Option<(&'a str, Instant)>,
    pub(crate) turn_closed: Option<Instant>,
}

/// An agent that is running but hasn't written its session yet.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pending<'a> {
    pub(crate) agent: &'a str,
    pub(crate) own: bool,
}

/// The author of an edit noticed at `noticed`, given the sessions working
/// at the checkout's path. `deciding` is a session known to have made it,
/// as a file tool's hook names its paths; otherwise the one session that
/// could have made it is credited, and several are listed as unclear.
pub(crate) fn author<'a>(
    importing: bool,
    sessions: &[SessionState<'a>],
    deciding: Option<&str>,
    pending: Option<Pending<'_>>,
    noticed: Instant,
) -> Author {
    if importing {
        return Author::Import;
    }
    let settling = |at: Instant| at.max(noticed) - at.min(noticed) < MARK_SETTLE;
    let tool_of = |session: &SessionState<'a>| -> Option<&'a str> {
        session.open_tool.or_else(|| session.closed_tool.filter(|(_, at)| settling(*at)).map(|(tool, _)| tool))
    };
    if let Some(session) = deciding.and_then(|id| sessions.iter().find(|session| session.session == id)) {
        return one(session, tool_of(session));
    }
    let with_tools: Vec<&SessionState<'a>> = sessions.iter().filter(|session| tool_of(session).is_some()).collect();
    match with_tools.as_slice() {
        [session] => return one(session, tool_of(session)),
        [] => {}
        several => return unclear(several),
    }
    // Without hooks, edits are only known to fall inside a turn; a store
    // read by snapshot learns even that late, so its whole run counts.
    let in_turn = |session: &&SessionState<'_>| match session.capture {
        Capture::Hooks => false,
        Capture::Transcript => session.in_turn || session.turn_closed.is_some_and(settling),
        Capture::Snapshot => session.in_turn || session.turn_closed.is_some_and(settling) || session.live,
    };
    let turning: Vec<&SessionState<'a>> = sessions.iter().filter(in_turn).collect();
    match turning.as_slice() {
        [session] => return one(session, None),
        [] => {}
        several => return unclear(several),
    }
    match pending {
        Some(Pending { agent, own: true }) => {
            Author::Agent { agent: agent.to_owned(), session: String::new(), tool_use_id: None }
        }
        Some(Pending { agent, own: false }) => {
            Author::Others { sessions: vec![SessionRef { agent: agent.to_owned(), id: String::new() }] }
        }
        None => Author::User,
    }
}

/// One session's edit: its own thread credits its tool call, any other
/// thread only the session, as the tool call is no part of its conversation.
fn one(session: &SessionState<'_>, tool: Option<&str>) -> Author {
    if session.own {
        Author::Agent {
            agent: session.agent.to_owned(),
            session: session.session.to_owned(),
            tool_use_id: tool.map(str::to_owned),
        }
    } else {
        Author::Others { sessions: vec![reference(session)] }
    }
}

fn unclear(sessions: &[&SessionState<'_>]) -> Author {
    let references = sessions.iter().map(|session| reference(session)).collect();
    if sessions.iter().any(|session| session.own) {
        Author::Agents { sessions: references }
    } else {
        Author::Others { sessions: references }
    }
}

fn reference(session: &SessionState<'_>) -> SessionRef {
    SessionRef { agent: session.agent.to_owned(), id: session.session.to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet() -> SessionState<'static> {
        SessionState {
            agent: "codex",
            session: "s1",
            own: true,
            capture: Capture::Transcript,
            live: true,
            in_turn: false,
            open_tool: None,
            closed_tool: None,
            turn_closed: None,
        }
    }

    fn agent(tool: Option<&str>) -> Author {
        Author::Agent { agent: "codex".into(), session: "s1".into(), tool_use_id: tool.map(str::to_owned) }
    }

    #[test]
    fn an_open_tool_or_one_just_closed_made_the_edit() {
        let now = Instant::now();
        let open = SessionState { open_tool: Some("t1"), ..quiet() };
        assert_eq!(author(false, &[open], None, None, now), agent(Some("t1")));
        let just_closed = SessionState { closed_tool: Some(("t2", now)), ..quiet() };
        assert_eq!(author(false, std::slice::from_ref(&just_closed), None, None, now + MARK_SETTLE / 2), agent(Some("t2")));
        assert_eq!(author(false, &[just_closed], None, None, now + MARK_SETTLE * 3), Author::User, "long after, it's the user's");
    }

    #[test]
    fn without_hooks_an_edit_in_a_turn_is_the_agents() {
        let now = Instant::now();
        let turning = SessionState { in_turn: true, ..quiet() };
        assert_eq!(author(false, std::slice::from_ref(&turning), None, None, now), agent(None));
        let hooked = SessionState { capture: Capture::Hooks, ..turning };
        assert_eq!(author(false, &[hooked], None, None, now), Author::User, "hooks name the tool, so a turn alone isn't enough");
        let stopped = SessionState { turn_closed: Some(now), ..quiet() };
        assert_eq!(author(false, &[stopped], None, None, now + MARK_SETTLE / 2), agent(None), "a stop read just after");
        let store = SessionState { capture: Capture::Snapshot, ..quiet() };
        assert_eq!(author(false, &[store], None, None, now), agent(None), "a store read late counts its whole run");
    }

    #[test]
    fn a_pending_agent_or_an_import_or_else_the_user() {
        let now = Instant::now();
        let pending = Author::Agent { agent: "gemini-cli".into(), session: String::new(), tool_use_id: None };
        assert_eq!(author(false, &[quiet()], None, Some(Pending { agent: "gemini-cli", own: true }), now), pending);
        assert_eq!(author(true, &[SessionState { open_tool: Some("t1"), ..quiet() }], None, None, now), Author::Import);
        assert_eq!(author(false, &[quiet()], None, None, now), Author::User);
    }

    fn other(session: &'static str, tool: Option<&'static str>) -> SessionState<'static> {
        SessionState { agent: "claude", session, own: false, capture: Capture::Hooks, open_tool: tool, ..quiet() }
    }

    fn refs(sessions: &[(&str, &str)]) -> Vec<SessionRef> {
        sessions.iter().map(|(agent, id)| SessionRef { agent: (*agent).into(), id: (*id).into() }).collect()
    }

    #[test]
    fn another_threads_agent_is_credited_without_its_tool() {
        let now = Instant::now();
        let mine = SessionState { capture: Capture::Hooks, ..quiet() };
        let theirs = other("s2", Some("toolu_1"));
        assert_eq!(
            author(false, &[mine.clone(), theirs.clone()], None, None, now),
            Author::Others { sessions: refs(&[("claude", "s2")]) },
            "the other thread's tool call is its own, not this thread's"
        );
        assert_eq!(
            author(false, &[mine], None, Some(Pending { agent: "codex", own: false }), now),
            Author::Others { sessions: refs(&[("codex", "")]) },
            "an agent starting beside this one"
        );
    }

    #[test]
    fn with_two_tools_open_the_hook_that_names_the_file_decides() {
        let now = Instant::now();
        let mine = SessionState { capture: Capture::Hooks, open_tool: Some("toolu_1"), ..quiet() };
        let theirs = other("s2", Some("toolu_1"));
        let both = [mine, theirs];
        assert_eq!(author(false, &both, Some("s1"), None, now), agent(Some("toolu_1")));
        assert_eq!(author(false, &both, Some("s2"), None, now), Author::Others { sessions: refs(&[("claude", "s2")]) });
        assert_eq!(
            author(false, &both, None, None, now),
            Author::Agents { sessions: refs(&[("codex", "s1"), ("claude", "s2")]) },
            "a watcher event or a rescan can't tell which"
        );
        assert_eq!(
            author(false, &both, Some("gone"), None, now),
            Author::Agents { sessions: refs(&[("codex", "s1"), ("claude", "s2")]) },
            "a session not working here decides nothing"
        );
        let two_others = [quiet(), other("s2", Some("t")), other("s3", Some("t"))];
        assert_eq!(
            author(false, &two_others, None, None, now),
            Author::Others { sessions: refs(&[("claude", "s2"), ("claude", "s3")]) },
            "this thread's agent is idle, so it was one of the others"
        );
        let turning =
            [SessionState { in_turn: true, ..quiet() }, SessionState { in_turn: true, own: false, session: "s2", ..quiet() }];
        assert_eq!(
            author(false, &turning, None, None, now),
            Author::Agents { sessions: refs(&[("codex", "s1"), ("codex", "s2")]) },
            "two agents without hooks in a turn at once"
        );
    }
}
