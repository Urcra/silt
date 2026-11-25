# 0074: An artifact's files and assets: pages served from a frame address

- Status: accepted
- Date: 2026-11-15
- Amends: [ADR 0068](0068-artifacts.md) (one file per version, at most 10 MiB, rendered from `srcdoc`), [ADR 0071](0071-artifact-design-guidance-design-systems-and-libraries.md) (the frame's policy), [ADR 0072](0072-what-an-artifacts-page-can-do.md) (the capabilities)

## Context
An artifact version is one file of at most 10 MiB, rendered from `srcdoc`.
- **Everything is inlined.** A page's images, fonts, data and scripts must all be inside it, usually as base64 `data:` URIs, which add a third to their size.
- **Nothing can be fetched.** A page can't keep its data in a separate `data.json` and `fetch` it, because a `srcdoc` frame has nowhere to fetch from. A page that grows its stylesheet or its data has to resend all of it.
- **No uploads.** People using a page can't add a photo or a file to it.

A page needs supporting files beside it, served at relative paths, and an asset store people can upload to, with uploads never run as code.

## Decision
**Every HTML version is served from a frame address.**
- **The address:** the holding page asks `POST /api/threads/{id}/artifacts/{artifact}/frame` (`/app/api/…` on the server), with its own session, for a frame key. It's random, at least 128 bits, valid for an hour from when it was last used (so an open frame keeps loading its files), and bound to the thread, the artifact, the version and the viewer.
- **What's served there:**
  - `/f/<key>/` serves the composed page: the policy, the frame's first script, the design system's tokens, then the page;
  - `/f/<key>/<path>` serves the version's files;
  - `/a/<key>/<id>` serves the artifact's assets.
- **One composer for every surface:** `silt_design::frame` composes the page, as it already does for read-only links. The daemon, the server and links all use it.
- **The frame stays sealed:** it keeps `sandbox="allow-scripts"`, so its origin is still opaque, and the routes need no cookie or token because the key is the whole permission. A key serves nothing but its own version and its artifact's assets. It's refused once it expires, once the thread is deleted or purged, and once the viewer can't read the thread any more.
- **No more `srcdoc`.** Frames move off it for every HTML version, files or none, so there is one way in.

**A version can have files.**
- **The shape:** `silt_artifact` takes `files: {"<path>": {content} | {path} | null}` beside the page.
- **Carrying files over:**
  - files left out of a new version keep the previous version's;
  - `null` removes one;
  - `edits` (ADR 0073) can take `file` to edit one of them.
- **How they're kept:** each file is a blob attached beside the session, at `artifact/<id>/v<n>/files/<path>`. The record lists them as `files: [{path, blob, media_type, size}]`.
- **Paths:** relative, made of `[A-Za-z0-9._-]` segments joined by `/`, with no `..`, no leading `/` or `.`, and none of `f`, `a` or `_silt` as the first segment.
- **Allowed types:**
  - CSS and JavaScript;
  - JSON, CSV, text and Markdown;
  - SVG, PNG, JPEG, GIF, WebP and AVIF;
  - WOFF2, WOFF, TTF and OTF;
  - MP3, Ogg, MP4 and WebM.
- **Limits:** at most 255 files, 16 MiB each, and 64 MiB for a version with its page. The page alone goes from 10 MiB to 16 MiB.

**The frame's policy names its own address.**
- **What's added:**
  - `script-src` and `style-src` add `/f/<key>/`;
  - `connect-src` goes from `'none'` to `/f/<key>/` only, so `fetch('data.json')` works and nothing else connects;
  - `img-src`, `media-src` and `font-src` add `/f/<key>/` and `/a/<key>/`.
- **Assets never run:** `/a/` is never a script or style source.
- **Every response** carries `X-Content-Type-Options: nosniff` and `Cross-Origin-Resource-Policy: same-site`.
- **SVG files** are served with `Content-Security-Policy: sandbox`, so one opened on its own runs nothing.

**`assets`: what people add to a page.**
- **Declaring:** `capabilities: {assets: {}}`, or `{assets: {upload: "comment"}}` to let anyone who may comment upload. By default only `collaborator` and up may upload (the levels are ADR 0072's).
- **The functions:**
  - `assets.upload(blob)` resolves `{id, url, size, media_type}`;
  - `assets.list()` and `assets.delete(id)`.
  - Those below `collaborator` list and delete only their own.
- **Limits:** the allowed types are those of files, without CSS or JavaScript. An asset is at most 20 MiB, and an artifact holds at most 256 MiB of them; a server can lower that with `artifact_assets_mib` in `server.toml`.
- **What's recorded:** each asset is a document in the reserved `_assets` collection of the page's data (ADR 0072): its blob, name, type, size and uploader. The blob syncs as an attachment's does. A delete removes the document, and the blob goes at the next purge.
- **Agents:**
  - `silt_artifact_data` lists `_assets` like any collection;
  - `silt_artifact_read` with an asset id writes it to a file the agent can open.

**The offline copy:**
- **What's inlined:** files that the page and its CSS name, as `data:` URIs, plus a small `fetch` shim for files the page loads by script.
- **What isn't:** assets, unless the viewer ticks them in.
- **Too big for one file:** above 32 MiB the copy is a `.zip` with the page and its files beside it.

## Consequences
- **Pages can be built like small sites:** a stylesheet, a script, a data file and images, each revised on its own, at their real size.
- **A page's data can be fetched,** so a dashboard's numbers can be revised without rewriting its page.
- **People can add photos and files to a page,** and they go with the thread like everything else. They also sync like everything else: 256 MiB of uploads is 256 MiB on the server and on every machine that pulls the family.
- **Frames load from an address now,** so opening one is a request to the daemon or the server. A frame key that leaks gives an hour's read of one version and its assets.
- **The protocol doesn't change for files;** they're attachments and record fields. Assets live in `Op::Page`, which ADR 0072 adds in protocol 13.
- **A version made before this keeps rendering;** the new route serves it too.
