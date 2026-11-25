# Security

## Reporting a vulnerability
Please report vulnerabilities privately, through GitHub: **Security → Report a vulnerability** on this repository. Don't open a public issue for one.

Say what an attacker can do, from where (another person on your server, a hostile peer's deltas, the server itself, a web page, an agent, a plugin), and how to reproduce it. You'll get an answer within a week. A fix is released together with an advisory that credits you, unless you'd rather not be named.

## Supported versions
Silt has no releases yet. Fixes land on `main`; build from it to get them.

## How Silt is meant to be used
Silt records coding agents' sessions and syncs them through a server you run. Its threat model:
- **Other people's work is untrusted.** Deltas from peers may be hostile. Silt checks them before they touch disk or become file names or arguments ([ADR 0016](docs/adr/0016-untrusted-peer-content.md)).
- **What leaves your machine follows git.** Files sync as `git` would commit them. Secret-looking values in conversations are replaced before they're pushed ([ADR 0019](docs/adr/0019-redact-to-gits-threat-model.md)).
- **Agents are eager, not hostile.** Silt's sandbox keeps an agent that tries too hard away from what it shouldn't reach. It isn't built to contain an agent written to escape ([Silt's sandbox](docs/guide/sandbox.md)).

## Known limitations
These are known and accepted for now. Reports that find a way around what they describe are welcome.

- **Your server is trusted with your machines.** A server you've signed a machine in to can start and steer agents there on your behalf, for threads that sync with it. A machine checks that the command comes from its own person, from a device allowed to send it, but it takes the server's word for both. Run your server yourself, and sign machines in only to servers you trust.
- **Redaction is a safety net, not a guarantee.** It covers:
  - values of environment variables whose names look secret;
  - values in the data directory's `secrets` file;
  - these token formats: `github_pat_`, `ghp_`, `gho_`, `ghs_`, `sk-ant-`, `sk-`, `xoxb-`, `xoxp-`, `AKIA`.

  It misses other formats (private keys in PEM, JWTs, other providers' keys) and secrets in files, which sync as git would commit them. Placeholders name a secret by a hash keyed per machine ([ADR 0123](docs/adr/0123-redaction-ids-keyed-per-machine.md)). Ones pushed before that change used an unkeyed hash, so rotate a weak secret that may have been redacted then.
- **Silt's sandbox is off until a workspace turns it on** (`silt workspace policy <name> --silt-sandbox on`). Outside it, agents run as you and could answer their own approvals through Silt's socket. Inside it:
  - the default network is any public host, so an agent can still send out what it reads; choose the gateway or no network to stop that;
  - shared build caches are writable;
  - `--network host` gives it this machine's network.
- **macOS's sandbox, on Seatbelt, is weaker than Linux's** and hasn't been tried on a real Mac yet. Windows has none.
- **Updates from a clone build what `origin/main` holds,** without checking commit signatures. There are no signed releases yet.
- **Agents added from the ACP registry** as npm or PyPI packages are run through `npx` or `uvx` without a checksum of their own. Release archives the registry lists are checked against the checksum it gives.
- **Phones on your network:** the listener is off until you turn it on. It drops connections from outside the local network; pairing needs a one-time code and your approval on the machine.
