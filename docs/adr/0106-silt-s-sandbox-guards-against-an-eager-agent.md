# 0106: Silt's sandbox guards against an eager agent, and the person reviews its work

- Status: accepted
- Date: 2026-11-18
- Amends: [ADR 0093](0093-silt-sandboxes-the-agents-it-starts.md) (Silt sandboxes the agents it starts)

## Context
ADR 0093 says what Silt's sandbox keeps out, but not whom it keeps out. Probing it turned up two kinds of finding:
- **Reach it shouldn't have:** credentials in the shell's variables and the agent's folder, other projects' sessions, the kernel keyring, another thread's sessions through the run's channel, and IPv6 forms of loopback through the gateway.
- **Code that runs later, outside:** a `.git` swapped for one whose config runs a command, an agent's settings with a hook in them, and caches whose contents a later build runs.

The second kind has no end. Whatever an agent writes into its checkout is code the person runs: `npm test`, `cargo test`, a build script, a dependency it added. A sandbox can't tell good code from bad code the person later chooses to run, so it can't stop an adversary who controls the agent from getting code run on the host.

## Decision
**The sandbox guards against an agent that is eager, not hostile.** An agent trying very hard to finish its task, such as one that goes looking for the production database's password to debug a bug, must find no way to do it. Production, credentials, the machine and other projects stay out of reach.

**It doesn't guard against an adversary running code in it.** A prompt injection that makes the agent write a malicious test gets that test run when the person runs it. That's out of scope, and the docs say so.

**The person reviews the agent's changes before trusting them.** The folders the agent can write, the checkout, the git dir's objects and refs, the shared build caches and the agent's own folder, are its work and the person's to review, as they would a colleague's pull request.

**What runs without the person choosing to stays pinned.** The line falls between code the person runs and configuration that runs by itself. git's `config`, `hooks` and `commondir`, `.git` itself, `.silt`, and the agent's settings, memory, plugins, hooks and MCP servers are run by git, Silt or the agent later without anyone running the code on purpose. A review of the diff wouldn't catch them, so they stay read-only, or the run gets its own copy.

## Consequences
- **Findings are triaged by this line.** Reach an eager agent could use is a bug. Code a hostile agent could get run by writing the work isn't, unless it would run without the person choosing to.
- **Caches stay shared and writable,** for build speed. A build reads what a run left in them, which the guide says.
- **The guide has a page on it,** [Silt's sandbox: what it protects, and what it doesn't](../guide/sandbox.md), saying what the sandbox is for, what it isn't, and which folders to review.
