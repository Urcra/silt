# 0013: Accounts, orgs, inbox, thread lifecycle and self-hosting

- Status: accepted
- Date: 2026-10-20
- Amended by: [ADR 0027](0027-read-only-links.md): Read-only links
- Amended by: [ADR 0028](0028-notifications-and-phones.md): Notifications and phones
- Amended by: [ADR 0032](0032-several-servers-and-local-projects.md): Several servers per daemon, and projects kept on this machine

## Context
Until M7 the server knew users only by name. A device joined with a one-time pairing code that the admin made on the server host, and "team" visibility meant everyone on the server. That is enough for one person or a trusted few. It is not enough for a company or an open-source group hosting Silt for itself: they need real accounts and single sign-on, orgs whose members see each other's work, notifications for review work, a way to remove threads, and a server that is easy to set up, run and back up.

## Decision
**Configuration.** The server reads `server.toml` in its data dir, written by `silt server init`. It holds:
- the listen address and the public URL;
- the sign-up policy: `invite`, `open`, or `domains = [...]`;
- the sign-in providers: local passwords, GitHub, and OIDC;
- how long deleted threads are kept before they are purged.

Secrets may be given inline or as `*_file` paths. `silt server run` uses the config; its flags override it.

**Accounts.**
- **Users.** A user has a name, optional display name and email, an optional password, and an `admin` flag.
  - Passwords are hashed with Argon2id.
  - External identities live in `identities(provider, subject, user)`.
  - `silt server init` creates the first admin. `silt server user add|list|passwd|admin|disable|sign-out` manage users on the host. `silt server invite` makes a sign-up link.
  - *Amended 2026-10-20:* a disabled user can't sign in, and disabling signs them out everywhere. A device token lapses after 90 days without use. The home page lists your devices to sign out one by one, and signs out your other browsers.
- **GitHub and OIDC.** Both use the authorization code flow with `state` and PKCE.
  - **GitHub** reads the user from `GET /user`. Its web and API URLs are configurable, for GitHub Enterprise and for tests.
  - **OIDC** reads its endpoints from the issuer's discovery document, and the user from the userinfo endpoint. The token is fetched straight from the token endpoint over TLS, so the ID token's signature is not needed and not checked.
  - **Account matching.** A first sign-in links an identity to the account with the same verified email, else to the invite it carries. Otherwise it creates an account when the sign-up policy allows.
- **Browser sessions.** Server-rendered pages (`/login`, `/signup`, `/device`) sign people in to a session cookie. The cookie is HttpOnly and SameSite=Lax, and its token is stored hashed.
- **Devices.** `silt auth login --server <url>` uses a device authorization flow:
  1. The CLI gets a device code and a short user code.
  2. It opens `<url>/device?code=…`.
  3. The person signs in there in any way the server allows, and approves.
  4. The CLI's poll returns a device token.
  - `--user <name>` signs in with a password instead, for headless machines. Pairing codes keep working for scripted setups.
  - `silt auth logout` revokes the device.

**Orgs and teams.**
- **Membership.** An org has members, who are owners or members, and teams.
- **Projects** (repositories by normalized remote) belong to one org. A project registered by a member of exactly one org joins that org; otherwise it joins the member's first org until an org owner moves it with `silt project move`.
- **Visibility** is one of:
  - `private`: the owner, plus work offered through merge requests;
  - `team:<name>`: members of that team of the project's org;
  - `org`: every member of the org.
- **Defaults.** A thread's own setting wins; then a fork's source's; then the project's; then the org's `default_visibility`, which defaults to `org`. Users outside the project's org see nothing of it.
- **Migration.** Existing data moves into one org, `default`, that every existing user joins, and the old `team` setting becomes `org`.
- **Rights.** Only org owners and server admins change org membership, teams, project placement and org defaults.

**Inbox.**
- **What notifies.** The server writes a notification when a delta it stores is one of these:
  - a merge request opened on your thread;
  - a merge request of yours approved or declined;
  - a comment on your thread;
  - an `@name` mention in a comment, merge request title or summary, from someone who can see the thread.
- Notifications come only from new pushes, never from rebuilding the index.
- **Reading.** `silt inbox` lists them and `silt inbox read` marks them read; the web UI has an inbox page; the agent's next prompt hears about new merge requests on its thread.

**Deleting threads.**
- **Delete.** `silt thread delete` moves a thread to the trash. Only its owner can, on the server and on every device. A deleted thread disappears from listings, search and in-flight for everyone, and `silt thread restore` brings it back.
- **Purge.** `silt thread purge`, or the retention period running out (30 days by default), removes a trashed thread for good:
  - its own deltas that no remaining thread builds on;
  - its metadata, index rows and notifications;
  - blobs nothing references any more;
  - the family's git mirror once no thread of the family is left.
- **No resurrection.** The server keeps the purged thread ids, drops any of their deltas that are pushed again, and lists them. Every daemon then deletes them too before it syncs. Positions in the family's log change, so clients re-sync it from the start; that costs bandwidth once and loses nothing.

**Operations.**
- **Backups.** `silt server backup <dir>` works while the server runs. It writes the database with `VACUUM INTO` first, then copies the blob store, the git mirrors and the config. Blobs and mirror objects are only ever added, so everything the database snapshot refers to is in the copy. `silt server restore <dir>` puts a backup in place on a stopped server; clients that are ahead push back what the server lost (ADR 0009).
- **Running the server.** `packaging/` holds:
  - a system unit for the server, running under its own user with a hardened sandbox;
  - a user unit for the daemon;
  - a Caddyfile for TLS.
  `silt server init --systemd` writes the server's unit.

**Documentation as a test.** docs/self-hosting.md is the install guide.
- `scripts/selfhost-walkthrough.sh` runs its commands on fresh containers: one server host and two client machines, each with only the binary and git.
- On them, two users sign up, share a project, fork each other's thread and merge it back through a merge request.
- It fails if any step fails, or if the run takes longer than 10 minutes.

## Consequences
- **New dependencies:** `argon2`, `toml` and `reqwest` on the server, plus server-rendered HTML pages. The pages are deliberately plain; the full web app stays in the daemon.
- **The sync protocol changes** (`PROTOCOL` 2), because visibility gained levels and listings carry purged threads. Old clients get the protocol-mismatch message.
- **Purge is permanent.** A fork that kept deltas of a purged thread keeps exactly those, and nothing else of the thread.
- Pairing codes stay as the simplest path for a single person and for scripted setups.
