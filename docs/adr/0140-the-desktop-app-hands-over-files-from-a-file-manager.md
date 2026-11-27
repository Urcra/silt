# 0140: The desktop app hands over files from a file manager

- Status: accepted
- Date: 2026-11-27
- Builds on: [ADR 0041](0041-the-desktop-app.md), [ADR 0121](0121-videos-and-images-without-a-size-limit.md)

## Context
In the desktop app on Linux, a prompt took files only through +. Pasting a file copied in a file manager attached nothing, and neither did dropping one on the composer. A screenshot pasted from a screenshot tool worked.

- **Paste.** A file manager copies files as a `text/uri-list`, not their contents. WebKitGTK gives the page that type with its `file:` addresses removed, and no `File`s. A screenshot tool puts the image's pixels on the clipboard, which WebKitGTK does give as a file.
- **Drop.** Tauri takes native file drops for itself by default, so the web view never got the `drop` event. With Tauri's handler turned off, the page does get the event, but WebKitGTK treats the drop the same way as a paste: the `text/uri-list` type with its addresses removed, and no `File`s.

A page can't read a local path, and shouldn't be able to. The app can, so it has to hand the files over.

## Decision
**The page asks the window, and the window answers only after something the person did.**
- When a paste or a drop brings a `text/uri-list` and no files, and every address left in it is a `file:` one (a link dragged from a browser keeps its `https:` address and goes in as text), the composer asks the window with `silt://files/pasted` or `silt://files/dropped`. The link carries the window's key, so only its own page can ask, as with its other links.
- The window answers a paste only within 3 seconds of a Ctrl+V, Ctrl+Shift+V or Shift+Insert it saw itself. It answers a drop only within 3 seconds of a native drop on it. Each key press or drop answers one ask. A page can't make either one, so it can't read the clipboard or a past drop whenever it likes.
- The window takes the files itself: for a paste, the URIs on GTK's clipboard; for a drop, those GTK handed the web view during the drag. The page never names a path.

**Each file is served once, on the app's own scheme.**
- Each regular file is offered at `silt-files://localhost/<token>`, where the token is 128 random bits. It's served once, within 60 seconds, with `Access-Control-Allow-Origin: *`, since the page is the daemon's and on another origin. The unguessable token is what keeps the file to the page that was told it.
- The page fetches the files into `File`s and attaches them as if they had been picked with +. They upload as any attached file does, so this works on a machine over SSH too.
- A folder, an unreadable file, or files over 256 MiB in all (the page holds them in memory) are refused, each with its reason. The composer shows the reasons where it shows its other errors. A larger file is attached with +, which reads it from disk.

**Linux first.** The window tells the page `localFiles: true` only on Linux, where it watches GTK's keys, drops and clipboard. On macOS and Windows the page keeps its own paste and drop handling. Tauri's drop handler is off everywhere, so drops reach the page on every platform.

## Consequences
- Files pasted or dropped from a file manager attach in the desktop app on Linux. A screenshot pastes as before.
- Paste from a context menu isn't answered, as the window only trusts its own paste keys; the composer says to use Ctrl+V or Shift+Insert.
- Whether macOS's WKWebView and Windows' WebView2 give the page pasted and dropped files themselves is still to check there. If they don't, each gets its own native side behind the same links.
