-- The git state a session's tool call started at, kept until the call
-- ends, to tell whether git rewrote the worktree meanwhile (ADR 0066); a
-- `silt_git::GitState` as text.
CREATE TABLE tool_git_states (
    agent TEXT NOT NULL,
    session_id TEXT NOT NULL,
    tool_use_id TEXT NOT NULL,
    state TEXT NOT NULL,
    PRIMARY KEY (agent, session_id, tool_use_id)
);
