# Sandboxing agents: a feasibility study

Researched 2026-11-17. The question: can Silt build in sandboxing that meets ten requirements, from everyday development through device labs to production? This note gives a verdict for each, and the design the verdicts assume.

**What Silt must do, as asked:**
- **Every agent Silt starts:** CLI agents, the ACP registry's and custom ones, not only Claude Code.
- **Linux and macOS first.**
- **Configurable rules,** so many companies with different rules can each set theirs.
- **Nothing to change on anyone's side:** no change to anyone's production systems, data or infrastructure.
- **Production only through connectors.** Production keys never enter the agent's sandbox, so the agent reaches production only through tools Silt runs, as MCP connectors.

Versions checked that day:
- **Agents:** Claude Code 2.1.326, `sandbox-runtime` 0.0.83, `claude-agent-acp` 0.107.3, Codex 0.169.0, Gemini CLI 0.68.0, Copilot CLI 1.0.112.
- **Sandboxes:** bubblewrap 0.13.1, passt 2026_10_02, `hakoniwa` 1.8.1, `nono` 0.82.0, libkrun 1.19.8, Podman 6.0.
- **Infrastructure:** Kubernetes 1.37.2, OpenSSH 10.6p2, labgrid 26.0.1.

Claims marked *(unverified)* come from secondary sources, or are inferences still to be tested.

## Verdicts

| # | Requirement | Feasible? | What it hinges on |
|---|---|---|---|
| 1 | Everyday development stays easy | **Yes** | Each run gets a light sandbox (bubblewrap on Linux, Seatbelt on macOS) using the host's own toolchains. What it can reach (none, the gateway only, full internet, devices) is an option. The gateway adds credentials |
| 2 | Device development | **Yes** | Devices stay on the host. Silt relays serial and adb into the sandbox, and keeps sessions alive across the agent's restarts. labgrid goes through a connector pinned to the place granted |
| 3 | Production actions are deliberate | **Yes** | Production only through connectors. Each command is approved by the person in Silt's UI, then run once from Silt's stored copy, without a terminal |
| 4 | Production location never reaches the agent | **Yes for fields allowed by name; strong for GraphQL; best effort for free text** | Silt checks every production result before the agent gets it. Only an allowlist of output fields is a guarantee without changing the source |
| 5 | Credentials stay confidential | **Yes** | No real credential inside the sandbox. The gateway (dev and staging) and connectors (production) hold them |
| 6 | API reads vs changes | **Yes** | Silt parses GraphQL against its introspected schema: queries go through, each mutation is approved with its document and variables. A query whose resolver writes can't be detected |
| 7 | Only authorized destinations and resources | **Yes** on Linux; **yes with care** on macOS | Each mode is enforced below the agent: no network, the gateway only, or full internet without the host, the LAN or metadata. On macOS full internet goes through the gateway, as Seatbelt can't exclude the LAN |
| 8 | Developer decisions are trustworthy | **Yes** | The sandbox can't reach Silt's socket or web UI. Approvals are tied to a hash of the request and used once |
| 9 | Conversations and lifecycle stay apart | **Yes** | One sandbox and one channel per run. Stopping it ends everything (on macOS, a process that double-forks can escape), and outcomes nobody can confirm are reported as unknown |
| 10 | The system stays manageable | **Yes, with effort** | Rules in one place, sets of rules ready to use, refusals that say why, a dry run, `silt access doctor`. The cost is images, a macOS VM and connectors to maintain |

**In short:** all ten can be built in Silt, for any agent, with rules that are configuration rather than code. It reverses ADR 0017's "Silt sandboxes nothing itself" ([ADR 0017](../adr/0017-acp-agents-only.md)). The one requirement that can't be fully guaranteed without the source's help is requirement 4 for free-form text, such as logs read over SSH. There Silt offers three levels and says plainly which one a rule uses (below).

## Threat model
- **The adversary is the model,** steered by prompt injection from code, issues, logs, web pages, device output or another person's thread ([ADR 0016](../adr/0016-untrusted-peer-content.md)). It will retry, encode, and look for other routes and clients.
- **Partly trusted: the agent CLI.** Its permission system is not a boundary. Claude Code's docs say so of Bash rules, and 2025–2026 brought advisories for Claude Code's and Codex's sandboxes and allowlists.
- **Trusted:**
  - the person and their devices;
  - the host's kernel, or the VM's hypervisor on macOS;
  - Silt's daemon, gateway and connectors, all outside the sandbox;
  - the connectors a company configures.
- **Out of scope:**
  - agents run without Silt;
  - kernel exploits on Linux without the hardened runtime;
  - a person approving a harmful production command. Approval makes an action deliberate, not safe. Requirement 4 is the one rule no approval lifts.

## Where Silt stands today
**What can be reused:**
- Workspace rules and org policies that only tighten ([ADR 0065](../adr/0065-workspaces.md)).
- Per-run tokens.
- An allow-listed environment for agents ([ADR 0016](../adr/0016-untrusted-peer-content.md)).
- Ending an agent's process tree ([ADR 0057](../adr/0057-silt-ends-what-agents-leave-running.md)).
- Device capabilities for relayed commands ([ADR 0026](../adr/0026-hosted-mode-and-remote-control.md)).
- Page connectors: MCP servers the person sets up, with secrets in the keyring and every action confirmed ([ADR 0080](../adr/0080-a-page-calls-mcp-servers.md)).
- Managed checkouts with git dirs of their own (`.silt/checkouts/`).

**Gaps:**
- **An agent can act as the person.**
  - The daemon's socket admits any process with the person's uid (`crates/silt-daemon/src/server.rs:198`).
  - The web UI's token comes in exchange for a one-time code that `silt open` gets over that socket (`web.rs:276`).
  - So an agent outside an isolated workspace can answer its own permission requests.
  - A sandbox removes the route for sandboxed runs, but the daemon should still tell agents from the person.
- **Approvals relayed by the server aren't signed end to end.** Whoever controls the server can forge one.
- **Credentials are files the agent can read:** `~/.ssh`, `~/.aws`, gh's `hosts.yml`, `~/.claude/.credentials.json` and `servers.json`. Vendor variables such as `GH_TOKEN` are passed on (`crates/silt-acp/src/registry.rs:229`).
- **ACP's permission answers aren't bound to what runs.** What's shown is written by the agent, and "allow always" is the agent's to scope.

**The last sandbox.** Silt's own sandbox was removed (ADR 0017) after the 2026-10-20 review found that:
- it reached host Unix sockets: systemd, D-Bus, Docker and Silt's daemon (B2.1);
- it inherited the environment (B2.2);
- `.git` was writable, so hooks and `fsmonitor` ran on the host later (B2.3);
- the file tools followed symlinks (B2.4).

2026's advisories for the agents' own sandboxes repeat the pattern:
- a `SessionStart` hook planted from inside the sandbox (CVE-2026-25725);
- git worktree and `fsmonitor` confusion (CVE-2026-55607);
- symlink TOCTOU;
- approval dialogs spoofed through symlinks (GhostApproval, 2026-07).

The design below answers each.

## Why Silt confines the agent itself
- **The agents' sandboxes confine shell commands, not the agent.** Claude Code's covers Bash, PowerShell and Monitor. Its Read, Edit, WebFetch, hooks, MCP servers and `apiKeyHelper` run outside. Codex, Copilot (MXC) and Gemini differ in detail but not in kind.
- **None of them binds an approval to what runs, or knows about production, GraphQL or devices.**
- **A sandbox Silt starts wraps whatever process it starts, the same way for every agent.**

## The design

```
 person ── approves in Silt's UI ──┐
                                   ▼
 ┌──── sandbox (one per run) ───────────┐  run channel   ┌────────── host: Silt ───────────┐
 │ agent CLI, its tools, MCP clients     │═══(socket)════▶│ gateway   ─▶ internet / allowed │
 │ host's /usr, toolchains (ro)          │                │           (credentials added)   │
 │ managed checkout (rw)                 │                │ capture   ─▶ store              │
 │ .git/config, hooks, .silt (ro)        │                │ silt mcp  ─▶ history (scoped)   │
 │ agent home (sessions; no sign-in)     │                │ connectors ─▶ production         │
 │ no credentials · no host sockets      │                │ devices   ─▶ serial, adb, labgrid│
 │ network, devices: as the rules say    │                └─────────────────────────────────┘
 └──────────────────────────────────────┘
```

### The sandbox: light by default, heavier where wanted
A workspace chooses its isolation, and an org's policy can require a minimum.

| Backend | Linux | macOS | Starts in | Gives |
|---|---|---|---|---|
| **Process sandbox** (default) | bubblewrap: user, mount, PID and network namespaces, plus seccomp and Landlock where the kernel has it | Seatbelt (`sandbox-exec`), deprecated but working on macOS 26 | ~10 ms (bwrap `ls /`: 8 ms against Podman's 279 ms and Docker's 378 ms) | The host's toolchains as installed, no image, no daemon |
| **OCI image** (optional) | `crun` or `youki` run straight on a bundle (47 and 111 ms create→delete), or Podman/Docker if present | Through the VM below | Under 0.5 s | Reproducible toolchains, a project's devcontainer |
| **VM** (optional) | gVisor (`runsc`), Kata, or libkrun | libkrun (Hypervisor.framework; `microsandbox` 0.7 claims ~100–300 ms), Lima, OrbStack | 0.1 s to seconds | A kernel of its own, for untrusted content |

**Why bubblewrap on Linux:**
- It's small and audited. Version 0.13.1 is current; 0.12 dropped setuid builds and fixed a symlink escape, GHSA-pxhw-h44j-8pfx.
- Each option is a flag, so Silt builds the argv from the rules.
- Silt uses the system's `bwrap`, or ships one.

**Alternatives for later:**
- `hakoniwa` 1.8 is a Rust library doing the same, with pasta built in. Its LGPL-3.0 linking exception works with AGPL.
- `nono` 0.82 (Apache-2.0) wraps Landlock and Seatbelt.
- firejail is setuid root and has a history of root CVEs, so it's ruled out.

**How Silt uses it:**
- **A fresh root, not `--ro-bind / /` with masks over it.** The root is a tmpfs holding read-only binds of `/usr`, `/etc`, `/opt`, the toolchain directories and the parts of `$HOME` the rules name, plus the checkout read-write. A path never mounted can't be revealed, even by an agent that gets a mount namespace of its own.
- **Read-only inside it:** `.git/config`, `.git/hooks`, `.silt/`, and the agent's settings and hooks (B2.3, CVE-2026-25725).
- **The agent's home** (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`…), read-write, without its stored sign-in. The host reads session files there, as capture does today ([ADR 0007](../adr/0007-agent-session-capture.md)).
- **`--unshare-pid --as-pid-1 --die-with-parent`:** killing the sandbox kills everything in it.
- **seccomp** always blocks `io_uring` and `AF_VSOCK`.
- **Ubuntu 24.04** restricts unprivileged user namespaces through AppArmor. bwrap needs its profile, which ships with the apparmor package from 25.04 and has to be copied in once, with root, on 24.04. `silt access doctor` says so.

**On macOS, two limits of Seatbelt to state plainly:**
- **Network rules know only `localhost` or every host.** "Internet but not the LAN or cloud metadata" can't be written. Even `localhost` misses `::ffff:127.0.0.1`.
  - So on macOS, full internet goes through the gateway, which does refuse the LAN, the host and metadata. Tools that ignore the proxy then fail closed.
  - Raw connections to the internet need the VM backend.
- **No PID namespace.** A process that calls `setsid` and forks twice escapes a process-group kill. Silt still finds descendants by parent ([ADR 0057](../adr/0057-silt-ends-what-agents-leave-running.md)), but a reparented process can be missed. The VM backend has no such gap.
- **The Keychain** is reached through `com.apple.SecurityServer`, which TLS also uses. Denying it protects Keychain items, but breaks tools that verify TLS through the system. So macOS relies on keeping the agent's own sign-in out of the Keychain: the gateway adds the credential instead.

### What the sandbox can reach: options
Every reach is an option a workspace sets. An org's policy can cap each one.

**Network:**

| Mode | What it does | Linux | macOS |
|---|---|---|---|
| **None** | Nothing | `--unshare-net` | `(deny network*)` |
| **Gateway only** | HTTP(S) and SOCKS through the gateway: its allowlist, credentials, content rules | `--unshare-net`; the run channel's socket bridged to a port inside | only `localhost:<gateway port>` |
| **Full internet** (a likely default for dev) | Any public host. HTTP(S) still goes through the gateway, so credentials are added and production's hosts refused. The host, its loopback, the LAN, private ranges and cloud metadata are always blocked | `--unshare-net` plus `pasta` (`--no-map-gw -t none -u none -T none -U none`, since `-T`/`-U` default to forwarding host ports). nftables rules in the sandbox's network namespace drop 127/8, 10/8, 172.16/12, 192.168/16, 169.254/16, 100.64/10, the host's own addresses, ::1, fc00::/7 and fe80::/10, set before capabilities are dropped (tested: the agent can't flush them) | Through the gateway only (see the limits above); raw TCP and UDP need the VM backend |
| **Host network** (opt-in, warned) | The host's network as is, the host's `127.0.0.1` included | `--share-net` | `(allow network*)` |

Notes:
- **Full internet, to state plainly:** it gives a prompt-injected agent a way to send out whatever it can read, meaning code and what it was shown. Credentials and production data stay protected, since they never enter the sandbox, and production results pass the data rules first. That's why full internet can be a reasonable default for development.
- **Dependencies:** pasta is a system package (passt). Without it, full internet falls back to the gateway for HTTP(S) alone.
- **Kernel surface:** nftables in a user namespace is a frequent target of kernel exploits (CVE-2024-1086). The sandbox drops `CAP_NET_ADMIN` after setting its rules. If nested sandboxes are allowed (below), the agent can make a user namespace of its own and reach nftables there. `ip rule … prohibit` is the smaller alternative *(unverified)*.

**Devices:**

| Mode | Linux | macOS |
|---|---|---|
| **None** (default) | a minimal `/dev` | no `/dev/cu.*`, no IOKit |
| **Relayed** | Silt opens a named serial port on the host and gives the sandbox a pseudo-terminal joined to it; adb goes to the host's adb server on 5037 through the run channel, limited to one device's serial. Sessions survive the agent's restart | same |
| **Named nodes** | `--dev-bind /dev/ttyACM0`, or `/dev/serial/by-id/…` resolved | `/dev/cu.<name>` allowed for reading, writing and ioctl |
| **All USB** (opt-in) | `/dev/bus/usb` read-write, `/sys/bus/usb` and `/sys/devices` read-only, `/run/udev` for hotplug *(unverified)*; groups such as `plugdev` and `dialout` still apply | `/dev/cu.*` plus IOKit user clients for libusb (class names to confirm against the denial log) |

**Host extras, each opt-in, each warned of what it hands over:**
- named host ports, such as a local database or a dev server;
- extra paths, read-only or read-write;
- the GPU (`/dev/dri`, `/dev/nvidia*`);
- the Docker socket ("root on this machine");
- the `ssh-agent` socket ("signs as you for every loaded key, anywhere").

**Nested sandboxes:** allowed by default, so Codex's and Claude Code's own bubblewrap work inside as a second layer. With a fresh root it reveals nothing hidden, but it widens the kernel's surface. A workspace can turn it off, and the agent's own sandbox off with it.

**Credentials are never an option to pass in.** The sandbox's environment is allow-listed. Its filesystem never holds:
- keys and tokens: `~/.ssh`, `~/.gnupg`, `~/.aws`, `~/.config/gcloud`, `~/.azure`, `~/.kube`, `~/.docker/config.json`, `~/.git-credentials`, `~/.config/gh`, `~/.netrc`, `~/.npmrc`, `~/.pypirc`, `~/.cargo/credentials.toml`, `~/.vault-token`;
- the agents' sign-ins;
- browser profiles, keyrings, password managers' stores and agent sockets;
- `/run/user/$UID`, which holds D-Bus and its Secret Service, gpg-agent and Podman;
- Silt's data dir.

The fresh root makes this a list of what's **in**, rather than of what's hidden. A new credential store somewhere in `$HOME` stays out by default.

### The run channel
One socket per run, bind-mounted on Linux and on macOS (both are the same machine for the process sandbox; a VM backend relays it over vsock). It carries:
- **egress** to the gateway (HTTP and SOCKS);
- **capture:** `silt hook` inside forwards its step here. There is no daemonless capture in a sandbox, since the store isn't there, so a sandboxed run needs the daemon ([ADR 0036](../adr/0036-a-hook-starts-the-daemon.md));
- **`silt mcp`:** history scoped to the run's workspace (ADR 0065), and the connectors' tools;
- **devices:** serial relays, adb, and host ports the rules name.

The channel itself is the run's identity: whatever arrives on it acts as that run and nothing more. It has no way to approve anything.

### The gateway: dev and staging
- **HTTP(S):**
  - A host the rules don't look into passes on CONNECT, checked by hostname.
  - A host with credentials or a content rule has its TLS terminated with the run's CA. Then the gateway removes the placeholder `Authorization` and adds the real one per host, method and path, and applies content rules.
  - A response that echoes the injected secret is refused; the gateway knows the value, so that check is exact.
- **Git:** smart-HTTP, with a rule for which refs may be pushed.
- **Kubernetes and the clouds (staging):**
  - kubectl's exec plugins (`gke-gcloud-auth-plugin`, `aws eks get-token`, `kubelogin`) all end in a bearer token, which the gateway gets on the host and adds.
  - GCP and Azure use bearer tokens. gcloud's `auth/access_token_file` takes a placeholder.
  - AWS SigV4 is signed again by the gateway; S3's chunked uploads need each chunk signed again.
- **Always refused:** names resolving to private, link-local or metadata addresses, unless a rule names them; and domain fronting, where a terminated host's `Host` must match its SNI.

### Connectors: production
Production is reached only through connectors, run by Silt on the host. ADR 0093 calls them **plugins**, and decides that the clouds, SSH to a test fleet and the rest all come as plugins, with a marketplace later; the kinds below are what Silt's plugin host has to classify. They're ADR 0080's page connectors made available to agents. A connector is:
- **an MCP server,** either Silt's own kind (GraphQL, HTTP API, Kubernetes, SSH, AWS, GCP, Azure, labgrid) or the company's;
- **its configuration:** endpoints, clusters, hosts and accounts, and credentials taken from the host's keyring or existing logins;
- **its rules:** which tools are reads and which need approval, the data rules on its results, and whether sessions are allowed;
- **optionally, skills** that teach agents how to use it.

An org can publish connectors through its policy, so everyone gets the same ones with the same rules.

Every call:
1. **Is classified.**
   - **GraphQL:** Silt parses the document and checks it against the introspected schema. A query is a read; a mutation is a change.
   - **Kubernetes:** by the request's verb. `exec`, `attach`, `portforward`, `proxy` and any protocol upgrade are interactive.
   - **AWS:** by the operation's `IsWrite`/`IsList`/`IsDataPlane` in AWS's machine-readable service reference, not by the operation's name.
   - **GCP:** through its Discovery documents. `:getIamPolicy` is a POST but a read, and BigQuery's `jobs.query` can write.
   - **A company's MCP server:** by its `readOnlyHint`, since the company configured that server.
2. **Is approved where its rules say so,** in Silt's UI, never in the agent's own prompt (requirement 8).
3. **Runs Silt's stored copy, once.**
   - Commands (`kubectl`, `aws`, `gcloud`, `az`, `ssh`) run in a **throwaway executor sandbox**: only the approved stdin, no terminal, a timeout. Its egress goes through the gateway with production's credentials, and it's destroyed afterwards.
   - So no argv parser has to be perfect. None exists for these CLIs; even AWS's MCP server warns that its read-only mode writes files and returns secrets.
   - `--kubeconfig`, `-o file` and plugins find nothing to use. Port-forwards, `ssm start-session` and exec are refused at the gateway as protocol upgrades.
   - SSH goes through Silt's own client (`russh`): one `exec` channel, never `pty-req`, `shell`, `subsystem` or forwarding, then the connection closes.
4. **Passes the data rules** before the agent sees the result (requirement 4).

**Approval outlasts MCP timeouts:** agents time out tool calls, and an approval can take minutes. Silt uses MCP's task-augmented calls where the agent supports them. Otherwise it answers "pending approval" with an id, and offers a tool to wait for it.

### Rules: configuration, not code
Rules extend the workspace rules, kept in the pure `silt-workspace` crate. An org's policy can only tighten them, and a person can add to the org's but not loosen them ([ADR 0065](../adr/0065-workspaces.md)).
- **Environments:** names such as `dev`, `staging` and `production`, each matched by hosts, Kubernetes API servers or contexts, cloud accounts or projects, SSH hosts, API endpoints and devices.
- **Per environment and connector:**
  - reads: allowed, or asked each time;
  - changes: asked each time, always single use;
  - interactive sessions: allowed or refused;
  - which credentials apply.
- **Data classes:** for example `location`, `personal data` or `health`. Each has:
  - its detectors;
  - which environments it covers: production location data is protected, while a test fleet's isn't;
  - its level (below);
  - a lock that no approval lifts.
- **Approvers:** the person themselves, or a second person; by a click or with a passkey.
- **Devices and labs:** which local devices, which labgrid places, which fleet hosts.
- **Profiles:** sets of rules to start from. One example is "the ten requirements": dev open within the gateway's rules, staging mutations approved, every production command approved, production location locked.

## Each requirement

### 1. Keep everyday development easy — yes
- **Inside the policy, nothing asks:**
  - the agent builds, tests and uses git in the thread's checkout;
  - package registries and the forge are reached through the gateway, with credentials it adds;
  - caches persist between runs.
- **Time to the first prompt:** the process sandbox starts in about 10 ms, against hundreds of ms for a container. The OCI and VM backends add their own start.
- **Friction:**
  - **Tools that bring their own roots** (Rust binaries built with `webpki-roots`, some Go and Java tools) can't use hosts that are terminated. Hosts that need no credentials or content rules pass through on CONNECT.
  - **Tools that ignore the proxy** fail closed: raw TCP database drivers and QUIC fall back or fail. The gateway can forward a TCP port named by a rule.
  - **Docker inside the agent's work:** the host's socket only as a warned opt-in; otherwise rootless Podman inside, or a connector.
  - **Commit signing:** the key stays on the host, and Silt signs commit objects only, through a `gpg.program` or `ssh-keygen -Y` shim.
- **Effort:** large. This is the sandbox's two backends, the reach options, the gateway and the run channel.

### 2. Support device development — yes
- **Local devices stay on the host. Silt relays them:**
  - **Serial:** the daemon opens the port and gives the sandbox a pseudo-terminal joined to it, through the run channel.
  - **adb:** the host's adb server on port 5037, reached through the gateway and limited to the granted device's serial.

  This works the same on Linux and macOS, survives renumbering, and can be revoked by closing the relay. Passing a device straight in (`--device` with a cgroup rule) is a Linux-only extra. USB into macOS VMs is OrbStack only.
- **Sessions that keep their state:**
  - Each one is a pseudo-terminal or tmux in control mode, held by the daemon. It survives the agent's restart, a resume, and the sandbox being replaced. The agent drives it with `device_send`, `device_read` and `device_expect`.
  - ACP's terminals take no input, and pexpect's die with their process, so neither can do this.
  - What a session shows enters the thread's history.
- **labgrid:**
  - Version 25 moved to gRPC. Its coordinator has no authentication or ACLs: a client names itself, `release -k` takes any place, and reservation ids "are not secret".
  - Silt's labgrid connector runs `labgrid-client` on the host, pinned to the granted place. The sandbox can't reach the coordinator or the exporters.
  - That keeps Silt's agents within their grant. People using labgrid without Silt are out of scope, and nothing in the lab has to change.
- **The test fleet:** each car is a target. It can be an SSH host, a fleet API through the gateway, or the company's own connector. Its data classes are its own, so location from the test fleet can reach the agent if the rules say so.
- **Caveat:** a device with a network can reach whatever its network allows. Its network can be ruled only where Silt is its route.
- **Effort:** medium.

### 3. Make production actions deliberate — yes
- **There's no route:** no production credential exists in the sandbox, and the gateway refuses production's hosts on the agent's channel. Production exists only as connectors.
- **Each production command:**
  1. Silt makes the request canonical (target, argv, stdin, files by content hash) and shows exactly that. Escapes, control characters, bidi characters and look-alikes are made visible, and the agent's own title for the request isn't used.
  2. The person approves that hash, freshly, in Silt's UI.
  3. Silt runs its stored copy once, in a throwaway executor or over its own SSH client.
- **Files the agent could change after approval** (`-f values.yaml`, `bash deploy.sh`) are uploaded as the snapshot that was shown.
- **No persistent interactive sessions:**
  - no terminal;
  - stdin is the approved input, then EOF;
  - a timeout;
  - an executor that's destroyed;
  - protocol upgrades and SSH `pty-req`, `shell` and forwarding refused;
  - session tools refusing production targets.
- **Whether reads ask too** is each environment's rule. In the example profile every production command asks, while production GraphQL queries are reads (requirement 6).
- **Effort:** medium. Executors and canonical requests, plus one connector per kind.

### 4. Prevent production location disclosure — guaranteed for allowed fields, strong for GraphQL, best effort for free text
Silt checks every production result before the agent sees it, and before it enters history, search or sync. A data class's lock can't be lifted by any approval. How much it guarantees depends on the level each connector's rule uses:

| Level | What reaches the agent | Guarantee |
|---|---|---|
| **Allowed fields** | Only output fields the rule names (GraphQL fields, JSON paths of a Kubernetes or cloud response), from output Silt parses completely. Anything else, and anything it can't parse, is refused | **Guaranteed** for what's listed, without changing the source. It costs configuration |
| **Schema** | GraphQL: requests that select a location-bearing type or field are refused before they're sent. The schema comes from introspection; the walk covers fragments, aliases, interfaces and unions. Structured responses: fields whose names or values look like location are refused | **Strong.** A `JSON` or `String` scalar can still carry coordinates, which detectors check |
| **Free text** | Logs and command output: refused when they hold coordinates, NMEA, GPX, KML or GeoJSON, binary, compressed or encoded data | **Best effort.** Detectors miss scaled integers (degE7), split pairs and encodings they don't know |

- **Why free text can't be promised:**
  - Detectors are weak here: AWS Macie needs a nearby keyword and decimal degrees, and Presidio has no coordinate entity.
  - "The Framing Gap" (arXiv 2608.27092) found a guard that normalizes known encodings leaked every time on one it didn't know.
  - No vendor claims guaranteed prevention.
  - The strongest Silt-only answer for a production environment is therefore to allow only the first two levels, and to refuse free text from sources that may hold location.
- **What also helps:**
  - Production results are refused rather than redacted when in doubt.
  - Nothing from production is cached or written into the checkout before the rules have run.
  - Streams (`logs -f`, exec, live tail) are refused in production, as they get round checks made per response.
- **Effort:** medium. The schema walk with `apollo-compiler` 1.33 and `cynic-introspection` 3.14, the field allowlists, the detectors.

### 5. Keep credentials confidential — yes
- **Placeholders only:** the sandbox holds placeholders and Silt's CA. The real values stay with the gateway for dev and staging, and with connectors for production.
- **Short-lived, scoped tokens** where the service has them, such as GitHub installation tokens, which last an hour and can be limited to repositories.
- **SSH:**
  - The agent never gets an `ssh-agent` socket. Whoever reaches one gets signatures from every loaded key, and destination constraints rely on a cooperating client.
  - Dev hosts go through the gateway's SSH relay; production goes through Silt's SSH client.
- **The agent's own sign-in:**
  - The gateway adds the model API's credential for that agent's hosts, such as `api.anthropic.com` or `api.openai.com`.
  - The agent gets a placeholder variable such as `ANTHROPIC_AUTH_TOKEN` or `OPENAI_API_KEY`.
  - Signing in and refreshing OAuth tokens happen on the host ([ADR 0056](../adr/0056-signing-an-agent-in-from-the-web-ui.md)). Whether each agent's refresh works through a gateway is *(unverified)*, agent by agent.
- **Nothing beyond its rule:** a credential works only on the hosts, methods and paths its rule names.
- **Kubernetes Secrets** are refused in production; reading them is reading credentials.
- **Effort:** medium.

### 6. Distinguish API reads from changes — yes
- **Parsing:** each request is parsed and validated against the schema from introspection (`cynic-introspection` → `apollo-compiler`). This applies to staging GraphQL through the gateway and production GraphQL through its connector.
- **Reads:** queries and introspection are routine, and still pass the data rules.
- **Changes:** each mutation shows its complete document and actual variables, is approved by hash, and is sent once from Silt's copy.
  - In staging, the gateway holds the request while it waits, or the agent uses the connector.
  - In production, it's always the connector.
- **Closing the ways round it:**
  - **Several operations in one document:** refused if any is a mutation, whatever `operationName` says.
  - **Batches:** every element is checked.
  - **GET:** classified too; not every server forbids mutations over GET.
  - **Persisted queries:** only those whose text Silt saw are allowed.
  - **Other bodies:** `application/graphql` and multipart are parsed or refused.
  - **WebSockets:** graphql-ws frames, whose `Subscribe` carries mutations too, are inspected or the upgrade refused.
- **Limit:** a query whose resolver writes can't be detected. A rule can list allowed query fields for such APIs.
- **Effort:** medium.

### 7. Limit access to authorized destinations and resources — yes on Linux, yes with care on macOS
- **One way out:**
  - **On Linux,** with no network or the gateway only, the sandbox has no network of its own, and its channel is the only way out, so curl, Python, `gh` and a static binary all meet the same rules. With full internet, pasta gives it public hosts, and nftables rules it can't undo drop the host, the LAN and metadata.
  - **On macOS,** Seatbelt allows only the gateway's port. Full internet goes through the gateway, which refuses the host, the LAN and metadata, since Seatbelt can't tell them apart. The VM backend gives raw connections.
- **Scope finer than a host:**
  - by host, method and path;
  - by GraphQL operation and field;
  - by Kubernetes verb and resource;
  - by AWS operation;
  - by SSH target;
  - by labgrid place and by device.

  One run's grants reach nothing of another run's, since each has its own channel.
- **What remains:**
  - **Allowed hosts that serve anyone's content are ways out:** gists, issue comments, package uploads. Rules scope them by path and method.
  - **Devices and fleet cars** reach what their own networks allow.
- **Effort:** included in the sandbox and gateway work.

### 8. Make developer decisions trustworthy — yes
- **The agent can't approve:** the sandbox has no route to Silt's socket or web UI (host loopback is blocked in every mode but Host network), and its channel can't approve. For runs outside a sandbox, the daemon should still tell the person from agents rather than trust the uid alone.
- **What was approved is what runs:** Silt runs only its stored copy, which matches the hash, once, before it expires.
- **Scope is the person's choice and never more:**
  - grants are once, for this run, or for this thread until a time;
  - production changes and mutations are always once;
  - the agent's own "allow always" never applies to connectors.
- **What's shown is the truth:** Silt draws the approval from its canonical request, resolves paths and follows links, which is what GhostApproval exploited.
- **Approvals from a phone go through the server,** which is trusted to relay them today. A passkey signature over the hash, checked by the daemon, would remove that trust. It's offered as a rule, "Approve with a passkey".
- **A second approver** is a rule too.
- **Effort:** small to medium.

### 9. Keep conversations and lifecycle separate — yes
- **No inherited authority:**
  - Grants belong to a run: its sandbox, its channel and its token.
  - A fork, a fork from a turn or a new thread starts with none.
  - Subagents share their run's grants, being the same conversation.
- **Resume:**
  - On the same machine, a thread's grants that haven't expired come back if the rules haven't changed.
  - On another machine they don't, as credentials and devices are per machine. The web UI lists them to confirm again.
  - Device sessions outlive the sandbox and are offered back.
- **Cancel, revoke, shut down:**
  - Revoking a grant closes its connections and relays.
  - Stopping a run kills its sandbox and executors.
  - When the daemon stops, it does the same for every run, as ADR 0057 does for processes today.
- **Unknown outcomes:**
  - Each connector call goes through pending → approved → sent → succeeded, failed, or cancelled before it was sent, or **unknown**: sent, with no end seen.
  - An unknown outcome is recorded in the thread, told to the agent and the person, and never retried by itself. MCP's own cancellation admits the same uncertainty.
- **Effort:** medium.

### 10. Keep the system manageable — yes, with effort
- **Setup:**
  - `silt access setup` checks for `bwrap` and `pasta` on Linux (or Silt's own copies) and the AppArmor profile on Ubuntu 24.04. Nothing to install on macOS. A heavier backend only when a workspace asks for one.
  - It builds or pulls the image, and takes credentials and connectors from the host's existing logins and an org's policy.
- **Daily use:** inside the rules nothing asks. An approval shows the exact request and a choice of scope, and each thread lists its grants, sessions and connectors.
- **When access fails:**
  - The gateway answers with a body naming the rule, where it came from (the person's or the org's), and how to ask.
  - A refused CONNECT, which tools show poorly, is also recorded in the thread and explained by an `access_explain` tool.
- **Troubleshooting:**
  - `silt access doctor` checks the runtime, rootless setup, the VM's loopback, the image's CA and the connectors' sign-ins.
  - A dry run records what the rules would refuse, before they're enforced.
- **Maintenance:**
  - images and agent versions; Claude Code shipped 300 patch releases in 2026;
  - each connector kind;
  - the detectors;
  - the macOS VM.
- **Licences:** the defaults are free. Docker Desktop and OrbStack, if a workspace picks them for the VM backend, are paid for most companies.
- **Effort:** medium, ongoing.

## Platforms
| | Linux | macOS |
|---|---|---|
| Default backend | bubblewrap (fresh root, namespaces, seccomp, Landlock where present) | Seatbelt |
| Heavier backends | OCI bundles with `crun`/`youki`, or Podman/Docker; gVisor, Kata, libkrun | libkrun (`microsandbox`), Lima, OrbStack, Docker Desktop |
| No network / gateway only | `--unshare-net` plus the run's socket | only `localhost:<gateway port>` |
| Full internet | `pasta` plus nftables drops for the host, the LAN and metadata | through the gateway (Seatbelt can't exclude the LAN); raw connections with a VM |
| Devices | relayed, named nodes, or all USB | relayed, `/dev/cu.*`, or IOKit for libusb |
| Ending a run | PID namespace: everything in it ends | no PID namespace; a double fork can escape the kill |
| Kernel | shared; a VM backend for untrusted content | shared; a VM backend for untrusted content |
| Ubuntu 24.04 | bwrap needs its AppArmor profile, once, with root | — |

## Building it
- **Sandbox:**
  - `bwrap`, the system's or shipped with Silt, driven by an argv Silt builds;
  - `seccompiler` 0.5 for its filter;
  - `landlock` 0.4.7;
  - `pasta` for full internet;
  - Seatbelt profiles generated per run on macOS;
  - a `Backend` trait in `silt-os`, so OCI (`crun`/`youki`), libkrun or Podman can follow.

  `hakoniwa` is the library alternative to running `bwrap`.
- **Gateway:** `hudsucker` 0.25, `rustls`, `rcgen` 0.14, AWS SigV4 signing.
- **Connectors:**
  - `rmcp` (already a dependency);
  - `apollo-compiler` and `cynic-introspection`;
  - `russh` 0.65 and `ssh-key`;
  - AWS's service reference JSON and GCP's Discovery documents for classifying.
- **Approvals:** `webauthn-rs` for passkeys.
- **Crates and ADRs:** the sandbox's platform code goes in `silt-os` ([ADR 0045](../adr/0045-windows-natively-and-in-wsl.md)); the reach options and rules go in `silt-workspace`; the gateway and connectors go in a new shell crate. A new ADR amends ADR 0017, 0022, 0065 and 0080.

## A possible order
1. **The daemon tells agents from the person** (requirement 8). It's small, and useful whatever comes next.
2. **The bubblewrap sandbox on Linux,** for every agent, with its reach options:
   - the run channel (capture, `silt mcp`, egress);
   - the gateway with added credentials and domain rules;
   - grants and their lifecycle.

   Requirements 1, 5, 7 and 9.
3. **Connectors and approvals:**
   - GraphQL first (requirement 6), then Kubernetes, SSH and the clouds in executors (requirement 3);
   - the agent's own MCP servers;
   - org-published connectors;
   - unknown outcomes.
4. **Data rules:** allowed fields, the GraphQL schema walk, detectors (requirement 4).
5. **Devices:** serial and adb relays, sessions the daemon keeps, the labgrid connector (requirement 2).
6. **macOS:** Seatbelt profiles generated from the same rules.
7. **Heavier backends** where workspaces want them: OCI bundles, then a VM (libkrun).

## Decided (2026-11-17)
Recorded in [ADR 0093](../adr/0093-silt-sandboxes-the-agents-it-starts.md); the work is M32 in ROADMAP.md.
- **Full internet is the default** network for development. The host, the LAN, private ranges and metadata stay blocked, so hosts on a VPN are reached through plugins or a range a rule names.
- **Silt ships `bwrap` and `pasta`** rather than relying on the system's.
- **The server stays trusted to relay approvals** from phones and the hosted app. Passkey signatures can come later as a rule.
- **One person may approve their own agent's requests.** The agent never can.
- **Production, the test fleet (SSH over a VPN) and the clouds are all reached through plugins.** A plugin is the agents' counterpart of page connectors: an MCP server, its configuration, rules and skills, run on the host. Silt's own part is the plugin host, its classification, approvals and data rules. GraphQL comes first; a marketplace of plugins and cloud plugins are later work.
- **Plugins and credentials are per workspace.** A plugin is installed once and turned on per workspace; an org's plugins stay in that org's workspaces, so personal projects never reach them.
- **Linux, then macOS.**

## Sources
- **Silt:**
  - ADRs 0007, 0016, 0017, 0026, 0036, 0056, 0057, 0065, 0080;
  - `crates/silt-daemon/src/server.rs`, `web.rs`; `crates/silt-acp/src/registry.rs`; `crates/silt-agents/src/claude.rs`.
- **Agents' sandboxes:**
  - https://code.claude.com/docs/en/sandboxing, /hooks, /permissions, /managed-settings, /sandbox-environments;
  - https://github.com/anthropics/sandbox-runtime;
  - Codex's `codex-rs/linux-sandbox`;
  - Copilot's https://docs.github.com/en/copilot/concepts/security-governance-and-network-settings/about-cloud-and-local-sandboxes.
- **Advisories:**
  - Claude Code and its sandbox: CVE-2025-66479, CVE-2026-25725, CVE-2026-55607, CVE-2025-55284;
  - Codex: CVE-2025-59532;
  - Docker: CVE-2025-9074 (Docker Desktop), CVE-2026-17106 (CopyEscape);
  - runc: CVE-2025-31133, -52565, -52881 (https://openwall.com/lists/oss-security/2025/11/05/3);
  - the Linux kernel: CVE-2026-80521 *(unverified)*;
  - research: Trail of Bits on ANSI in MCP output and on argument injection; GhostApproval (Wiz).
- **Sandboxes:**
  - bubblewrap https://github.com/containers/bubblewrap/releases; startup numbers https://www.jvns.ca/blog/2022/06/28/some-notes-on-bubblewrap/;
  - passt/pasta https://passt.top/builds/latest/web/passt.1.html; slirp4netns's man page;
  - hakoniwa https://github.com/souk4711/hakoniwa; nono https://github.com/always-further/nono; libkrun https://github.com/containers/libkrun;
  - CVE-2024-1086 (nf_tables), CVE-2022-31214 (firejail).
- **Containers and VMs:**
  - Lima networking https://lima-vm.io/docs/config/network/user/;
  - Podman https://docs.podman.io/en/latest/markdown/podman-run.1.html;
  - OrbStack USB https://docs.orbstack.dev/features/usb;
  - gVisor https://gvisor.dev/docs/user_guide/filesystem/.
- **Production APIs:**
  - Kubernetes authorization https://kubernetes.io/docs/reference/access-authn-authz/authorization/, impersonation, the WebSockets transition (KEP-4006), `nodes/proxy` https://grahamhelton.com/blog/nodes-proxy-rce;
  - AWS's service reference https://servicereference.us-east-1.amazonaws.com/v1/s3/s3.json, ReadOnlyAccess, the AWS API MCP server https://awslabs.github.io/mcp/servers/aws-api-mcp-server;
  - gcloud's proxy settings https://docs.cloud.google.com/sdk/docs/proxy-settings;
  - `sshd_config(5)`, PROTOCOL.certkeys.
- **GraphQL:**
  - https://http-spec.graphql.org/draft/;
  - APQ https://www.apollographql.com/docs/apollo-server/performance/apq;
  - graphql-ws's PROTOCOL.md;
  - `apollo-compiler` and `cynic-introspection` on docs.rs.
- **Location:**
  - Google https://docs.cloud.google.com/sensitive-data-protection/docs/infotypes-reference;
  - Macie https://docs.aws.amazon.com/macie/latest/user/mdis-reference-pii.html;
  - Presidio https://presidio.dataprivacystack.org/supported_entities/;
  - MAVLink;
  - "The Framing Gap", arXiv 2608.27092;
  - CaMeL, arXiv 2503.18813; FIDES, arXiv 2505.23643.
- **Labs and sessions:**
  - labgrid https://labgrid.readthedocs.io/en/latest/changes.html, discussion 1499;
  - tmux control mode;
  - ACP terminals https://agentclientprotocol.com/protocol/terminals;
  - MCP cancellation (2025-11-25).
