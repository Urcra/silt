# 0104: One plugin model, in WebAssembly, for Silt's plugins and everyone's

- Status: accepted
- Date: 2026-11-18
- Amends: [ADR 0093](0093-silt-sandboxes-the-agents-it-starts.md) (plugins), [ADR 0102](0102-agents-ask-for-folders.md) (Folders), [ADR 0103](0103-the-github-plugin.md) (GitHub)
- Amended by: [ADR 0122](0122-the-plugin-sdk-is-mit.md): the SDK, the crates under it and the template are MIT

## Context
Folders and GitHub are Silt's first plugins. A marketplace of other people's comes soon, and teams will want private plugins of their own that never go near a marketplace.

**What a third party's plugin must be able to do** is at least what Silt's own do:
- declare its actions, each a read or a change, with rules for each;
- carry settings, credentials and environments;
- show approval cards that set out exactly what will happen;
- bind an approval to what then runs;
- give the agent files, as GitHub's mirrors do;
- ask for more of the machine, as Folders does.

It must also say, in its own words, what it does and how its settings work.

**What the model has to avoid:**
- **Everything or nothing.** A plugin gets only the capabilities it was granted, with no open HTTP and no process that escapes.
- **MCP servers outside the sandbox.** A local MCP server a plugin needs runs in a sandbox too.
- **Trusting annotations.** The MCP spec warns that a tool's annotations are untrusted unless its server is.

## Decision
**One model for every plugin, and every plugin is WebAssembly.** Silt's own, the marketplace's, an org's and the person's are all built the same way: the same manifest, the same API, the same host, the same capabilities and consent.

**Silt's own plugins differ only in being shipped with Silt.**
- **Where they live:** Folders and GitHub are plugin packages in Silt's repository (`plugins/folders/`, `plugins/github/`), each with its own `silt-plugin.toml`. They're built as any plugin is, and embedded in the `silt` binary.
- **Installed and updated with Silt.**
- **Turned off, not removed.** They can be turned off in any workspace, but not removed.

They get no exception: whatever they can do, any plugin can. If one needs something the API lacks, the API grows.

### A plugin is a WASI component, run by Silt
- **What a plugin is:** a WebAssembly component built for `wasm32-wasip2`. Silt runs it in a wasmtime host inside the daemon. A plugin is never a process and never starts one itself. Where it needs a local MCP server, Silt runs that server for it, in an OS sandbox (`mcp-process`, below).
- **The interface:** the plugin and Silt talk through a versioned WIT world, `silt:plugin@1`. When the world changes, its version goes up; a plugin built for an older world keeps loading.
- **The host is the sandbox:**
  - no ambient files, network or clock beyond WASI's;
  - a data folder of its own, pre-opened, and nothing else of the file system;
  - limits on memory, time per call (wasmtime epochs) and the size of what it returns;
  - everything else only through the capabilities it was granted.
- **Exports, what Silt calls:**
  - `describe() -> list<tool>`: each tool's name, description, input schema, and the action it maps to.
  - `prepare(tool, input, context) -> prepared`. `prepared` holds:
    - the action, and the environment it picked;
    - the fields rules may match on, as the manifest declares them;
    - a **card** built from Silt's blocks (title, fields, lists of items, a diff summary, a code block, a note), which Silt draws with hidden characters made visible;
    - its **effects**.
  - `run(prepared, context) -> result`.
- **Imports, what the plugin may call:** `settings`, `log`, a small key-value store, and the capability interfaces below, only those granted.
- **Authoring:**
  - **Rust is first,** with `silt-plugin-api`, an SDK crate that wraps the WIT world, and a template repository.
  - **Other languages** that build WASI components (componentize-js, componentize-py, TinyGo) work against the same WIT, and the guide shows one example of each later.

### Capabilities are host interfaces
Each capability is declared in the manifest, shown at install, and granted per plugin. Any new one in an update needs consent again. A plugin without a capability can't even name its interface.

| Capability | What the host does for the plugin | Effects | Who uses it |
|---|---|---|---|
| `http` | Sends requests through Silt's gateway, to the hosts the manifest and settings name, adding the credential for that host. The plugin never sees the credential. | Bound: method, URL, body hash | GitHub, GraphQL API, Remote MCP, most others |
| `git` | Runs git on the host, on mirrors Silt keeps for the plugin and thread: fetch from a remote, import a commit from the agent's repo (upload-pack only), push one commit with a lease. Credentials go only on git's command line. | Bound: remote, ref, old and new commit | GitHub |
| `agent-files` | Shows a folder of the plugin's data, read-only, in the agent's sandbox | none | GitHub's mirrors |
| `thread-read` | Lets it read the thread's checkout and work area, read-only, during a call | none | GitHub's push, release notes |
| `reach` | Asks Silt to widen an agent's sandbox (folders, hosts, devices). Always an approval of its own, with the workspace's rules and Silt's built-in refusals applied | Bound: what was asked | Folders |
| `sockets` | TCP to a host and port the manifest names | **Opaque** | databases, custom protocols |
| `ssh` | One command on a named host, through this machine's `ssh` with the person's keys and known hosts, in batch mode with no terminal, forwarding or local command | **Opaque**, with the command and input shown | SSH commands |
| `mcp-process` | Runs a local MCP server the manifest names (a command, or a package pinned by hash) in an OS sandbox of its own: Silt's bubblewrap or Seatbelt with its data folder writable, the thread's checkout read-only only with `thread-read`, no other file of the person's, and a network only through the gateway to the declared hosts, with credentials added there. The plugin is the server's MCP client through the host. | **Opaque**: the process makes its own requests | Local MCP, plugins wrapping an existing MCP server |

What a capability can never do, for any plugin:
- reach the person's files outside what `reach` was granted for;
- reach the daemon's socket or the web UI;
- answer an approval;
- start a process itself. A process runs only as `mcp-process` runs it, in its sandbox.

### Calls: prepared, shown, then run as approved
1. **Prepare.** The plugin returns the action, its fields, the card and the effects. Silt hashes all of it, and the approval is bound to that hash.
2. **Run,** once the rules or the person allow it.
   - The host compares each effect the plugin causes with the approved ones: the same request, the same push.
   - Anything else is refused, reported on the card and in the Access tab, and the call ends as refused.
   - An opaque effect is allowed only when the approved card said so. It reads "Silt can't check what this sends", and an org's policy can refuse opaque effects.

Rules are matched on the action and its declared fields, never on free text. That's what makes "always allow pushes to `ada/*`" safe for any plugin.

### A plugin is a git repository
- **The manifest:** `silt-plugin.toml` at the repository's root, or in a folder of a monorepo. It names:
  - **its identity:** id, name, publisher, version, license, repository, and the world version it was built for;
  - **its build:** the toolchain (Rust with `cargo build --target wasm32-wasip2`, pinned), or a release asset with its sha256;
  - **its actions:** for each, an id, a label, a Markdown description, read or change, a default rule, and the fields its rules may match on;
  - **its environments:** what one is for this plugin, and which input field picks it;
  - **its settings:** a JSON Schema subset (string, number, boolean, enum, path, list, secret), each with a Markdown description, a default and a scope (the plugin, an environment, a workspace);
  - **its credentials:** each with the hosts it goes to, how it's sent (a header and scheme, Basic, or a query parameter), and how a person gets one;
  - **its capabilities,** each with a reason shown at install;
  - **its docs:** `README.md`, an optional `docs/` folder, and skills that teach agents how to use it.
- **Where plugins come from, all through the same install dialog:**
  - **The marketplace** (ADR 0105): it builds the plugin from a tagged commit and signs the build.
  - **A git URL**, for private plugins: `silt plugin add git@github.com:acme/silt-deploys.git#v2.1.0`, or Settings › Plugins › Add from git.
    - Silt clones it with the person's own git credentials, outside every sandbox.
    - It pins the commit.
    - It uses the release asset the manifest names after checking its hash, or else builds the plugin in a build sandbox that reaches only crates.io.
    - A tag or branch is followed only when the person says so.
  - **A local folder**, for developing a plugin (`silt plugin dev ./my-plugin`). It's rebuilt and reloaded on change, and marked "dev".
  - **An org's policy**, which can name plugins by marketplace id, or git URL and commit. They install on its members' machines, on and locked in its workspaces.

### Trust depends on what Silt can check and whom the person trusts
- **Binding:** for an effect Silt binds, the card says Silt checks it, for any plugin.
- **A publisher's defaults:** a plugin's classification of an action as a read is its publisher's word. Its actions start from the manifest's defaults only once the person trusts that publisher; until then every action starts as Asked.
- **Silt counts as a trusted publisher** because it ships with Silt, and the person can withdraw that trust like any other.
- **Updates:**
  - No third-party plugin updates by itself.
  - An update shows what it adds (actions, hosts, capabilities, credentials, settings' new defaults), and anything new needs consent.
  - A git plugin's update also lists the commits between the two versions.

### Rules, settings and where each value comes from
- **Precedence, highest first:**
  1. Silt's built-in refusals, which can't be lifted;
  2. a "never" from any scope;
  3. "asked";
  4. "allowed";
  5. the action's default;
  6. the plugin's default.

  An org's rules only tighten.
- **Every rule and setting shows where it comes from:** the manifest's default, the person's (with Reset), or the org's (locked).
- **Secrets go to the keyring,** never to a file, a sync, a plugin or a sandbox.
- **Available is separate from allowed:** turning a plugin on in a workspace offers its tools there, and its rules decide each call.

### Each plugin's page
The page has these tabs:
- **Overview:** the README, from the repository at the installed commit;
- **How it works:** its build, its capabilities with their reasons, its hosts and its effects, drawn from the manifest;
- **Settings:** each with its Markdown description;
- **Actions:** each with its description and its rule;
- **Status:** loaded, its world version, its last error, and how long its calls take;
- **Calls.**

The agent gets the plugin's skills and its tools' descriptions, never its settings' secrets.

### Today's plugins
`plugins.json`'s three kinds were never released.
- **GraphQL** becomes a plugin shipped with Silt, **GraphQL API**. It's configured by its settings (endpoint, credential, data rules, `direct`), its documents are classified by `silt-access`, and its requests are bound effects.
- **Command** (SSH to a test fleet) becomes **SSH commands**, shipped with Silt, on the `ssh` capability, with opaque effects and the command and input on the card.
- **MCP servers** become two plugins shipped with Silt:
  - **Local MCP** wraps a command the person names, through `mcp-process`;
  - **Remote MCP** bridges an MCP server over HTTP, through `http`.

  Each of a server's tools is an action that starts as Asked, and its effects are opaque.
- **A plugin of anyone's**, the marketplace's included, can wrap an existing MCP server the same way, with its own cards and rules around the server's tools.
- `plugins.json` becomes the list of installed plugins, their grants and settings.

## Consequences
- **Plugins come from git until the marketplace lands** (amended 2026-11-18, before release). The first release has plugins shipped with Silt, from a git URL, from a dev folder and from an org's policy, which names them by git URL and commit. The marketplace (ADR 0105) is a milestone of its own; its install, updates and blocklist build on the same install dialog and consent.
- **A capability's hosts, and a local MCP server's command, can come from a setting** (amended 2026-11-18, before release): `hosts_from` and `command_from` name one of the plugin's own settings, whose values the person sets, as an API's endpoint or a fleet's hosts are theirs to know. Setting it is the consent; the plugin can't change it. A credential can name the same setting for the hosts it's sent to. This is how GraphQL API, SSH commands, Local MCP and Remote MCP reach what the person names.
- **SSH runs as the person's own `ssh` would** (amended 2026-11-18, before release). Silt doesn't carry its own client: `ssh -T` in batch mode, with forwarding, agent forwarding, X11, local commands and terminals off on its command line, so the person's keys, agent and `known_hosts` apply as they set them, and an unknown host key fails rather than asks. Their `~/.ssh/config` applies too, as it would at their terminal; the host is one the manifest names, which the person consented to.
- **Folders and GitHub are Wasm plugins** in `plugins/`, built in the workspace and embedded in `silt`, and they're the first users of the API.
  - Folders uses `reach`.
  - GitHub uses `http`, `git`, `agent-files` and `thread-read`.
  - Tests check that neither does anything its manifest doesn't declare.
- **New pieces:**
  - `silt-plugin`, a pure crate: the manifest, its validation, card blocks, effects and rule matching;
  - `silt-plugin-api`, the SDK, with the WIT world;
  - the Wasm host in `silt-daemon`;
  - the capability interfaces, and the process sandbox for `mcp-process`;
  - effect binding;
  - plugin storage and grants in `silt-store`, with migrations;
  - `silt plugin add|dev|update|remove|trust`;
  - the Plugins pages in the web UI.
- **New dependencies:** `wasmtime` with its component model and WASI. The daemon grows by its size.
- **A plugin that wraps a local MCP server is as safe as its sandbox, not as its binding.** Its calls are opaque, since the server makes its own requests. The card says so, and an org can refuse `mcp-process` or opaque effects.
- **Writing a plugin** needs a guide, the template repository and a test harness that runs a plugin against a stand-in host.
