# Plugins: what sandboxed agents reach through Silt

A sandboxed agent holds no production keys and reaches nothing beyond its checkout but what you give it. Plugins are how it reaches the rest: more folders of this machine, GitHub, an API of yours, a test fleet over SSH, an MCP server. Each is a WebAssembly component Silt runs on this machine, outside the sandbox, with credentials the agent never sees, for the workspaces you turn it on in. [Workspaces](workspaces.md) says how a workspace sandboxes its agents and sets up Silt's own plugins; this page is about plugins themselves.

## Where plugins come from
- **Shipped with Silt:** Folders, GitHub, GraphQL API, SSH commands, Local MCP and Remote MCP. They update with Silt.
- **Git:** `silt plugin add <git-url>#<tag>`, or Settings › Plugins › Add from git. Silt clones it with your own git sign-in, shows everything it gets before anything of it is built or run, and builds it offline in a sandbox that sees only the repository.
- **A folder you're writing one in:** `silt plugin dev <folder>`, with `--watch` to build it again on every change ([Writing a plugin](writing-plugins.md)).
- **Your org:** its policy can install plugins from git at a commit on its members' machines, on and locked in its workspaces ([Workspaces](workspaces.md#an-orgs-plugins)).

`silt plugin list` lists them, and Settings › Plugins in the web UI shows them by where they came from. Each has a page: what it does, how it works, its settings, what runs without asking, whether it loads, and its calls in every thread.

A plugin is on only in the workspaces you turn it on in: `silt plugin enable <id> --workspace <workspace>`, or its row's workspaces in Settings › Plugins.

## Every plugin is built the same way
Silt's own plugins are built exactly as anyone else's: the same manifest, the same API, the same host. A plugin's manifest, `silt-plugin.toml`, declares everything it can do:
- **its capabilities,** such as `http` to the hosts it names, `git`, `ssh` or a local MCP server's command, each with its reason;
- **its credentials,** which Silt adds to its requests to those hosts only;
- **its actions,** each a read or a change, allowed, asked or never by default;
- **its settings,** which Silt checks against what it says each is.

What a plugin doesn't declare, it can't do: it sees no files beyond its own folder, opens nothing but what its capabilities allow, and runs with limits on memory and time.

## Each call, shown before it runs
A plugin prepares each call first: a card that Silt draws, and the exact requests it will send. Your rules decide it:
- **allowed** runs it;
- **asked** shows you the card, in the thread and under Needs you, with every effect it lets through. Approving can also allow calls like it for the run, for the thread a while, or always, by a rule of yours;
- **never** refuses it, and a never from any rule wins.

The call then runs with only what was approved getting through: a request that differs by a byte is refused. What Silt can't check, a TCP connection, an SSH command or a local MCP server's work, is labelled so on the card, and an org can refuse it outright.

`silt plugin rule <id> <action> allow|ask|never` sets your rule for an action, with `--where field=pattern` for some calls only. A publisher you don't trust yet has each of its actions ask, whatever its defaults say; `silt plugin trust <publisher>` trusts it.

## Updates
A plugin added at a tag is offered the highest version tagged since; one added at a branch, its newest commit; one pinned to a commit stays there. Silt looks daily and when Settings › Plugins opens, and says what came in and what the new version asks for that the installed one doesn't. Nothing updates until you say so:

```sh
silt plugin update acme.deploys
```

shows the commits since, and asks you to agree to what it adds; `--yes` agrees. The update is built from the commit shown and no other.

## Removing one
`silt plugin remove <id>` removes a plugin and what it kept. Silt's own can only be turned off, and an org's stay as its policy says.
