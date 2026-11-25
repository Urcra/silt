# 0110: Suggested prompts in the composer

- Status: accepted
- Date: 2026-11-19

## Context
An agent can predict the prompt the person will send next, so it can show grey in the empty box for a key to take. Claude Code predicts one after each turn (`--prompt-suggestions`, the Agent SDK's `promptSuggestions`), as a `prompt_suggestion` message on its stream. Hooks and transcripts don't carry it, so Silt's capture under `silt claude` can't see it.

ACP has an unstable session update for it, `prompt_suggestions_update`, sent only to a client that says `session.promptSuggestions` in `initialize`. An empty list takes the suggestion back.
- **Claude's adapter** sends it from 0.109, over its ACP v2 core only. It sends none on a session's first turn, in plan mode, after an API error or at the usage limit, drops one that arrives while Claude works, and takes it back when work starts.
- **Codex's adapter**, `codex-acp`, which Qudiks runs too, predicts one from 2.5 with an extra request through Codex's `thread/sample`. It sends it only to JetBrains AIR, in `session_info_update._meta.promptSuggestions`. agentclientprotocol/codex-acp#763 asks for the standard update.
- **GitHub Copilot CLI** predicts none (1.0.114).

## Decision
**Silt takes the standard update from any agent, over v2.** `silt-acp` moves to `agent-client-protocol` 5.4 with `unstable_prompt_suggestions` and says `session.promptSuggestions` in its v2 `initialize`. The first suggestion's text becomes `Event::Suggestion`; an empty list, or one with no text, is `None`.

**Not over v1.** The SDK keeps a connection an agent answered in v1 only when Silt's v1 `initialize` says exactly what its v2 one does as v1, and its conversion carries no `promptSuggestions`. Saying it over v1 would start such an agent again. Claude's adapter sends suggestions over v2 alone, so nothing is lost today.

**No JetBrains AIR form.** Silt doesn't pretend to be another client to get Codex's. It waits for `codex-acp` to send the standard update.

**The daemon holds it for the run, not the thread's history.** `AgentStatus.suggestion` carries it to browsers. It's cleared when the person prompts and when the agent works, and one that comes while the agent works is dropped. It isn't recorded, synced or shown for an agent on another device: it's a hint for the person at the composer now.

**The composer shows it, and nothing is sent until the person sends.** It shows grey in the box while the box is empty. Tab or → puts it in the draft, to edit or send; typing anything else hides it. On a phone, which has no Tab, a Use button does the same.

## Consequences
- Claude's adapter is pinned at 0.109.0 or later.
- Codex, Qudiks and Copilot show none until their adapters send the standard update; an issue tracks each.
- The hosted app shows none, as it doesn't drive agents.
