# 0121: Videos, and images without a size limit

- Status: accepted
- Date: 2026-11-24
- Amends: [ADR 0068](0068-artifacts.md) (what agents show), [ADR 0009](0009-sync-and-handoff.md) (blobs on the server)

## Context
Agents show the person images with `silt_show`, and people attach images to prompts. Nothing showed a video: not an agent's recording of a bug, not a video a person attached, not a video file in the repository. Images were capped at 10 MiB as artifacts and at 10 MiB each, 25 MiB together, as attachments. They opened at one size in a small dialog, with no zoom.

The caps came from how files moved:
- A blob went between a machine and the server whole, as one HTTP body, held in memory at both ends.
- The web UI read an artifact's content as base64 inside JSON.
- Nothing answered a byte range.

A 400 MB screen recording can't go that way. Even a 20 MB screenshot is a poor thing to send to a phone to draw a 420 px thumbnail.

## Decision
**Videos.**
- `silt_show` and `silt_artifact` (`kind: "video"`) take MP4, MOV, WebM and Matroska.
- Prompts take video attachments. The agent gets a video's path, since no agent takes video in a prompt.
- A thread's file page plays a video file where it said "A binary file".
- Silt keeps a video as it was given. It makes no copy at another size and has no ffmpeg; the browser plays what it can and offers Download for the rest.

**No size limit.**
- Media an agent shows, and files attached to a prompt, have no limit.
- A server may set one, `max_file_mib` in `server.toml`; it has none by default. A file over it stays on the machine that made it, and the thread syncs without it.
- The server keeps a note of each blob's other facts: the device whose push first named it, why it refused it, and whether its last upload arrived damaged. A blob's state carries them, so every machine says where a file is coming from, why it won't come, or that it's coming again.
- Text artifacts keep their 10 MiB limit, and a page's files and assets keep theirs (ADR 0074).

**Blobs move in chunks.**
- A blob larger than 8 MiB goes to the server in 8 MiB chunks: `PUT /v1/blobs/{hash}/chunks?offset=&size=`.
- `GET /v1/blobs/{hash}/state` says how much of it the server has, so a dropped connection resumes from there.
- The server keeps a partial upload per account, checks the whole against its BLAKE3 hash, then moves it into place.
- Smaller blobs go whole as before.
- Nothing holds a large blob in memory: the blob store writes from a file or a stream and reads a range.

**Large attachments go after the conversation.**
- A blob over 8 MiB named only by a conversation attachment (a shown video, a prompt's file, an image's original) is uploaded after the deltas that name it, by a background upload that a restart resumes.
- Other machines see the message at once and the file once it arrives. Its state says how much has arrived.
- A file's content in the tree is still uploaded before its delta, as checkouts need it.

**Ranges.**
- The daemon (`/api/threads/{id}/blobs/{hash}`) and the server (`/app/api/threads/{id}/blobs/{hash}`) serve a thread's blobs raw, with `Range`, as immutable, under a sandboxing CSP and `nosniff`.
- The server's `/v1/blobs/{hash}` answers ranges too. A daemon without the blob passes the browser's range on to the server rather than fetching it whole.
- `<video>` seeks by range, so a long recording starts playing after a few hundred KB.

**Previews of images.**
- A machine keeping a raster image larger than 960 px makes a 960 px preview of it, and one of 2048 px when it is larger than that. They are JPEG, or lossless WebP where the image has transparency.
- They are named in the version (`ArtifactView.previews`) or the prompt's image (`ImageView.stored`), and kept beside it as attachments, so they sync with the conversation.
- The conversation shows the 960 px preview. The viewer opens on the 2048 px one, and loads the original when asked.
- A file page's image gets its previews made on demand and cached (`?preview=960|2048`).
- GIFs and SVGs get no previews, so animations and vectors stay as they are.
- An image attached to a prompt goes to the agent as its 2048 px preview when the original is larger, as models take only so much.

**The viewer.**
- Images and videos open in one full-window viewer.
- Images zoom with scroll and pinch, pan by dragging, and have keys and a navigator.
- Videos have Silt's own controls, the same on every surface.

**The protocol version goes to 24.**

## Consequences
- Every machine and the server update together, as for any protocol change.
- One person can fill a server's disk unless its owner sets `max_file_mib`.
- A large file another machine made can't be seen until that machine uploads it. The web UI says so and shows how much has arrived.
- Previews cost the machine that keeps an image some time and memory once. An image too large to decode within the decoder's limits gets no preview, and the viewer offers the original.
- Prompt lines and artifact versions name blobs rather than carrying base64 images, so a long thread's history stays small. Older lines with inline images read as before.
