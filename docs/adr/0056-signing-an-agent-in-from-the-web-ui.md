# 0056: Signing an agent in from the web UI

- Status: accepted
- Date: 2026-11-01
- Amends: [ADR 0014](0014-live-collaboration-and-silt-driven-agents.md) (agents the web UI starts), [ADR 0026](0026-hosted-mode-and-remote-control.md) (commands from elsewhere)
- Amended by: [ADR 0125](0125-antigravity-resumes-from-its-own-store.md) (an agent that answered v2 in v1 is asked its methods again in v1)

## Context
An agent CLI keeps its own sign-in: Claude Code under `~/.claude`, Copilot under `~/.copilot`. When it expires, the agent the web UI started fails, and the person had to open a terminal on that machine, run `claude` or `copilot`, and type `/login`. From the hosted app or a phone there was no way at all.

ACP has a way. An agent lists its `authMethods` when it starts and answers `auth_required` (-32000) when it needs one. Each method is either:
- an `agent` method, where the client calls `authenticate` (v2: `auth/login`) and the agent signs in itself;
- a `terminal` method, where the client runs the agent's own program with the method's `args` and `env` added, in a terminal. A zero exit is a success. Agents offer these only to a client that says it can (`clientCapabilities.auth.terminal`).

Silt ignored all of it, so `auth_required` was a plain error.

Run against both with scratch homes (Claude's adapter 0.94.0 and 0.99.1, Copilot 1.0.105), an expired sign-in standing in as a server that answers 401:
- **Claude's adapter** offers two `terminal` methods with no sign-in at all: `--cli auth login --claudeai` and `--cli auth login --console`. Once an expired one is there, it offers one instead: `--cli` alone, its whole terminal UI, where the person would type `/login`. A new session opens without a sign-in. The prompt fails: the adapter says "Failed to authenticate …", then answers `auth_required`, as the turn's error over v2. Its sign-in prints a URL and waits for "Paste code here if prompted >". The URL's page shows the code to paste back, so it works from any browser. A code it refuses makes it print "Login failed: …" and wait, rather than exit. It needs a terminal: without one it prints nothing.
- 0.94.0 answers Silt's v2 `initialize` with v1. So it never sees the v2 request's `auth.terminal`, and lists no methods. 0.99.1 answers v2 and lists them, by `methodId` as v2 names a method's id.
- **Copilot** offers one method in the older form, an `agent` method whose `_meta["terminal-auth"]` names a program (its own binary, by full path) and `args` (`login`). It answers `auth_required` to `session/new`. `copilot login` signs in through the browser with a callback on `127.0.0.1` on a desktop, which a phone can't reach. With `--device-code` it uses GitHub's device flow, a URL and a code to type there, which works from anywhere.

## Decision
**Silt signs agents in the way ACP says, from a card in the thread.**
- Silt tells agents it can run `terminal` methods, keeps the methods an agent lists, and reads `auth_required` from `session/new`, `session/load` and a prompt as the agent needing a sign-in, not as an error.
- The thread then shows a card: "<agent> needs you to sign in", with a button for each method, by its name and description. The run's state is `awaiting_permission`, so the thread is among those that need the person.

**A `terminal` method runs on the agent's machine, in a pseudo-terminal, and the card shows what matters in it.**
- The daemon runs the method's program: the agent's own command with the method's `args` added, or the program `_meta["terminal-auth"]` names. It runs with the agent's environment ([ADR 0016](0016-untrusted-peer-content.md)) and the method's `env`, in the thread's checkout.
- From its output, without escape codes, the card shows the first web link, the code to enter there when it prints one (a device code such as `ABCD-1234`), and a box for a code to paste back when the program asks for one. The last lines of output show under a fold, for a program that asks something else.
- Two adjustments for known agents, as with `presents_plans`:
  - Copilot's login is run with `--device-code`, as its browser flow can't finish anywhere but the machine itself.
  - Claude's `--cli` method is offered as the two `auth login` methods its adapter lists before any sign-in. They sign in the same way and ask only for the code, where a card can't drive a whole terminal UI.
- A pasted code goes to the program's input, followed by Enter. Once a code was typed, a last line that says the sign-in failed ends the program, as Claude's waits after one. Cancel ends it too.
- When the program exits with 0, the agent is started again, loading its session, and the prompt that failed is sent again. It isn't recorded a second time: the log has it already. Any other exit leaves the card, saying the sign-in failed, with its buttons to try again.

**An `agent` method starts the agent again, signing in first.** Silt calls `authenticate` before it opens the session, and the agent does what it does to sign in, on its machine. The card says so. No agent probed uses one.

**From the hosted app and phones, through the control channel.** The sign-in and what the card shows travel as the agent's other state does ([ADR 0026](0026-hosted-mode-and-remote-control.md)): in the agent's report to the server and its live view, and as `RemoteCommand::SignIn`, `SignInCode` and `SignInCancel` from the server to the daemon. The wire format changes, so `PROTOCOL` goes to 9 (8 went to [ADR 0055](0055-reviewing-a-threads-changes.md)).

**Signing in needs "start and stop agents".** Signing a machine's agent in decides which account its work goes to. A device that could sign it in to another account could read whatever the agent did next there. So the device needs `run_agents`, as starting and stopping an agent does, not `answer_permissions`. A paired phone doesn't have it unless its person gives it ([ADR 0028](0028-notifications-and-phones.md)).

**What a sign-in shows isn't recorded.** The URL, the code and the program's output stay in the daemon's memory and the agent's report. They aren't in the log, so they never sync, and they aren't searchable. A pasted code is a command like any other relayed one, kept by the server until it's delivered. Both flows bind their code to the program that asked for it: Claude's with PKCE, whose verifier never leaves the program, and the device flow's to the device code the program holds. So a code read off the server signs nothing in.

## Consequences
- Signing in never needs a terminal on the agent's machine, from any of the person's devices that may start agents there.
- What the card shows is read from the login program's output. A program that prints a link or a code differently still runs, and its output shows under the fold, but the card may miss the link or the code. The probes and their tests pin what Claude and Copilot print now.
- A pasted code is kept by the server until the daemon takes it. That is safe only because of the binding above. A future method that pastes a token instead of a code would need to bypass the relay.
- An agent signed in to a different account, or a token in the environment that has expired, still fails the same way, and the card offers the same sign-in.
- An agent that answers v2's `initialize` with v1, as Claude's adapter 0.94.0 does, may list no methods. The card then says to sign it in on its machine. Setting a token for agents (`CLAUDE_CODE_OAUTH_TOKEN`, `COPILOT_GITHUB_TOKEN`) from the web UI is left for later.
