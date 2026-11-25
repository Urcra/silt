# 0092: Local models, run by Silt, and workspaces that stay on the machine

- Status: accepted
- Date: 2026-11-14
- Amends: [ADR 0017](0017-acp-agents-only.md) (Silt has its own agent again, for local models only), [ADR 0065](0065-workspaces.md) (a Local only rule, and a model among a workspace's defaults)

## Context
People want two things an online model can't give them:
- **Quick prompts to a model of their choosing,** such as an abliterated one, without setting anything up.
- **Work on data that may not leave the machine,** such as logs with personal data under the GDPR.

**What works today.** Agents bring their own providers (ADR 0017). A workspace's `env` can point Claude Code at Unsloth Studio, Ollama or LM Studio (`ANTHROPIC_BASE_URL`). That falls short in two ways:
- **The person runs the model server,** in the background, and keeps it in step with the agent.
- **Nothing makes sure the data stays on the machine:**
  - the agent sends traffic of its own: telemetry, update checks, web tools, and the commands it runs;
  - Silt syncs the thread, and the transcript holds what the agent read;
  - redaction catches secrets, not names, emails or addresses.

**The machines** (November 2026):

| Machine | Memory for the model | llama.cpp backend | What fits |
|---|---|---|---|
| Laptop RTX 5090 | 24 GB GDDR7, about 900 GB/s | CUDA 12.8 or later for Blackwell (`sm_120`, driver 570+), or Vulkan | a mixture-of-experts coder of about 30B (3B active), or a 24–32B dense model, at 4 bits; 32k–64k of context with a quantized KV cache |
| Apple silicon | unified; the GPU may wire about three quarters of it by default (`iogpu.wired_limit_mb`) | Metal | with 64 GB or more, MoE models of 100B and over, such as gpt-oss-120b, with long context |
| AMD Ryzen AI Max+ 395 (Strix Halo) | up to 128 GB unified, about 256 GB/s | Vulkan (RADV): steadier than ROCm at long context in 2026 tests, and it maps the whole GTT; the amdgpu/TTM limits need raising for large models | MoE models; the bandwidth makes large dense ones slow |

**llama.cpp's `llama-server`**:
- serves OpenAI's chat API and, since January 2026, Anthropic's Messages API, which Claude Code uses through `ANTHROPIC_BASE_URL`;
- calls tools through the model's own chat template (`--jinja`);
- reuses a cached prompt prefix, so an agent's long system prompt is read once;
- lists the devices it sees and their memory (`--list-devices`).

**ADR 0017 removed `silt harness`** because owning an agent means owning everything it executes:
- a sandbox;
- file tools that resist symlinks and races;
- redaction.

The harness had its own namespace sandbox, which was Linux only, and the 2026-10-20 review found two critical findings in it.

## Decision
**Silt runs the model itself.** The daemon manages one `llama-server`:
- **Where it comes from:** `$SILT_LLAMA_SERVER`, else one on the `PATH`, else llama.cpp's own build. When the person asks, Silt fetches the latest release from GitHub, checks it against the SHA-256 GitHub publishes for it, and unpacks it whole into `$SILT_HOME/runtimes/llama.cpp/<tag>/`. A pinned release would be out of date like a catalog; GitHub's release is the source either way.
  - Metal on macOS, Vulkan on Linux. The Vulkan build serves both NVIDIA and AMD.
  - On NVIDIA, a CUDA build on the `PATH` is preferred, as it's faster. The guide says how to build one for Blackwell.
- **Run as a program, not linked,** as `silt-voice` is (ADR 0061):
  - llama.cpp ships its GPU builds per vendor, and Silt's musl releases can't link them;
  - a crash in native code ends a turn, not the daemon.
- **Started on demand:** on the first turn that needs a model, and stopped after 10 idle minutes, so the GPU's memory is free for other work.
- **One model loaded at a time.**
  - Threads on the same model share its slots.
  - A turn on another model waits for the running turn, then the model is swapped. The thread says so while it waits.
- **The machine's budget** comes from `--list-devices`.

**Agents reach it through the daemon's model gateway.**
- `llama-server` listens on loopback with an `--api-key` drawn at each start, which never leaves the daemon.
- The gateway, also on loopback, passes on OpenAI's chat API, Anthropic's Messages API, the model list and `/props`.
- **Each run gets a token of its own,** revoked when the run ends.
- **It queues turns and swaps models.**
- **It remembers which threads `llama-server` holds a cached prompt for,** so erasing one can clear it.

**Models** go to `$SILT_HOME/models/llm/`, again only when the person agrees.
- **The person chooses; Silt keeps no list.** A catalog in the binary was dropped before release: it was out of date before it shipped (a newer Qwen was Hugging Face's most downloaded GGUF already). Instead:
  - **finding one:** Hugging Face's GGUF text-generation models, the most downloaded first, or a search (`silt model search`, Settings › Local models);
  - **choosing one:** a repository's quantizations (`silt model files`), each with its size, whether it fits here, tight, or doesn't, and the context it would start with; a split model's parts are one;
  - **any GGUF** as `owner/repo:file.gguf`, or a file the person has, such as an abliterated model.
- **Checked:** every file against the size and SHA-256 Hugging Face keeps for it; a file without one isn't offered.
- **Its context is sized to fit:** the most, as a power of two up to 131k and no more than the model was trained for, that fits beside its weights by the KV cache size its header gives.
- **What to try first on each machine** is advice in the guide, dated, with its research in `docs/research/local-models.md`, not code.
- **Where to manage them:** `silt model list|search|files|pull|rm|runtime` and Settings › Local models.

**Claude Code on a local model.** A model is one of a workspace's defaults, and a new thread can choose another. `local:<name>` sets, for that run only:
- `ANTHROPIC_BASE_URL` to the gateway;
- `ANTHROPIC_AUTH_TOKEN` to the run's token;
- `ANTHROPIC_MODEL` and the small and fast model both to the local model;
- `CLAUDE_CODE_AUTO_COMPACT_WINDOW` to the model's context, so it compacts in time;
- `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`;
- the workspace's `CLAUDE_CONFIG_DIR`, so the person's claude.ai sign-in is neither needed nor used.

Some Claude Code releases don't compact by themselves against a model that isn't Anthropic's; the guide says `/compact` does it then.

**Silt's own agent, `local`, for chats and work without Claude Code.** It runs only on local models; online model APIs stay with agents that bring their own, as ADR 0017 decided.
- **An ACP agent in the `silt` binary** (`silt agent local`). The daemon starts it like any ACP agent, so:
  - it's recorded, resumes and forks like the others;
  - it goes through presets, permission requests and the workspace's rules.
- **How it reaches the model:** the gateway's OpenAI chat API, with tools.
- **Tools:** read, list, glob, grep, edit, write, and run a command.
- **Modes, mapped to presets (ADR 0048).** Its mode ids are Claude Code's, so the mapping already knows them:
  - `chat`: no tools, for quick prompts;
  - `default`: ask for everything;
  - `acceptEdits`: ask before commands;
  - `bypassPermissions`: full access.
- **Context:** it compacts itself as it nears the model's context size. Silt keeps the whole history, as for any agent.
- **Restricted like any other agent, and no more.**
  - Its file tools act inside the project as its mode allows, and ask before touching a path outside it. Silt answers those requests by the preset like any agent's, so under Full access they go through.
  - Its commands run as the person, under the same permission prompts.
  - A workspace's sandbox and isolated rules (ADR 0065) apply to it as to any ACP agent.
- **What ADR 0017 objected to, and how this differs:**
  - **No sandbox of Silt's own.** What a command may do is decided by permission prompts, as for the other agents.
  - **No redaction** is needed: it reaches only a local model.
- **Its core is Sans-I/O:** the conversation and tool calls in, actions out. It's tested against a scripted model server in `silt-fake-agent`.

**Local only: a workspace rule that keeps what Silt holds on the machine.**
- **It implies:** no server and a locked home, so nothing syncs (threads, files and chats).
- **It doesn't imply the sandbox or isolated rules.** Those stay the person's choice, as for any workspace.
- **Agents allowed:** `local`, and `claude` only on a local model. Switching a running agent to an online model is refused.
- **Nothing about its threads reaches a server** that other workspaces are signed in to:
  - no agent reports;
  - no notifications with their content;
  - no remote control, and its agents aren't offered to start from elsewhere.
- **Downloads** of models and of `llama-server` are made by the daemon, and carry nothing from the workspace.
- **The web UI marks** these workspaces and their threads Local only.
- **History tools see only the thread's own project.**
  - An agent's `silt mcp` tools, `local`'s included, answer only for threads of the project the agent runs in, not the rest of the workspace. Other workspaces see nothing of it.
  - A workspace can widen this to all its projects (`silt workspace policy <workspace> --wide-history on`). It's never wider than the workspace.
  - The person reads every thread in the web UI and the desktop app as usual.

**Local only threads can be erased.** `silt thread erase <thread>` (with `--yes` to go ahead), and Erase in the thread's menu, each saying what goes before it's confirmed.
- **What `silt thread purge` keeps, erasure doesn't.** Purge keeps what forks build on, so erasing a thread erases its forks too. A merge into another thread is that thread's content, and stays with it.
- **What goes:**
  - from `silt.sqlite`: the family's deltas and snapshots, its sessions and their offsets, its search index entries, its prompt queue and personal state;
  - its blobs that no other thread references;
  - its managed checkouts under `<repo>/.silt/checkouts/`;
  - the agent's own copy of the session: the transcripts Silt recorded a terminal agent from, with the folder beside each, and `local`'s session file;
  - anything `llama-server` holds of it, by restarting it if one of its turns is cached.
- **Then the store is compacted:** the search index merged, so it keeps no trace of the deleted rows, then `VACUUM`, which rebuilds the file from what's left, and a WAL checkpoint. Deleted pages stay in neither file.
- **It survives a crash.** A marker written first (`erasing/<thread>.json` in the data dir) lists what goes; every step can run again, and the daemon finishes an erasure at its next start.
- **Erasing a whole workspace** erases each of its threads, then the workspace.
- **What erasure can't reach,** which the guide says: copies the person made, commits the agent pushed, backups of the disk, and blocks an SSD keeps after a delete. Disk encryption covers the last two.
- The daemon's log never carries thread content, so it needs no erasing. A test checks this.

**Where it lives:**
- `silt-llm`, a new shell crate: finding models on Hugging Face, downloads, the device budget, fetching llama.cpp, and `llama-server`'s lifecycle;
- `silt-local`: the agent, its pure core and its tools;
- the gateway and the `/api/models` routes in the daemon;
- the Local only rule in `silt-workspace`.

Only the daemon's UI offers local models: the hosted app has no machine to run them on.

## Consequences
- **No model server for the person to run.** Choosing a model in a chat downloads it once, then it works. A server of their own (Unsloth Studio, Ollama, LM Studio) can still be used through a workspace's `env`, outside Local only.
- **Silt owns an agent again,** and with it a security surface: its file tools and its commands. They get a security review before release.
- **What Local only promises is about Silt.** Nothing Silt holds of these threads leaves the machine, and only local models answer. What an agent's commands or web tools reach is governed as for any agent: its permission prompts, its preset, and the sandbox if the workspace requires one. The guide says so plainly.
- **Compatibility moves with three projects.**
  - **llama.cpp** is pinned, and bumped like a dependency.
  - **Claude Code and the Anthropic endpoint** are checked again with each Claude Code release.
  - **Tests** run without a GPU: a scripted model server, Hugging Face and GitHub stand in. The real backends weren't run; the research stands in for that, and `scripts/local-model-probe.sh` checks a machine for whoever wants to.
- **The hardware needs setting up,** and the guide covers it: Strix Halo's GTT limits, Apple's wired limit, and a CUDA build for Blackwell.
- **Out of scope:**
  - native Windows, the hosted app and phones;
  - a terminal front end for `local`: the web UI and the desktop app are enough;
  - an automatic expiry of Local only threads, which can come later on top of erasure.
- **What Local only can't stop:** the person copying text out, what an agent's commands do once the person allowed them, and other programs on the machine.
