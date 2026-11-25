# 0077: Docs and sheets that people and the agent edit together

- Status: accepted
- Date: 2026-11-15
- Amends: [ADR 0072](0072-what-an-artifacts-page-can-do.md) (`Op::Page`), [ADR 0076](0076-artifact-types-slides-canvases-and-data-pages.md) (the types), [ADR 0055](0055-reviewing-a-threads-changes.md) (comment targets)
- Amended by: [ADR 0082](0082-comments-inside-docs-and-pages.md): Comments inside docs and pages
- Amended by: [ADR 0083](0083-a-sheets-names-widths-and-formats.md): A sheet's names, column widths and number formats

## Context
A plan, a spec or a tracker is something people keep editing, not something an agent shows once. In Silt today a person can only comment on an artifact and wait for the agent to write a new version. People need docs and sheets that they and the agent edit together, with comments and exports.

Silt already has what this needs: the text CRDT `silt-crdt` that files use, with anchors comments follow (ADR 0066), the WASM replica that edits it live in the browser (ADR 0014), and the log that carries it. A doc or a sheet can be edited through the same machinery, kept beside the thread rather than in its tree, so it never shows in the thread's changes or a commit.

## Decision
**Two types edited in place:** `doc` and `sheet`. Their state is the thread's page data (ADR 0072), not a blob per version.

**`doc`: a document.**
- **Text:** Markdown, held in a text CRDT. `PageOp` gains `Text { artifact, op: TextOp }`, carrying `silt-crdt`'s operations as file text does. The first version's content is the starting text.
- **Blocks beyond Markdown:**
  - `<?silt chart {…}?>`, a chart drawn as in ADR 0076's data pages;
  - `<?silt mention user?>`;
  - `<?silt date 2026-11-15?>`.
  - Nothing in a doc runs code, and raw HTML is shown as text.
- **Editing:**
  - in the daemon's UI, with the WASM replica, live at typing speed on one machine and at sync speed between machines (ADR 0014);
  - in the hosted app, each edit is written by the server as the person (ADR 0026), at sync speed.
  - The editor is a rich-text view over the Markdown, with a source view.
- **Who may edit:** `comment` and up by default. The first version's declaration may set `collaborator`.
- **The agent:** `silt_artifact_doc { id, edits: [{old, new}] }` turns each edit into CRDT operations on the current text, so the agent's edits merge with what people type at the same time. Reading the doc returns its current text.
- **Versions are snapshots.** A version of a doc names a point in its history, a frontier, and keeps no blob. One is made:
  - when the agent finishes a turn that edited it;
  - when someone presses Save version, with a note (ADR 0073).
  - The version menu compares any two as a diff.
- **Comments:**
  - `CommentTarget` gains `Page { artifact, start, end }`, anchored with the CRDT's anchors, so a comment follows its text through edits, as on code (ADR 0066);
  - Send to agent works as for every artifact (ADR 0068).
- **Exports:** Markdown, HTML and the offline copy, plus PDF where a headless browser is installed. Silt doesn't make Word files.

**`sheet`: a grid with formulas.**
- **Rows and columns have stable ids.** Each axis of a sheet is a sequence in `silt-crdt`'s text CRDT, one character per row or column, whose character id is the line's id. `PageOp` gains `Lines { artifact, sheet, axis, op: TextOp }` for it. Inserting and removing rows never conflict, and two people adding a row at once both get theirs. Moving a row is a removal and an insertion that keeps its cells.
- **Cells:** `PageOp` gains `Cell { artifact, sheet, row, column, raw }`, where `raw` is the text typed. The last writer wins per cell, in the log's order.
- **References follow their cells.** A formula is stored with references to row and column ids and shown as addresses (`B7`). Inserting a row moves what the formula shows, not what it means.
- **Columns' widths and formats** are page data documents under `_sheet/<sheet>/`.
- **Formulas:** a new pure crate, `silt-sheet`, evaluates them, natively for the daemon, the server and agents, and as wasm in the browser, so every surface computes the same value.
  - **Values:** numbers, text, booleans and errors.
  - **Syntax:** cell and range references across sheets, and arithmetic and comparison operators.
  - **Functions:** `SUM`, `AVERAGE`, `MIN`, `MAX`, `COUNT`, `COUNTA`, `IF`, `IFS`, `AND`, `OR`, `NOT`, `ROUND`, `ABS`, `CONCAT`, `TEXT`, `LEFT`, `RIGHT`, `LEN`, `TODAY`, `DATE`, `VLOOKUP`, `XLOOKUP`, `SUMIF` and `COUNTIF`.
  - **Cycles** are an error, not a hang.
- **Limits:** at most 16 sheets, 10 000 rows and 200 columns each, and 100 000 non-empty cells per artifact.
- **The agent:** `silt_artifact_sheet { id, set: {"A1": "…"}, read: "A1:D20" }` writes raw cells and reads computed values.
- **Versions and comments** work as for a doc. A comment targets a range by its rows' and columns' ids, so it follows the cells when rows or columns move.
- **Exports:** CSV per sheet, and `.xlsx` through `rust_xlsxwriter`, formulas included.

**Both types:**
- **Forks and merges:** a fork inherits the doc or sheet as it was, and the two diverge (ADR 0072). A merge request shows a doc's diff, but merging doesn't carry it back.
- **Search:** Silt indexes a doc's text and a sheet's cell text with the thread, unlike other page data, since that's what people look for.

## Consequences
- **People can work on a plan or a tracker beside the agent,** in the same thread, live on one machine and at sync speed across machines.
- **Comments on a doc follow their text,** as on code.
- **`PageOp::Text`, `PageOp::Lines`, `PageOp::Cell` and `CommentTarget::Page` are new on the wire.** They ship with ADR 0072's protocol 13.
- **`silt-sheet` is a new pure crate,** built for wasm by the gate. `rust_xlsxwriter` is a new dependency of the shell crates.
- **A doc's history is every keystroke's operation,** like a file's. Its snapshots cost nothing.
- **The hosted app's editing goes through the server's writes,** so it's slower than the daemon's live replica.
