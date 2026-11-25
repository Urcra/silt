# Self-hosting Silt

One server lets a team hand threads between machines, fork each other's work and review it through merge requests. The server is a single binary with a SQLite database, a blob store and git mirrors on disk. Going from a fresh Linux machine to two people exchanging a fork and a merge request takes about ten minutes; `scripts/selfhost-walkthrough.sh` checks that by following this guide on fresh containers.

## What you need
- **The server:** a Linux machine with `git`, reachable over HTTPS by your team's machines. 1 CPU and 1 GB of RAM are plenty for a small team; disk grows with your threads.
- **Each person's machine:** Linux or WSL with `git` and the agent CLI they use: Claude Code, Codex, Qudiks, Gemini CLI, Antigravity CLI, OpenCode or GitHub Copilot CLI.
- **The same Silt version everywhere.** Server and devices check that they speak the same protocol.

## 1. Install Silt
Silt is one binary, `silt`, on the server and on every machine. Until there are published releases, build it with Rust 1.99 or newer:

```sh install
cargo install --locked --git https://github.com/Urcra/silt silt-cli
```

Or copy a `silt` you built elsewhere to `/usr/local/bin/silt`. The server also needs `git`, for the git mirror:

```sh install
sudo apt-get update && sudo apt-get install -y git
```

## 2. Set up the server
Create the server's config and its first admin. Use the URL your team will reach it at. The server listens on this machine only, behind a proxy that answers HTTPS: Caddy does it in two lines with `packaging/Caddyfile`, answering on `https://silt.example.com` and forwarding to port 7450.

```sh server
silt server init --data /var/lib/silt --listen 127.0.0.1:7450 --public-url https://silt.example.com
```

This writes `/var/lib/silt/server.toml` and prints the admin's password, once.

To try Silt on a private network without a proxy, listen on every interface instead. Passwords and tokens then cross the network unencrypted, so never do this on a network you don't control:

```sh server
silt server init --data /var/lib/silt --listen 0.0.0.0:7450 --public-url https://silt.example.com
```

### Run it
As a systemd service, under its own user. The unit keeps the service out of home directories, so `silt` has to be installed outside them, in `/usr/local/bin` for instance; `init` says so if it isn't, and gives the data directory to the service's user.

```sh systemd
sudo useradd --system --home-dir /var/lib/silt --shell /usr/sbin/nologin silt
sudo silt server init --data /var/lib/silt --listen 127.0.0.1:7450 --public-url https://silt.example.com --systemd /etc/systemd/system/silt-server.service --force
sudo chown -R silt:silt /var/lib/silt
sudo systemctl daemon-reload && sudo systemctl enable --now silt-server
```

Or straight from a shell, to try it out:

```sh server
silt server run --data /var/lib/silt &
```

## 3. Add your team
Give each person an account. The command prints a password to hand over:

```sh server
silt server user add alice --data /var/lib/silt
silt server user add bob --data /var/lib/silt
```

Or send each person a sign-up link, where they choose their own name and password, or use GitHub or your company's sign-in (see below):

```sh server
silt server invite --data /var/lib/silt
```

When someone loses a laptop, `silt server user sign-out alice` signs them out of every device and browser; people can also sign single devices out from the server's home page. `silt server user disable alice` stops someone signing in at all (`--enable` undoes it). A device that goes unused for 90 days has to sign in again.

## 4. Sign in from each machine
On Alice's machine:

```sh alice
silt auth login --server https://silt.example.com --user alice
```

On Bob's machine:

```sh bob
silt auth login --server https://silt.example.com --user bob
```

Without `--user`, `silt auth login --server <url>` opens the server's page in a browser. You sign in there however the server allows and approve the machine, so passwords never touch the terminal.

## 5. Work together
Everyone works in their own clone of the team's repository. Alice starts a session; Silt records it, code and conversation:

```sh alice
cd ~/src/app
silt claude
```

Bob sees Alice's thread, forks it and works on it in his own agent session, then offers the work back. `silt fork` names the fork it made:

```sh bob
cd ~/src/app
silt log
silt fork <thread>
silt mr create --thread <fork> --title "Add Bob's change"
```

Alice hears about it, reviews it and merges it. Her checkout gets Bob's change, and her agent hears about it on its next prompt:

```sh alice
silt inbox
silt mr show <mr>
silt mr approve <mr>
```

That's the whole loop. `silt resume <thread>` continues one of your own threads on another machine. `silt open` shows threads, history and merge requests in a browser.

To edit a thread together in real time, or to start agents from the web UI, see [live-and-agents.md](guide/live-and-agents.md).

## Sign-in with GitHub or your identity provider
Add a provider to `server.toml` and restart the server. With GitHub, create an OAuth app whose callback URL is `https://silt.example.com/auth/github/callback`:

```toml
[auth.github]
client_id = "Iv1.0123456789"
client_secret_file = "/var/lib/silt/github-secret"
```

Any OpenID Connect provider works the same way, with the callback `https://silt.example.com/auth/oidc/callback`:

```toml
[auth.oidc]
name = "Example SSO"
issuer = "https://sso.example.com"
client_id = "silt"
client_secret_file = "/var/lib/silt/oidc-secret"
```

Who may create an account on first sign-in is set under `[signup]`:
- `policy = "invite"` (the default): only people with an invite link.
- `policy = "open"`: anyone who can sign in.
- `policy = "domains"` with `domains = ["example.com"]`: people whose verified email is at those domains, plus invites.

Set `passwords = false` under `[auth]` to allow only the providers.

## Comments on pull requests
For projects that turn it on with `silt project set --forge-comments on`, the server keeps one comment on each thread's open pull request ([Git, pull requests and the Agent SDK](guide/git-and-forges.md)). Give it a token for each forge in `server.toml` and restart it:

```toml
[[forges]]
host = "github.com"
kind = "github"
token_file = "/var/lib/silt/github-token"

[[forges]]
host = "gitlab.example.com"
kind = "gitlab"
token_file = "/var/lib/silt/gitlab-token"

[[forges]]
host = "codeberg.org"
kind = "forgejo"
api_url = "https://codeberg.org/api/v1"
token_file = "/var/lib/silt/codeberg-token"
```

- **`host`** is how the projects name the forge, as their remotes do.
- **`kind`** is `github`, `gitlab` or `forgejo`.
- **`api_url`** is needed only when the API isn't where the forge usually puts it: `https://api.github.com` for github.com, else `/api/v3` (GitHub Enterprise), `/api/v4` (GitLab) or `/api/v1` (Forgejo) on the host.
- **The token** needs to read pull requests and write comments on them. `token` takes it inline instead of from a file.

A forge that's down, or a token without the rights, only shows in the server's log.

## Orgs, teams and who sees what
Everyone starts in the org `default`, and everyone in an org sees its projects' threads. To give a group its own space:

```sh
silt org create acme
silt org add acme alice --owner
silt team create acme core
silt team add acme core alice
silt project move acme
silt org default acme team:core
silt visibility private --thread <thread>
```

In order:
1. `silt org create` makes the org (server admins only).
2. `silt org add` adds Alice as an owner.
3. `silt team create` and `silt team add` make the team `core` and put Alice in it.
4. `silt project move` moves the project of the clone you're in into the org.
5. `silt org default` makes its threads visible to the team by default.
6. `silt visibility` keeps one thread to yourself.

A fork without its own setting follows its source. A private thread reaches others only through the merge requests it offers. `silt fork --private <thread>` makes a fork that is private from the start; making a fork private afterwards leaves a moment in which others can pull it.

## Artifacts' libraries and fonts

An HTML artifact may load scripts from a few public CDNs and fonts from Google Fonts, so by default a person opening one, or a read-only link to one, contacts those hosts. To keep viewers off them, set this in `server.toml` and restart the server:

```toml
artifact_proxy = true
```

The server then fetches each file once, only pinned versions of the allowlisted CDNs, keeps it under `vendor/` in its data folder (at most 512 MiB), and serves it to the hosted app's and read-only links' frames from `/vendor/`. Each machine has the same choice for its own web UI, in Settings › Appearance ([ADR 0071](adr/0071-artifact-design-guidance-design-systems-and-libraries.md)).

## Artifacts' files

The files people add to one artifact's page, such as a whiteboard's images, take at most 256 MiB on the server. Change it in `server.toml`:

```toml
artifact_assets_mib = 512
```

## Large files

Images and videos agents show, and files attached to prompts, have no size limit ([ADR 0121](adr/0121-videos-and-images-without-a-size-limit.md)). A file over 8 MiB arrives in chunks after the messages that name it. To cap what the server keeps, set the largest file it takes, in MiB, in `server.toml`:

```toml
max_file_mib = 2048
```

A larger file stays on the machine that made it, and the rest of the thread syncs without it. That machine shows it with a note that the server won't keep it; other machines say which machine has it and why.

## Deleting threads
`silt thread delete <thread>` moves one of your threads to the trash on every device, and `silt thread restore <thread>` brings it back. `silt thread purge <thread>` removes it for good. Threads left in the trash are purged after `trash_days` (30 by default). Forks keep what they built on.

## Backups
Back up while the server runs, for example nightly from cron:

```sh
silt server backup /var/backups/silt/$(date +%F) --data /var/lib/silt
```

To restore, stop the server and run:

```sh
silt server restore /var/backups/silt/2026-10-20 --data /var/lib/silt --replace
```

Devices that synced after the backup push back what the server lost. Any machine whose own data was restored from an older backup moves its newer work into a recovery fork.

A backup includes `vapid.key`, the key the server signs browser pushes with. Browsers subscribed against it, so keep it: a server with a new key reaches none of them until each allows notifications again.

## Notifications on phones and browsers
People who use the web app at `https://silt.example.com/app/` can allow notifications from its inbox page. The server then sends what notifies them as Web Push, through their browser vendor's push service, so it needs outbound HTTPS to those services. Without that, they still have the inbox, and notifications while a Silt tab is open.

## Upgrading
Replace the binary on the server and on every machine, then restart the server (`sudo systemctl restart silt-server`). Each machine's daemon restarts with its next `silt` command, or with `silt daemon restart`, once agents the web UI started finish their turns.

## Troubleshooting
- **"the server speaks protocol …"**: the machine and the server run different Silt versions.
- **"not signed in, or the sign-in was revoked"**: run `silt auth login` again. `silt auth logout` signs a machine out.
- **The server's log:** `journalctl -u silt-server`, or the terminal it runs in. Set `SILT_LOG=debug` for more.
