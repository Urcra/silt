# 0028: Notifications and phones

- Status: accepted
- Date: 2026-10-21
- Amends: [ADR 0013](0013-accounts-orgs-and-self-hosting.md) (what notifies) and [ADR 0026](0026-hosted-mode-and-remote-control.md) (who may send commands).
- Amended by: [ADR 0112](0112-phones-on-the-local-network.md): Phones on the local network

## Context
The inbox only hears what's in the log: merge requests, comments, mentions. What most needs the person happens on their machines instead: an agent waiting on a permission, a plan to approve, a turn finished or failed. Since ADR 0026, the server hears about those in each daemon's running-agent reports. The plan for the web UI wants them in the inbox, as notifications in the browser and on a phone, and a phone that can answer them.

## Decision
**New inbox kinds, written by the server from what devices report.**
- **From reports:** a report that changes an agent's state writes one notification for the person whose device runs it:
  - `permission`: the agent waits on a permission (`awaiting_permission`);
  - `plan`: it wrote a plan to approve (`awaiting_input`);
  - `finished`: a turn ended and the agent waits for the next prompt (`working` to `idle`) with nothing queued;
  - `errored`: it stopped with an error;
  - `offline`: the device went offline while an agent there was working or waiting.
- **Recovery:** a daemon that makes a recovery thread says so when it registers the thread, and the person hears `recovered`.
- **Deduplication:** an unread notification of the same kind and thread is brought up to date rather than repeated.
- **Inline actions:** a `permission` notification carries the request's id and options, and a `plan` notification can be approved. The inbox answers them through the relayed commands of ADR 0026, with the same checks.

**Per-person settings, kept on the server.** For each kind: show it in the inbox only, or also notify. `permission`, `plan`, `errored` and `offline` notify by default, while `finished`, `recovered` and the older kinds don't.

**Notifying.**
- **In an open tab:** the app, served by a daemon or by the server, shows a browser notification for each new unread item of a notifying kind while the tab isn't focused, after the person allows it.
- **Web Push, for the hosted app:**
  - The server has a VAPID key pair. `silt server init` makes it, or the first start without one does.
  - A browser that allows notifications registers a push subscription with the server, and the server pushes each new notifying item to the person's subscriptions.
  - The payload is the notification's title, text and link, encrypted as RFC 8291 says. It holds no more than the inbox shows.
  - A subscription the push service refuses as gone is dropped.
  - Only the server's origin can push. A daemon's page notifies only while it's open.

**Phones.**
- **The app is a PWA:** a manifest, icons and a service worker at `/app/`, and phone layouts of the same screens.
- **Pairing:** a signed-in device or browser shows a QR code and a short code from the Devices page. A phone opening it is signed in to the hosted app as a browser session, without a password. The code is used once and lasts 10 minutes.
- **Capabilities:** a paired phone's session has a phone's capabilities by default: answer permissions and review. Sessions signed in with a password keep every capability.
  - Sessions with capabilities are listed on the Devices page beside devices, where they can be renamed, changed and signed out.
  - A command keeps the capabilities it was sent with, fixed when it's relayed, and the daemon checks those.

## Consequences
- **New tables:** notification settings, push subscriptions, and capabilities and names on web sessions, plus a capabilities column on relayed commands, each with a migration. The VAPID keys live in the server's data directory and its backups.
- **Report history:** the server keeps the previous report per device to tell transitions, still in memory. A restart forgets them, so the first report afterwards notifies nothing.
- **Push services:** the person's browser vendor's service relays pushes, so the server needs outbound HTTPS to it. Self-hosters without that still get the inbox and open-tab notifications.
- **iOS:** pushes arrive only for the app installed to the home screen, as Safari allows.
