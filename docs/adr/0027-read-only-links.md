# 0027: Read-only links

- Status: accepted
- Date: 2026-10-21
- Amends: [ADR 0013](0013-accounts-orgs-and-self-hosting.md), where only accounts the visibility rules admit read a thread.
- Amended by: [ADR 0072](0072-what-an-artifacts-page-can-do.md): a link's page reads its artifact's page data, frozen where the link is; every other capability is `null` there
- Amended by: [ADR 0068](0068-artifacts.md): Artifacts: what an agent shows the person, beside the thread

## Context
Showing one thread to someone outside the org takes an account today: an invite, a membership, and a visibility that lets them in. Often the need is smaller: send a reviewer, a client or a colleague in another org how an agent reached a change, once. The plan for the web UI asks for a link that opens one thread read-only for anyone holding it.

A link widens who reads a thread, so it needs the same care as visibility. It must not reach other threads in the family, the code beyond what the conversation says, or anything that can't be taken back.

## Decision
**A link is a token for one thread.**
- **Who makes one:** the thread's owner, from the Share dialog or `silt thread link <thread>`.
- **Expiry:** a link always expires: in a day, a week or 30 days, chosen when it's made. It can't be extended; make another instead.
- **What it shows:** either
  - **following:** the thread as it is when the link is opened, new turns included; or
  - **frozen:** the thread as it was when the link was made, fixed at the last delta then.
- **Revoking:** the owner lists the thread's links and revokes any, at once. Deleting or purging the thread ends its links, and so does making it private.
- **Storage:** tokens are random 32 bytes, kept hashed like session tokens. A link's URL is `<public url>/r/<token>`.

**What a link opens.**
- **A page the server renders,** not the app: the thread's title, who owns it, and its own conversation, with turn markers. A short note says when the conversation came from a fork. A frozen link also says that it's frozen and when it was made.
- **Left out:** comments, inherited conversation from before a fork, other threads of the family, files and diffs, presence, and every API. Holding a link gives no session and reaches no `/app/api/` or `/v1/` route.
- **Secrets:** the page shows what the server stores, which is redacted at capture ([ADR 0019](0019-redact-to-gits-threat-model.md)), as every other view is.
- **Headers:** `Cache-Control: no-store`, `Referrer-Policy: no-referrer`, and `X-Robots-Tag: noindex`. Failed tokens are rate-limited like password attempts.

**Orgs decide whether links exist.**
- An org setting, `links`, on by default, lets owners turn links off for the org's projects.
- Turning it off stops every existing link of those projects too. Turning it on again doesn't bring them back: they stay stopped until they expire.

## Consequences
- New table `thread_links` (token hash, thread, owner, created, expires, frozen-at delta, revoked), and an org column for the setting, each with a migration.
- Anyone a link is forwarded to reads the conversation until it expires or is revoked. The Share dialog says so when the link is made, and lists the links with their expiry.
- The read page duplicates a small part of the app's conversation rendering on the server, kept to plain text and turn markers.
- Read-only links are the only way into a thread without an account. Writing, commenting and reviewing still need one.
