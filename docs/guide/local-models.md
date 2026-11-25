# Local models: agents on a model this machine runs

Silt can run a model on your own machine, with llama.cpp, and run agents on it: **Silt (local model)**, Silt's own agent, and Claude Code. Use it for quick prompts to a model of your choosing, such as an abliterated one, or for work on data that mustn't go to an online model, such as logs with personal data in them.

Nothing runs in the background until you use it. Silt starts the model on the first prompt that needs it, keeps one model loaded at a time, and stops it after ten idle minutes so the GPU's memory is free again.

## What you need
- **llama.cpp's `llama-server`.** Silt uses one on the `PATH`, or the one `SILT_LLAMA_SERVER` names when you start Silt. Without one, Settings › Local models offers **Download llama.cpp**, and `silt model runtime` does the same: Silt fetches llama.cpp's latest release for this machine, Metal on macOS and Vulkan on Linux, and checks it against the SHA-256 GitHub publishes for it. On an NVIDIA GPU, a CUDA build of your own on the `PATH` is faster.
- **Memory for the model.** `silt model list` shows the GPU memory `llama-server` finds, and each model shows whether it fits.

### Your machine
- **NVIDIA RTX 5090 (32 GB in a desktop, 24 GB in a laptop):** a CUDA build needs CUDA 12.8 or later and driver 570 or later for Blackwell; build llama.cpp with `-DGGML_CUDA=ON -DCMAKE_CUDA_ARCHITECTURES=120`, from a recent release.
- **Apple silicon:** the GPU may wire about three quarters of memory by default. `sudo sysctl iogpu.wired_limit_mb=<megabytes>` raises that until the next restart.
- **AMD Ryzen AI Max+ 395 (Strix Halo):** the Vulkan build works well. To give the GPU more than its default share of the unified memory, raise the kernel's GTT limit, such as `ttm.pages_limit` and `amdgpu.gttsize` on its command line, as your distribution documents. Mixture-of-experts models suit it; large dense ones run slowly.

## Find a model
You choose the model: Silt keeps no list of its own, which would be out of date by the time you read it. In Settings › Local models, **Find a model** shows the most downloaded GGUF models on Hugging Face, or what you search for. Open one to see its quantizations, each with its size, whether it fits here, and the context it would start with, and download the one you want. In a terminal:
```bash
silt model search qwen coder
silt model files unsloth/Qwen3.6-27B-GGUF
silt model pull unsloth/Qwen3.6-27B-GGUF:Qwen3.6-27B-Q4_K_M.gguf
```
`silt model pull` takes a GGUF on Hugging Face as `owner/repo:file.gguf`, fetching every part of a model split into several, or a GGUF file you have, which is used where it is. Abliterated models are found and pulled like any other.

A download resumes where it stopped, and every file is checked against the size and SHA-256 Hugging Face keeps for it before it's used. A model starts with the most context that fits beside it here, up to 131k tokens. `silt model rm <name>` removes a model; a file of your own stays where it is.

### What to try first (November 2026)
These date quickly; [the research behind them](../research/local-models.md) has the sources.

| Machine | First choice | Also good |
|---|---|---|
| RTX 5090 | Qwen 3.6 27B, Q4_K_M (`unsloth/Qwen3.6-27B-GGUF`) | Qwen 3.6 35B-A3B (`unsloth/Qwen3.6-35B-A3B-GGUF`), about four times faster; Devstral Small 2 24B; Gemma 4 26B-A4B |
| Mac, 64–96 GB | Qwen3-Coder-Next (`unsloth/Qwen3-Coder-Next-GGUF`), Q4 | gpt-oss-120b (`ggml-org/gpt-oss-120b-GGUF`) on 96 GB and up |
| Strix Halo, 128 GB | Qwen 3.5 122B-A10B (`unsloth/Qwen3.5-122B-A10B-GGUF`), UD-Q4_K_XL | gpt-oss-120b; Qwen3-Coder-Next or Qwen 3.6 35B-A3B for speed |

Tool calls depend on the model's chat template, which llama.cpp renders with `--jinja`. If an agent's tool calls fail to parse, a newer llama.cpp or a fixed template for that model is usually the cure.

## Silt's own agent
In a new chat or thread, pick **Silt (local model)** and its model. Its modes:
- **Chat:** it only talks.
- **Ask for everything**, **Ask before commands** and **Full access:** it also reads, lists, finds and searches files, edits and writes them, and runs commands in the project, asking as the mode and Silt's preset say. Before it touches a path outside the project, it asks.

Its conversations pick up again where they left off, and a long one is summarized as it nears the model's context.

## Claude Code on a local model
Choose a `local:<name>` model in Claude Code's model setting in the web UI, or in a terminal:
```bash
silt claude --model local:qwen3.6-27b-q4_k_m
```
Silt points Claude Code at the model through its daemon, with a key for that run only, turns off Claude Code's other traffic and its Bedrock, Vertex and Foundry settings, and keeps a proxy you've set out of the way. Your claude.ai sign-in isn't needed. Once Claude Code runs on a local model, its model can't be switched while it runs; start it again for another.

Silt tells Claude Code the model's context, so it compacts in time. Some Claude Code releases don't compact by themselves against a model that isn't Anthropic's; `/compact` then does it by hand. An `env` block in Claude Code's own settings files can override the variables Silt sets, so don't point `ANTHROPIC_BASE_URL` anywhere there.

## Local only workspaces
For data that mustn't reach an online model, make a Local only workspace for its projects:
```bash
silt workspace new Logs --local-only --folder ~/incidents
```
In it:
- **threads stay on this machine:** they never sync, and no server sees them, their agents' state or notifications about them. Local only can only be turned on for a workspace whose threads don't sync yet;
- **only local models answer:** Silt (local model), and Claude Code on a local model. Any other agent, or Claude Code on Anthropic's models, is refused;
- **an agent's history tools read only its own project's threads.** `silt workspace policy Logs --wide-history on` lets them read the whole workspace.

What Local only doesn't cover: the commands an agent runs and the web tools it has are governed as for any agent, by its permission prompts and Silt's preset. Under Full access, a command could still reach the network. Choose the preset to match, and turn on the agent's sandbox where it has one (see [Workspaces](workspaces.md)).

## Erase a thread
A Local only thread can be erased from this machine for good:
```bash
silt thread erase <thread>
silt thread erase <thread> --yes
```
The first says what goes; `--yes` erases it. The web UI has **Erase from this machine** in the thread's More menu. Erasing removes:
- the thread and its forks: their history, search entries and files;
- their managed checkouts, and a chat's own folder;
- the agents' own copies of the session: the transcripts Silt recorded terminal agents from, and Silt (local model)'s session files;
- whatever the model still holds of them in its cache.

Silt then rebuilds its store, so nothing of the thread stays in its files. An erasure a stop interrupts is finished the next time Silt starts.

Erasing can't reach copies you made, commits an agent made, which stay in the repository until `git gc` prunes them and wherever they were pushed, backups of the disk, or blocks an SSD keeps after a delete; encrypt the disk for those.
