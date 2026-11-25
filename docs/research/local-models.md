# Local models for the three target machines (ADR 0092), November 2026

What to try first on each machine Silt is set up for, from public benchmarks, model cards and community reports rather than runs on the machines themselves. Silt keeps no list of models of its own (ADR 0092): people find models on Hugging Face from Settings › Local models or `silt model search`, so this is advice for the guide, not code. It dates quickly; recheck it before relying on it.

## The machines
- **RTX 5090.** 32 GB of GDDR7 in a desktop, 24 GB in a laptop. CUDA builds of llama.cpp need CUDA 12.8 or later and driver 570 or later for Blackwell (`sm_120`, `-DCMAKE_CUDA_ARCHITECTURES=120`); one build issue in early 2026 failed on MXFP4 kernels for `sm_120`, so a recent llama.cpp matters. The Vulkan release build also runs on it, slower.
- **Apple silicon.** Unified memory; the GPU may wire about three quarters of it by default (`iogpu.wired_limit_mb` raises it). Metal is in llama.cpp's macOS release build.
- **AMD Ryzen AI Max+ 395 (Strix Halo).** Up to 128 GB unified at about 256 GB/s, of which about 96 GB by default and up to about 120 GB with the GTT/TTM limits raised (`ttm.pages_limit`, `amdgpu.gttsize`) go to the GPU. Vulkan (RADV) is the steadier backend in 2026 reports; ROCm can be faster at empty context but drops more at long context, and maps only the BIOS VRAM unless unified memory is enabled. Its bandwidth suits mixture-of-experts models; large dense ones run slowly.

## What to try first
| Machine | First choice | Also good |
|---|---|---|
| RTX 5090, 24–32 GB | Qwen 3.6 27B (dense; `unsloth/Qwen3.6-27B-GGUF`, Q4_K_M 16.8 GB): the strongest at agentic coding in this size, SWE-bench Verified 77.2 and Terminal-Bench 2.0 59.3 on Qwen's card | Qwen 3.6 35B-A3B (MoE; `unsloth/Qwen3.6-35B-A3B-GGUF`, UD-Q4_K_XL 22.4 GB): about four times faster, 73.4 and 51.5; Devstral Small 2 24B (14.3 GB); Gemma 4 26B-A4B (16.9 GB) |
| Mac, 64–96 GB | Qwen3-Coder-Next 80B-A3B (`unsloth/Qwen3-Coder-Next-GGUF`, Q4 about 48 GB) | gpt-oss-120b (`ggml-org/gpt-oss-120b-GGUF`, 63 GB MXFP4) on 96 GB and up; the 5090's models with long context |
| Strix Halo, 128 GB | Qwen 3.5 122B-A10B (`unsloth/Qwen3.5-122B-A10B-GGUF`, UD-Q4_K_XL about 77 GB in three parts) | gpt-oss-120b (about 30–55 tokens/s reported on Vulkan); Qwen3-Coder-Next and Qwen 3.6 35B-A3B (about 60–100 tokens/s) for speed |

Newer releases keep arriving: Hugging Face's most downloaded GGUF list in November 2026 already had Qwen 3.8 27B. Abliterated variants, such as `huihui-ai/Huihui-Qwen3.6-35B-A3B-abliterated-MTP-GGUF`, are found and downloaded like any other.

## What makes them work as agents
- **Tool calls need `--jinja`** (Silt passes it) and a recent llama.cpp. Qwen 3.5 and 3.6 chat templates have had rendering bugs under llama.cpp, with community-fixed templates published; tool calls that fail to parse are the usual sign.
- **Thinking models** put their reasoning in a separate field with `--jinja`; Silt's agent shows it as the agent's thoughts.
- **Claude Code on a local model** compacts by `CLAUDE_CODE_AUTO_COMPACT_WINDOW`, which Silt sets to the model's context; values above 200k were reported ignored. One report says auto-compact stopped firing for third-party base URLs after Claude Code 2.1.161, leaving `/compact` by hand.
- **Context** costs memory: Silt starts a model with the most context, as a power of two up to 131k, that fits beside its weights, from the KV cache size its header gives.

## Sources
- Model cards and files: Hugging Face's API for each repository named above (sizes and SHA-256 from `?blobs=true`).
- [Qwen3.6-27B blog](https://qwen.ai/blog?id=qwen3.6-27b), [Qwen 3.6 27B vs 35B-A3B](https://www.aimadetools.com/blog/qwen-3-6-27b-vs-35b-a3b/), [benchmark results](https://zoliben.com/en/posts/2026-04-23-qwen-36-35b-vs-27b-benchmark-results/), [Gemma 4 vs Qwen 3.6 coding verdict](https://gist.github.com/hungson175/61a805368649c225026a69adf2ad87e0)
- [Best local coding models by VRAM](https://insiderllm.com/guides/best-local-coding-models-2026/), [24 GB coding agents](https://specpicks.com/reviews/best-local-llm-coding-agent-24gb-gpu-2026)
- [Strix Halo LLM notes](https://forum.level1techs.com/t/strix-halo-llm-inference-notes/249466/9), [Strix Halo 128 GB tests](https://blog.t1m.me/blog/local-llms-on-strix-halo-128gb-shared-ram), [Phoronix: ROCm vs Vulkan](https://www.phoronix.com/review/lemonade-vulkan-rocm/2), [Strix Halo setup guide](https://hogeheer499-commits.github.io/strix-halo-guide/)
- [NVIDIA Blackwell migration guide](https://forums.developer.nvidia.com/t/software-migration-guide-for-nvidia-blackwell-rtx-gpus-a-guide-to-cuda-12-8-pytorch-tensorrt-and-llama-cpp/321330), [llama.cpp sm_120 build issue](https://github.com/ggml-org/llama.cpp/issues/19662)
- [llama.cpp's Anthropic Messages API](https://huggingface.co/blog/ggml-org/anthropic-messages-api-in-llamacpp), [Fixed Qwen chat templates](https://hf-2025.pages.dev/froggeric/Qwen-Fixed-Chat-Templates), [auto-compact window cap](https://claudeissues.com/issue/57964-bug-claude-code-auto-compact-window-is-capped-at-the-hardcoded-model-context-win), [auto-compact on third-party providers](https://claudeissues.com/issue/65585-auto-compact-stopped-working-for-third-party-api-providers-since-v2-1-161)
