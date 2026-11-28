# 0095: An agent's plan usage in the composer

- Status: accepted
- Date: 2026-11-17
- Amended by: [ADR 0134](0134-no-telemetry-and-an-offline-switch.md): plan usage can be turned off, and is off offline

## Context
People run agents on subscription plans: Claude Code on Pro or Max, Codex on a ChatGPT plan, Copilot on GitHub's. Each plan has limits, such as a 5-hour window, a week, or a month's premium requests. Each CLI shows them its own way: Claude Code's `/usage`, Codex's `/status`, Copilot's settings page. From Silt's web UI, the only way to know a limit is near is to hit it.

## Decision
**The composer shows the agent's plan usage as a ring** at the end of the agent's row: in a thread, New thread and New chat. It fills with the fullest limit, green (`ok`) below 75 %, then warn, then danger from 90 %. Opened, it lists each limit with its share used, when it resets and a bar, and links to the provider's own usage page.
- **The daemon reads it** (`GET /api/agents/{agent}/usage?thread=`, `Request::AgentUsage`). It uses the sign-in the agent's CLI keeps, in the agent's home for the thread's folder, so a workspace that gives an agent a home of its own (ADR 0065) shows that account's usage:
  - **Claude Code:** `.credentials.json`'s OAuth token, asking `api.anthropic.com/api/oauth/usage` as Claude Code's `/usage` does. Its `limits` list gives the 5-hour limit, the week's, and each weekly limit scoped to a model, named by the model; an answer without it is read from `five_hour` and the `seven_day` keys.
  - **Codex:** `auth.json`'s ChatGPT token, asking `chatgpt.com/backend-api/wham/usage`. Where that fails, as when the token has expired, the limits its newest session recorded (`token_count` events' `rate_limits`) stand in, marked as recorded.
  - **Copilot:** a GitHub token, asking `api.github.com/copilot_internal/user` for its quotas. Unlimited ones are left out. The token comes from `COPILOT_GITHUB_TOKEN`, `GH_TOKEN` or `GITHUB_TOKEN`, then the sign-in GitHub's Copilot editors keep (`github-copilot/apps.json`), then `gh auth token`.
  - **Antigravity:** runs the official `agy` binary in a PTY with a scratch `--gemini_dir` and a per-run `--settings` `statusLine` command (`AGY_CLI_DISABLE_AUTO_UPDATE=true`), reading the `quota` buckets and `plan_tier` JSON `agy` itself pipes to `statusLine`, then stopping `agy` at once. Silt never reads Antigravity's OAuth tokens or calls Google's internal endpoints directly.
- **Silt never refreshes a sign-in.** The CLI owns its tokens, and refreshing one rotates it under the CLI. An expired one says to run the agent once.
- **An answer is kept a minute** (three minutes for Antigravity, so `agy` isn't spawned often) per agent and home, and the composer asks at most once a minute. If a later ask fails, the last answer stands, marked as recorded.
- **Sans-I/O:** `silt_agents::usage` says what to ask and reads the answers; the daemon fetches. `SILT_CLAUDE_USAGE_URL`, `SILT_CODEX_USAGE_URL`, `SILT_COPILOT_USAGE_URL` and `SILT_ANTIGRAVITY_USAGE_COMMAND` point tests at stand-ins.
- **Nothing shows** for an agent with no plan limits to read: an API key, no sign-in, or an agent Silt doesn't read (Gemini, OpenCode, local).

## Consequences
- **These endpoints are each provider's own and undocumented.** A change on their side shows as no ring until Silt reads the new shape.
- **The usage is only the daemon's to read:** the hosted app has no agent's sign-in, so it shows no ring.
- **Claude Code on macOS keeps its sign-in in the Keychain,** which isn't read yet. **Copilot CLI's own sign-in in the system keyring isn't read either,** so Copilot's ring needs one of the tokens above.
