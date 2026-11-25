# 0004: Crate boundaries and dependency direction

- Status: accepted
- Date: 2026-10-20

## Context
The same CRDT and projection logic has to run in the daemon, in the server, and later in the browser (WASM). The I/O-heavy parts (git, the file system, ptys, agents, the network) must not leak into that core.

## Decision
- **Pure crates** have no I/O and must build for wasm32: `silt-types` ← `silt-sumtree` ← `silt-crdt` ← `silt-log`.
- **Shell crates:**
  - `silt-store` (sqlx SQLite, blobs)
  - `silt-git`, `silt-checkout`
  - `silt-proto`, `silt-sync`
  - `silt-acp`
  - later: `silt-mcp`, `silt-skills`, `silt-redact`, `silt-harness`
- **Wiring:** `silt-daemon` and `silt-server` are libraries that connect the shell crates together. `silt-server` never depends on checkout or acp.
- **The `silt` binary** comes from `silt-cli` and links both the daemon and the server. The web bundle is embedded with `rust-embed`.
- **Errors:** libraries use `thiserror` enums; binaries use `exn`.

## Consequences
- More crates to manage. In return, compile-time boundaries enforce the functional core / imperative shell split.
- New crates are added when their milestone starts, not up front.

## Amendment (2026-10-20)
Two crates created empty in the first skeleton were removed, having never been used:
- **`silt-api`**, a web-UI API shared by daemon and server. The browser only talks to its own daemon (ADR 0011), so that API lives in `silt-daemon`'s `web` module and the TypeScript types in `web/src/api.ts`.
- **`silt-term`**, pty terminals. People run agents in their own terminals (ADR 0007), shared terminals are not scheduled (ADR 0006), and the harness runs one-off commands in `silt sandbox`. It comes back if shared terminals do.
