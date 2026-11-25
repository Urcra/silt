# 0085: Comments on a page's own elements

- Status: accepted
- Date: 2026-11-16
- Amends: [ADR 0082](0082-comments-inside-docs-and-pages.md) (what a page's comments are on)

## Context

ADR 0082 let a page write comments as the viewer, each on the version shown, quoting what the page passed. It left out a comment on one element of the page, which the viewer can go back to, and names of the page's own for things it draws itself. A comment on "the total row" said so only in its quote, and nothing took the viewer there.

## Decision

**A comment can be on an element.** `CommentTarget::Element { artifact, anchor }` keeps the artifact and an anchor, a line of at most 300 characters. It's the last variant, as stored deltas name variants by position, and it goes out in protocol 15 with the sheet cell's.

**The page names the anchor.** `comments.anchorFor(element)` gives the element's `data-comment-anchor` (or an ancestor's), else its `#id`, else a CSS path. `openComposer({element})` and `add({element})` take one; a page that draws what's commented passes a name of its own as `anchor`. Silt doesn't check that an anchor names anything: it's the page's, as the quote is.

**Showing a comment's element is the viewer's.** The comments list says what each is on; choosing it sends the frame the anchor. The frame scrolls to the element and outlines it, or, for a page that called `customAnchors(onReveal)`, hands the anchor to the page instead.

**An element comment is on the artifact, not a version.** It's shown on every version and sent to the agent with its anchor, so the agent's next version can keep the anchor on what it names.

## Consequences

- Pages that give commented things a `data-comment-anchor` keep their comments' places across versions; a CSS path breaks when the structure changes, and the skill says so.
- Silt draws no pins over the page: the list, and the outline when asked, show where comments are.
- A peer may write any anchor. It's only ever a selector the frame tries inside its own sealed document, or a name handed to the page.
