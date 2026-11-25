# 0082: Comments inside docs and pages

- Status: accepted
- Date: 2026-11-15
- Amends: [ADR 0068](0068-artifacts.md) (comments on an artifact), [ADR 0072](0072-what-an-artifacts-page-can-do.md) (`comments`), [ADR 0077](0077-docs-and-sheets-edited-together.md) (comments on a doc)
- Amended by: [ADR 0085](0085-comments-on-a-pages-elements.md): Comments on a page's own elements

## Context
A comment on an artifact is a comment on the version's history entry, quoting what the person selected, and it goes to the agent as a prompt for a new version (ADR 0068). People also need comments anchored to the text or element they're about, and pages need to write comments as the viewer, which Silt then shows.

Two things were left open: ADR 0077 gave a doc's comments `CommentTarget::Page` anchors but nothing wrote them, and ADR 0072 gave a page only `openComposer`. It also wasn't settled whether comments on a page's content should be a second kind, replacing Silt's comments to the agent or sitting beside them.

## Decision
**One kind of comment.** A comment on a doc's text or one a page writes is a thread comment like any other. It has replies and is resolved as before, and Send to the agent works on it as on every comment. There's no second store of page comments: Silt's comments already are threads, and the agent is one of the people they're for.

**A doc's comments are on its text.** `CommentOn::Doc { artifact, start, end }` takes characters of the doc as it stands and anchors before the first and after the last, as a comment on code does (ADR 0066), so it follows its text through anyone's edits. `CommentLocation::Doc` says where the text is now and quotes it, or says it was deleted. The daemon and the hosted app place them alike, from the doc's text CRDT in `silt-proto`. A sheet's cells take no comments yet.

**A page writes comments as the viewer.** `comments: {}` serves `add`, `reply` and `resolve` beside `openComposer`, once the viewer agrees for that artifact, as `sample` asks. `add` comments on the version shown, quoting what the page passes. `reply` and `resolve` take only the ids of comments on this artifact. `comments: {composer_only: true}` serves `openComposer` alone and asks nothing. Pages never list comments; Silt shows them.

## Consequences
- A doc's comments show under it in both its views, and choosing one selects its text in the editor.
- A comment a page writes reaches the agent the way a person's does, so a page's control ("flag this row") can ask for a change.
- A page that declared `comments: {}` for `openComposer` alone now asks before its first write, and only then; it never asks if it never writes.
- **Not done:** anchors inside an HTML page's own elements, comments on a sheet's cells or a slide, and highlights in a doc's read view.
