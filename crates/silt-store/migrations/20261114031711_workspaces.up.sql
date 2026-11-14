-- Workspaces (ADR 0064). A project the person put in a workspace, by
-- normalized repository; others go by the workspaces' match rules.
CREATE TABLE project_workspaces (
    repo TEXT PRIMARY KEY NOT NULL,
    workspace TEXT NOT NULL
);

-- The workspace a family was made in, as its home is pinned.
CREATE TABLE family_workspaces (
    family_id BLOB PRIMARY KEY NOT NULL,
    workspace TEXT NOT NULL
);

-- One per agent Silt starts: what its history tools may read. Only the
-- token's hash is kept.
CREATE TABLE run_tokens (
    hash BLOB PRIMARY KEY NOT NULL,
    thread_id BLOB NOT NULL,
    workspace TEXT NOT NULL,
    created_ms INTEGER NOT NULL
);

-- An org's policy as this machine last fetched it from the org's server.
CREATE TABLE org_policies (
    server TEXT NOT NULL,
    org TEXT NOT NULL,
    policy TEXT NOT NULL,
    fetched_ms INTEGER NOT NULL,
    PRIMARY KEY (server, org)
);
