# 0103: The GitHub plugin

- Status: accepted
- Date: 2026-11-18
- Amends: [ADR 0093](0093-silt-sandboxes-the-agents-it-starts.md) (plugins' rules)
- Amended by: [ADR 0104](0104-one-plugin-model.md): GitHub is a WebAssembly plugin shipped with Silt, on the `http`, `git`, `agent-files` and `thread-read` capabilities, rather than code in the daemon

## Context
A sandboxed agent reaches GitHub today with a `silt credential` the gateway adds, such as `gh auth token`'s. Then it can do anything the token can (push to `main`, force-push, merge, delete a branch) and nothing asks first. People want to choose, per repo, what runs without asking (clone, fetch, read issues) and what waits for them (push, open a pull request, merge).

Classifying the agent's own `git` and `gh` at the gateway would mean reading git's smart-HTTP and every REST and GraphQL call a script might make, and holding requests open while a person decides, which agents' tool timeouts don't allow.

## Decision
**GitHub is reached only through the GitHub plugin's tools,** Silt's first plugin of its own, run in the daemon.
- **Environments are matched by repo,** such as `acme/*`, each with its credential and rules, so a work org and personal repos differ. GitHub Enterprise is another environment with its own API and git URLs.
- **Each action has its rule:** allowed, asked, or never.
  - Reads: `clone`, `fetch`, `repo`, `file`, `issues`, `pulls`, `checks`, `search`.
  - Changes: `push`, `push_default` (the default or a protected branch), `push_force`, `branch_delete`, `issue_write`, `comment`, `pr_create`, `pr_update`, `review`, `merge`, `workflow_run`, `release`.

  The defaults:
  - reads go;
  - `push_default` and `merge` are asked each time;
  - `push_force` and `release` are never;
  - the rest are asked.

  A tool with no action left isn't offered.
- **Silt never runs git in, or trusts the config of, a repo the agent can write:**
  - **Mirrors:** each thread has bare mirrors Silt owns, mounted read-only in its sandboxes. `clone` and `fetch` update a mirror with the credential given on git's command line, and the agent clones or fetches from it itself.
  - **Getting the agent's commits:** a push first fetches the agent's commit into the mirror. That's the only git command run against the agent's repo, and `upload-pack` is safe on untrusted repos.
- **A push is bound to its commits:**
  - Silt's copy of the request holds the repo, the remote branch, the commit it moves from and the one it moves to.
  - The approval card shows the commits and their diffstat, read from the mirror.
  - Silt pushes exactly that commit with `--force-with-lease` at the commit it moved from, so what goes is what was approved, or nothing does.
- **Other changes** are REST calls, built and stored by Silt and run once from that copy.
- **The sandbox holds no GitHub token.** With the plugin on, the gateway adds no credential for GitHub's hosts, and `GH_TOKEN`, `GITHUB_TOKEN`, `GH_ENTERPRISE_TOKEN` and `COPILOT_GITHUB_TOKEN` never pass into a sandbox.

**Plugins' rules grow for it, for every plugin:**
- a rule can be **never** as well as allowed or asked;
- an environment can set a rule per action, which for an MCP plugin is per tool;
- a plugin can choose its environment from the call, here by repo, rather than the agent naming it.

## Consequences
- **The agent can't use `gh` or push with its own `git`.** Anonymous reads of public repos still work, as they're harmless.
- **Clones live in the thread's work area,** not the person's folders. A folder grant (ADR 0102) can bring one of the person's in.
- **The forge bridge's GitHub requests** move into a pure crate that the plugin shares.
- **An action GitHub adds later** is a change of this plugin, not a rule people write.
