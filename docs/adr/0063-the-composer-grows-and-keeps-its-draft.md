# 0063: The composer: stacked to start, in line in a thread, grows with its text, keeps its draft

- Status: accepted
- Date: 2026-11-13
- Amends: [ADR 0062](0062-the-composer-plus-and-enter.md) (where + and the microphone sit)

## Context
ADR 0062 put + on the left of the box, the text in the middle and the microphone on the right, in one row aligned to the bottom. In use:
- In a box several lines tall, the text began at the top, indented past +, while + and the microphone sat at the bottom. They looked misplaced.
- A long prompt scrolled inside a box a few lines tall.
- Leaving a thread, New thread or New chat lost what was typed in it.

## Decision
**Two layouts.**
- **Stacked**, on New thread and New chat: the text fills the box's width, over a row with + on the left and the microphone on the right, inside the box.
- **In line**, under a thread: the box is one line of text, with stop (or, on a touch screen, send) on its right. + and the microphone sit under the box on the left, and the agent's settings on the right. While a take runs, the microphone stays where it was pressed, before the take's bar, so pressing it again keeps the words.

**The box grows with its text** from its least height up to half the pane it's in, and scrolls past that.

**Each composer keeps its draft** by where it is: each thread's own, New thread's and New chat's. Leaving the screen and coming back finds the text as it was. Drafts are kept for the browser tab, through a reload, but not shared between tabs, which would each tell a thread's viewers a different draft. The command palette's "New thread: …" leaves its text as New thread's draft rather than in the address.

## Consequences
- The screen references change with the composer.
- A draft is sent or emptied by hand. One left in a thread stays there until then.
