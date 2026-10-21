//! Who made an edit the watcher noticed: the functional core of
//! attribution, from what the daemon knows of each session at that moment.

use std::time::{Duration, Instant};

use silt_agents::Capture;
use silt_log::Author;

/// Marks read this close to an edit, before or after, were written around
/// it: transcripts trail the edits. So a tool or turn read as finished
/// still owns the edits that arrive this long after; longer than the
/// transcript lag plus a tick of tailing.
pub(crate) const MARK_SETTLE: Duration = Duration::from_millis(1000);

/// What one session on the checkout says about who is editing.
#[derive(Debug, Clone)]
pub(crate) struct SessionState<'a> {
    pub(crate) agent: &'a str,
    pub(crate) session: &'a str,
    pub(crate) capture: Capture,
    pub(crate) live: bool,
    pub(crate) in_turn: bool,
    pub(crate) open_tool: Option<&'a str>,
    pub(crate) closed_tool: Option<(&'a str, Instant)>,
    pub(crate) turn_closed: Option<Instant>,
}

/// The author of an edit noticed at `noticed`, given the checkout's
/// sessions and, if an agent is running that hasn't written its session
/// yet, that agent.
pub(crate) fn author<'a>(
    importing: bool,
    sessions: &[SessionState<'a>],
    pending_agent: Option<&str>,
    noticed: Instant,
) -> Author {
    if importing {
        return Author::Import;
    }
    let settling = |at: Instant| at.max(noticed) - at.min(noticed) < MARK_SETTLE;
    let tool_of = |session: &SessionState<'a>| -> Option<&'a str> {
        session.open_tool.or_else(|| session.closed_tool.filter(|(_, at)| settling(*at)).map(|(tool, _)| tool))
    };
    if let Some((session, tool)) = sessions.iter().find_map(|session| Some((session, tool_of(session)?))) {
        return Author::Agent {
            agent: session.agent.to_owned(),
            session: session.session.to_owned(),
            tool_use_id: Some(tool.to_owned()),
        };
    }
    // Without hooks, edits are only known to fall inside a turn; a store
    // read by snapshot learns even that late, so its whole run counts.
    let in_turn = |session: &&SessionState<'_>| match session.capture {
        Capture::Hooks => false,
        Capture::Transcript => session.in_turn || session.turn_closed.is_some_and(settling),
        Capture::Snapshot => session.in_turn || session.turn_closed.is_some_and(settling) || session.live,
    };
    if let Some(session) = sessions.iter().find(in_turn) {
        return Author::Agent { agent: session.agent.to_owned(), session: session.session.to_owned(), tool_use_id: None };
    }
    match pending_agent {
        Some(agent) => Author::Agent { agent: agent.to_owned(), session: String::new(), tool_use_id: None },
        None => Author::User,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet() -> SessionState<'static> {
        SessionState {
            agent: "codex",
            session: "s1",
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
        assert_eq!(author(false, &[open], None, now), agent(Some("t1")));
        let just_closed = SessionState { closed_tool: Some(("t2", now)), ..quiet() };
        assert_eq!(author(false, std::slice::from_ref(&just_closed), None, now + MARK_SETTLE / 2), agent(Some("t2")));
        assert_eq!(author(false, &[just_closed], None, now + MARK_SETTLE * 3), Author::User, "long after, it's the user's");
    }

    #[test]
    fn without_hooks_an_edit_in_a_turn_is_the_agents() {
        let now = Instant::now();
        let turning = SessionState { in_turn: true, ..quiet() };
        assert_eq!(author(false, std::slice::from_ref(&turning), None, now), agent(None));
        let hooked = SessionState { capture: Capture::Hooks, ..turning };
        assert_eq!(author(false, &[hooked], None, now), Author::User, "hooks name the tool, so a turn alone isn't enough");
        let stopped = SessionState { turn_closed: Some(now), ..quiet() };
        assert_eq!(author(false, &[stopped], None, now + MARK_SETTLE / 2), agent(None), "a stop read just after");
        let store = SessionState { capture: Capture::Snapshot, ..quiet() };
        assert_eq!(author(false, &[store], None, now), agent(None), "a store read late counts its whole run");
    }

    #[test]
    fn a_pending_agent_or_an_import_or_else_the_user() {
        let now = Instant::now();
        let pending = Author::Agent { agent: "gemini-cli".into(), session: String::new(), tool_use_id: None };
        assert_eq!(author(false, &[quiet()], Some("gemini-cli"), now), pending);
        assert_eq!(author(true, &[SessionState { open_tool: Some("t1"), ..quiet() }], None, now), Author::Import);
        assert_eq!(author(false, &[quiet()], None, now), Author::User);
    }
}
