# 0061: Dictation, on this machine

- Status: accepted
- Date: 2026-11-10
- Amended by: [ADR 0062](0062-the-composer-plus-and-enter.md) (the words at the caret, the take's bar, Ctrl+M; no Send when I stop talking)
- Amended by: [ADR 0079](0079-voice-in-a-page.md): an artifact's page can dictate through the holding page's take

## Context
People want to talk to the composer rather than type. Most of all in the Linux desktop app, where the webview has no speech recognition of its own. Three things had to be decided: which engine, where it runs, and what reaches the draft and when.

## Decision
**A streaming model, on this machine.** sherpa-onnx with a streaming Zipformer transducer for English (`sherpa-onnx-streaming-zipformer-en-2023-06-26`, int8, Apache-2.0), with its online punctuation model for case and punctuation.
- Whisper was the first choice, but it only produces text once a stretch of audio has ended. Words should show as they're said.
- The browser's own speech recognition sends the audio to the browser's vendor, isn't there in Firefox, and isn't there in WebKitGTK at all.
- A cloud dictation service would send audio off the machine and need an account.

**A separate program, `silt-voice`, in its own workspace.** The daemon runs it once per take, with audio on its stdin and JSON events on its stdout.
- The engine links a prebuilt native library, fetched at build time and built for glibc. Linux releases of `silt` are musl builds, and every other build would pay for it, so it stays out of the root workspace, as `silt-desktop` does (ADR 0041).
- On Windows it links the engine's DLLs rather than its static library. The static library needs the static CRT and the MSVC STL it was built with, which a slightly older Visual Studio lacks; the DLLs keep both behind a C API. They're installed beside `silt-voice.exe`.
- A crash in native code ends the take, not the daemon and the agents it runs.
- It is found as `$SILT_VOICE_BIN`, else beside the daemon's program, else on the `PATH`. `just install` installs it, and `just install-wsl` on Windows too; without it the button isn't shown.
- The models, about 103 MB to download and 80 MB on disk, go to `$SILT_HOME/models/` only when the person agrees to the download. Every file is checked against a pinned SHA-256, and the directory appears only once it's whole.

**The daemon passes audio through, and keeps none of it.**
- `GET /api/voice` says whether the helper and model are there, `POST /api/voice/model` starts the download, and `GET /api/voice/dictate` is a WebSocket: 16 kHz mono `f32le` frames in, events out (`partial`, `segment`, `paused`, `done`, `failed`).
- These routes don't become socket `Request`s, unlike the rest of the API (ADR 0011). Streamed audio has no CLI caller.
- One take at a time, five minutes at most. Closing the socket ends the helper. Audio stays in memory and pipes; it is never written to disk or synced.
- Only the daemon's UI offers it, so the hosted app has no button. `silt_proto::view` and `PROTOCOL` are unchanged.

**The draft gets words only once a take ends.**
- While talking, what's heard shows in a grey line under the text. Stopping puts it into the draft at the caret, set off by spaces. Esc throws the take away.
- Half-heard words so never reach other viewers through the draft's presence, and typing during a take isn't disturbed.
- **Send when I stop talking**, off by default, ends the take after about 2.5 s of silence following speech and sends it, as ⏎ would. A take with no words never sends.
- A take belongs to the composer it began in: moving to another thread drops it.

**The desktop app lets its own page use the microphone.** On Linux it turns WebKitGTK's media capture on, which wry leaves off. It allows the microphone only for the daemon's origin, and refuses the camera. WebKitGTK's audio needs GStreamer's good plugins (`gst-plugins-good`); without them the button says so.

## Consequences
- Dictation is English, on Linux and on Windows. Windows' `silt-voice` is built only by `just install-wsl` for now, not by releases or the gate. Other languages, macOS, and the hosted app and phones are future work.
- The streaming model is less accurate than a batch one. If that shows, a batch pass over the take when it ends can replace the preview without changing the API.
