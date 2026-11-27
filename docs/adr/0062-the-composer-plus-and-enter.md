# 0062: The composer: + for what goes with a prompt, ⏎ to send, settings under the box

- Status: accepted
- Date: 2026-11-10
- Amends: [ADR 0044](0044-the-standard-agent-workspace.md) (what's in the box), [ADR 0061](0061-dictation-on-this-machine.md) (how a take shows, Send when I stop talking), [ADR 0059](0059-chats-without-a-project.md) (New chat's start button, which goes as New thread's does)
- Amended by: [ADR 0137](0137-enter-steers-mid-turn.md): ⏎ steers an agent that takes a prompt mid-turn, and the button beside + queues
- Amended by: [ADR 0063](0063-the-composer-grows-and-keeps-its-draft.md): The composer: stacked to start, in line in a thread, grows with its text, keeps its draft

## Context
Dictation's first version put a microphone and a ▾ menu between the agent's settings and the send arrow, and showed what it heard in a grey line under the text. In use, that read as crowded and unsettled:
- The words appeared in one place and landed in another, so the text moved when a take ended.
- Recording turned the microphone into the same square as stopping the agent, in amber beside the amber send arrow.
- Nothing showed that the microphone heard anything; a muted one was only found out once the take ended.
- The box's bottom row held the paperclip, the agent, mode, model, effort and permissions, the microphone and its menu, and the arrow.

## Decision
**The box holds the prompt; the agent's settings go under it.**
- In the box: + on the left, the text, and the microphone on the right. While a turn runs, a stop button sits beside the microphone, so a steer can still be dictated.
- + opens a menu: attach files, the agent's slash commands, and what New thread's start arrow used to offer, start in the background and save as a draft.
- Under the box, on the right: the agent, its mode, model, effort and permissions, folded into one chip where they don't fit. On the left: Steer now while a turn runs, what came of the last take, or, in a person's first three sessions, which key sends and which key talks.

**⏎ sends; there's no send arrow, except on a touch screen.** A touch keyboard's ⏎ starts a new line, so there a send arrow shows: on New thread always, on a thread once there's a draft or an agent to start.

**A take shows where its words will go.**
- The words heard show in grey at the caret, in the text itself, and stay there when the take ends. The caret becomes a level meter.
- The settings row under the box becomes the take's bar: ✕ discards (Esc), a waveform of what the microphone hears with the take's time, and ✓ keeps the words (or the microphone, pressed again). ⏎ keeps them and sends the draft.
- A microphone that hears exactly nothing for 1.5 s says so in the bar while the take runs.
- As before, the draft only gets the words once the take ends, so half-heard words never reach other viewers.

**Ctrl+M talks.** Held, it records until let go, and the words go in. Tapped, it starts a take that another tap ends. It's Ctrl on a Mac too, where ⌘M minimises the window.

**Send when I stop talking is gone.** ⏎ during a take does the same thing when the person wants it, without a setting to remember. The helper still reports pauses; nothing reads them.

## Consequences
- Every e2e test that pressed Send, Queue, Start or Start agent presses ⏎ in the box instead. The screen references change with the composer.
- The model offer is a small dialog over the microphone rather than a line in the box.
- A screen reader user reaches + and the microphone in the box and the agent's settings after it. The send action is ⏎, as the box's label and placeholder say.
