# 0093: Silt sandboxes the agents it starts, and production is reached through plugins

- Status: accepted
- Date: 2026-11-17
- Amends: [ADR 0017](0017-acp-agents-only.md) (Silt sandboxes nothing itself), [ADR 0022](0022-agent-runtime.md) (who answers permission requests), [ADR 0065](0065-workspaces.md) (the sandbox rules), [ADR 0080](0080-a-page-calls-mcp-servers.md) (connectors, now for agents too)
- Amended by: [ADR 0094](0094-silt-s-sandbox-is-its-own-rule.md): Silt's sandbox is a workspace rule of its own
- Amended by: [ADR 0102](0102-agents-ask-for-folders.md): A sandboxed agent asks for more folders
- Amended by: [ADR 0103](0103-the-github-plugin.md): The GitHub plugin
- Amended by: [ADR 0104](0104-one-plugin-model.md): One plugin model, in WebAssembly, for Silt's plugins and everyone's
- Amended by: [ADR 0106](0106-silt-s-sandbox-guards-against-an-eager-agent.md): Silt's sandbox guards against an eager agent, and the person reviews its work

## Context
People want agents to work freely on everyday development and on devices, while:
- production actions stay deliberate;
- credentials stay out of the agent's reach;
- some data, such as production location data, never reaches the agent at all.

The ten requirements and the research behind this decision are in [docs/research/sandboxing.md](../research/sandboxing.md).

**What Silt does today falls short:**
- **It relies on each agent's own sandbox (ADR 0017).** Those confine shell commands, not the agent. Claude Code's Read, Edit, WebFetch, hooks and MCP servers run outside its sandbox, and the others are alike.
- **No agent binds an approval to what then runs.**
- **Copilot, Antigravity, OpenCode and every ACP agent can't be sandboxed at all**, so a workspace that requires the sandbox refuses them.
- **An agent can act as the person.** The daemon's socket admits any process with the person's uid, and `silt open` hands out the web UI's token over it. Outside an isolated workspace, an agent can answer its own permission requests.
- **Credential files are readable** by any agent running as the person.

**Silt's earlier sandbox was removed for findings** that a new one must answer:
- it reached host sockets (B2.1);
- it inherited the environment (B2.2);
- `.git` was writable, so hooks ran on the host later (B2.3);
- its file tools followed symlinks (B2.4).

2026's advisories against the agents' own sandboxes repeat them.

The rules must be configuration that suits many companies, and must need no change to anyone's production systems.

## Decision
**Silt runs every agent it starts in a sandbox of its own,** whether a CLI in a terminal or an ACP agent from the web UI, in a workspace that turns it on.
- **It's off until a workspace, or an org's policy, turns it on** (`silt workspace policy <name> --silt-sandbox on`), as it needs bubblewrap and changes what agents can reach. Outside it, agents run as the person, and approvals hold only for sandboxed runs.
- **It wraps the agent's whole process tree:** its tools, MCP clients and subprocesses.
- **The agent's own sandbox stays an optional inner layer.** Nested sandboxes are allowed by default and can be turned off per workspace.

**The default backend is a process sandbox:**
- **Linux:** bubblewrap, shipped with Silt, with seccomp and Landlock where the kernel has it.
- **macOS:** Seatbelt.
- **The root is built fresh:** read-only binds of the host's system and toolchain directories, plus the thread's managed checkout read-write. It is never the host's `/` with credentials masked. What isn't mounted can't be found.
- **Read-only inside it:** `.git/config`, `.git/hooks`, `.silt/`, and the agent's settings and hooks.
- **The agent's home** is read-write, without its stored sign-in.
- **No host socket enters.** The environment is allow-listed.
- **A PID namespace on Linux,** so stopping a run ends everything in it.
- **Heavier backends follow,** for workspaces that ask: OCI bundles (`crun`/`youki`) for reproducible toolchains, and VMs (libkrun, gVisor) for untrusted content.

**What a sandbox can reach is a rule of its workspace.** An org's policy can cap each one.
- **Network:**
  - none;
  - the gateway only;
  - **full internet**, the default;
  - the host's network, opt-in and warned.
- **Full internet:**
  - **Routing:** HTTP(S) goes through Silt's gateway, so credentials are added and production's hosts refused, and other traffic goes direct through `pasta`, shipped with Silt.
  - **Always blocked:** the host and its loopback, private ranges and cloud metadata, with rules the agent can't undo. Hosts on a VPN are private, so they're reached through plugins or a range a rule names.
  - **On macOS**, Seatbelt can't exclude the LAN, so full internet goes through the gateway only.
- **Devices:**
  - none;
  - relayed by Silt (serial through a pseudo-terminal, adb through the host's adb server), which survives the agent's restarts;
  - named device nodes;
  - all USB.
- **Host extras,** each opt-in and warned of what it hands over:
  - named host ports;
  - extra paths;
  - the GPU;
  - the Docker socket;
  - the `ssh-agent` socket.

**Each run has one channel to Silt,** a socket that carries:
- its egress to the gateway;
- its hooks' capture steps;
- `silt mcp` for history scoped to its workspace, and its plugins' tools;
- its device relays.

The channel is the run's identity and grants nothing beyond the run. It can't approve. A sandboxed run needs the daemon, which a hook starts (ADR 0036).

**The gateway holds the credentials for what a sandbox may reach directly.** The sandbox holds placeholders and Silt's CA. The gateway:
- adds the real credential per host, method and path, including the agent's own model API's;
- refuses responses that echo it;
- classifies GraphQL against the schema it introspected.

**Production is reached only through plugins.**
- **What a plugin is:** the agents' counterpart of page connectors (ADR 0080). It's a package of an MCP server, its configuration, its rules and optional skills, run by Silt on the host. Production's credentials live there, never in a sandbox.
- **On each call, Silt:**
  - classifies it as a read or a change: a GraphQL query or mutation by the schema; a plugin's own tools by its `readOnlyHint`;
  - asks for approval where the rules say so;
  - runs its stored copy once;
  - passes the result through the data rules before the agent sees it.
- **Commands a plugin runs** (such as SSH to a test-fleet car over the VPN) go without a terminal, with only the approved input, and with protocol upgrades and interactive channels refused.
- **Clouds** come as plugins too, as does the rest. A marketplace of plugins is later work.
- **Plugins are turned on per workspace.**
  - A plugin is installed once on a machine and turned on in the workspaces the person chooses.
  - An agent gets the tools of the plugins on in its thread's workspace, and no others, as Silt's history tools answer for the run's workspace only (ADR 0065).
  - An org's plugins come with its policy. They are on in every workspace bound to that org and can be turned on nowhere else, so a personal project's agent never reaches acme's API.
  - An org's policy can also refuse the person's own plugins in its workspaces, or allow only named ones, as it can page connectors (ADR 0080).
- **Credentials belong to a workspace too.** The gateway adds one only for its workspace's runs. The same host can have one credential in Work, such as an account on acme's forge, and another in Personal.

**Approvals are the person's, in Silt's UI, never the agent's.**
- An approval is of a hash of Silt's canonical request: target, command, input, and files by content.
- It is drawn from that request, not from the agent's description, with escapes and links made visible.
- It is used once, and Silt runs only its stored copy.
- A change in production, or a mutation, is always approved once. Other scopes are once, this run, or this thread until a time.
- One person may approve their own agent's request.
- Approvals from phones and the hosted app are relayed by the server, which is trusted for it (ADR 0026).

**Data rules** name a class (such as `location`), the environments it covers, and a level:
- **allowed fields only:** a guarantee;
- **schema:** GraphQL fields refused before the request is sent;
- **free text:** detectors, refusing rather than redacting, and refusing binary, compressed or encoded output.

A data class's lock can't be lifted by any approval. Test-fleet and lab data is a separate environment, which rules may leave open.

**The daemon tells agents from the person.** A sandboxed agent can't reach the socket or the web UI, and requests on a run's channel carry only that run's authority.

**In the web UI** (mockups drawn on screens captured from the app):
- **Settings › Workspaces** gains "Plugins its agents can use", with an org's plugins on and locked, and "What its agents can reach":
  - the isolation backend;
  - the four network modes, with the ranges always blocked and the ranges allowed;
  - devices;
  - what each extra from this machine hands over.

  The rules table gains rows for each, with an org's caps.
- **Settings › Plugins and Credentials:**
  - which workspaces each plugin is on in, one column per workspace;
  - each plugin's environments, what runs without asking and what waits, its data rules and its tools;
  - each credential's workspace, host, what it's for, where it comes from, and what the agent sees instead.
- **A thread:**
  - **The header** has a shield beside Fork, whose popover lists what the run can reach. It's for reading: the sandbox is set by the workspace's rules when the run starts, so it stays out of the composer, which holds what a person changes mid-run.
  - **A plugin's change** waits in the permission card's place, showing Silt's own copy of the request, its hash and expiry, and Approve and send once.
  - **Refusals, data-rule refusals and unknown outcomes** show in the transcript, with the next step offered in place.
  - **The inspector's Access tab** lists the run, its grants with Revoke, devices and sessions, plugin calls and refusals.

**Lifecycle:**
- Grants are a run's. Forks and new threads start with none.
- On the same machine, a resume restores a thread's grants that haven't expired, if the rules are unchanged.
- Revoking or stopping closes the run's connections, relays and sandbox.
- A plugin call that was sent but whose end wasn't seen is recorded as **unknown**, told to the agent and the person, and never retried by itself.

## Consequences
- **Every agent Silt starts can be sandboxed,** so a workspace rule requiring the sandbox no longer refuses Copilot, OpenCode or ACP agents. The `SandboxSupport` an adapter reports stops mattering.
- **Each adapter says three things:** its model API's hosts and credential variable, its home, and whether its own sandbox nests.
- **Silt ships `bwrap` and `pasta` for Linux.** Ubuntu 24.04 needs bwrap's AppArmor profile installed once, with root; `silt access doctor` says so.
- **Running agents without a daemon goes away for sandboxed runs.** Capture stays daemon-optional for unsandboxed ones (ADR 0018).
- **On macOS a process that double-forks can escape a run's end,** as Seatbelt has no PID namespace. The VM backend closes that.
- **Full internet lets an injected agent send out what it can read:** code, and what it was shown. Credentials and production data stay out by construction, which is what makes it an acceptable default.
- **Free-form text from production can't be guaranteed clean** without the source's help. The rules say which level each plugin uses.
- **New pieces:**
  - the sandbox's platform code in `silt-os`;
  - reach options and data rules in `silt-workspace`;
  - a crate for the gateway and the plugin host;
  - grants and approvals in `silt-store`, with migrations.
- **The wire format changes for relayed approvals,** so `PROTOCOL` bumps.
- **The guide's workspaces page** says what each reach option hands over, and what the sandbox can't stop.
