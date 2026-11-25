# 0079: Voice in a page: dictation and speech

- Status: accepted
- Date: 2026-11-15
- Amends: [ADR 0061](0061-dictation-on-this-machine.md) (dictation only in the composer), [ADR 0072](0072-what-an-artifacts-page-can-do.md) (the capabilities)

## Context
A page can't hear or speak. Its frame is refused the microphone, and audio plays only after a click. A page needs `dictation`, which turns speech into text, and `tts`, which reads text aloud.

Silt already dictates into the composer on the daemon's machine (ADR 0061). The holding page streams microphone audio to `GET /api/voice/dictate`, where `silt-voice` turns it into words, locally, with nothing kept. The holding page is also where the browser's own speech synthesis is available. The frame needs no microphone and no speaker of its own.

## Decision
**`dictation`: the holding page listens for the page.**
- **Declaring:** `capabilities: {dictation: {}}`. `silt.use("dictation")` is `null` where the composer's dictation isn't offered:
  - without `silt-voice` or its model;
  - in the hosted app, and so on phones;
  - on read-only links.
- **The functions:** `d.start()`, `d.stop()`, `d.onPartial`, `d.onFinal` and `d.onState`.
- **How a take runs:**
  - the frame's call reaches the holding page within the click that made it. The click inside the frame counts as the viewer's activation in the holding page too, so the holding page can open the microphone;
  - the holding page runs a take as the composer does, over the same WebSocket and the same one-take-at-a-time rule, and forwards `partial`, `segment` and `done` events to the frame as text;
  - the frame never receives audio.
- **What's shown:** a take shows the composer's listening indicator in the artifact's header, with a Stop button outside the frame. A page can't listen without the viewer seeing it.
- **Consent:** the first take asks the viewer once per artifact (ADR 0072's grants). Leaving the artifact ends the take.

**`tts`: the holding page speaks for the page.**
- **Declaring:** `capabilities: {tts: {}}`.
- **The voice:** the holding page uses the browser's speech synthesis (`speechSynthesis`), with the system's voices. It needs no model and no daemon, so it works in the hosted app and on phones too. It's `null` where the browser has no voices, as on some WebKitGTK builds.
- **The functions:**
  - `t.speak(text, {voice, rate, language})`, with plain text of at most 2,000 characters and a rate from 0.7 to 1.5;
  - `t.speak([{text, voice}, …])` for a reading of several lines;
  - `t.add`, `t.stop`, `t.pause` and `t.resume`;
  - `t.onState` and `t.onLine`;
  - `t.voices()`, which lists the system's voices by name and language.
- **Constraints:**
  - speaking starts only after the viewer has clicked or typed in the page;
  - it stops when the artifact closes;
  - it's refused with `busy` during a dictation take;
  - a Stop control shows in the header while it speaks.
- **No consent prompt.** Speaking is as visible as sound, and the viewer can turn it off for the artifact in its ⋯ menu.

**What `silt-voice` gains:** nothing for now. A local voice through sherpa-onnx's speech synthesis would match dictation's privacy and sound the same everywhere, at the cost of another model download. It's left for when the browser's voices prove too poor.

## Consequences
- **Pages can take spoken input and read aloud:** a language drill, a checklist read out, a form filled by voice.
- **Dictation stays local and on the daemon's machine,** as ADR 0061 decided. Pages in the hosted app type instead.
- **Speech sounds like the viewer's own system,** so it differs between machines and browsers.
- **The microphone is never the frame's,** and it shows in Silt's own header whenever it's open.
