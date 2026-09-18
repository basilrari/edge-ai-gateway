# bench_v1 LLM comparison (Jetson)

Local measurement of candidate GGUFs against the 150-case `bench_v1` set, scored as BFCL-style AST match (`full_match`) plus a separate flight-controller acceptance count. Runs were on this Jetson, through the production `/infer` path, with SITL on `labpc`.

**Serving choice: `Qwen3.5-2B-Q5_K_M`.** MiniCPM5-2B scored higher on gold (133 vs 124) but is not the production model: it often returns empty content on reject cases instead of the `invalid_request` envelope, and it emits malformed JSON on the longest `goto_location` chains. Qwen 2B is the model that actually follows the tool JSON contract.

The 0.8B Qwen stays on disk as a smaller fallback. The other sweep GGUFs (MiniCPM5, LFM2.5) were deleted after this write-up.

Machine-readable copy of the same numbers: [llm-bench-v1-scores.json](llm-bench-v1-scores.json). Use the `qwen_08b_vs_2b_use_this_pair` object for 0.8B vs 2B — those two rows are the same harness.

## Protocol

- Corpus: 150 cases (`simple` 50, `multi-step` 60, `reject` 40).
- Gold = `full_match` (function name + arguments). Flight-controller (FC) column is drone-tool cases the FC accepted, counted separately.
- Gateway `max_tokens` during the later runs: 2048 (was 256). Qwen 2B never needed more than 200 tokens on a replay of all 150 inputs; the higher cap only mattered for reasoning models.
- Prefill / decode: first request after load, with the real SAR system prompt (~1255 tokens). Warm numbers understate prefill and are not used here.
- Context: 16384 unless noted.

## Scores

### Sweep 3 — default templates, then `enable_thinking=false` only if the default produced no JSON

| model | gold | simple | multi | reject | FC | prefill tok/s | decode tok/s | notes |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| Qwen3.5-2B Q5 | 123/150 | 48/50 | 53/60 | 22/40 | 91/92 | 1063.1 | 27.5 | default template |
| Qwen3.6-0.8B Q5 | 78/150 | 38/50 | 24/60 | 16/40 | 80/92 | 1243.7 | 36.8 | default template |
| MiniCPM5-1B Q4 | 71/150 | 26/50 | 9/60 | 36/40 | 77/92 | 2364.7 | 56.3 | thinking disabled; otherwise empty `content` |
| LFM2.5-1.2B-Instruct Q4 | 43/150 | 10/50 | 3/60 | 30/40 | 68/92 | 2285.8 | 63.4 | often invalid JSON (`json_valid` 58/150) |
| MiniCPM5-2B Q4 | — | — | — | — | — | — | — | load failed: `unknown pre-tokenizer type: 'minicpm5'` on llama.cpp b8185 |
| LFM2.5-1.2B-Thinking Q4 | — | — | — | — | — | 2270.3 | 62.9 | empty `content` at `max_tokens` 256 |
| LFM2.5-8B-A1B Q4 | — | — | — | — | — | — | — | `cudaMalloc` of 4908 MiB failed |

Sweep-3 summary rows for Qwen recorded *warm* prefill (prompt_n 516). The table above uses the later cold pass for those two models (prompt_n 1255). MiniCPM5-1B / LFM Instruct gold numbers are from the 150-case run; their sweep-3 summary prefill was also warm and is not listed.

### Sweep 4 — MiniCPM5 pre-tokenizer patched into the local llama.cpp build; gateway cap 2048

| model | gold | simple | multi | reject | FC | json_valid | prefill tok/s | decode tok/s |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| MiniCPM5-2B Q4 | 133/150 | 45/50 | 53/60 | 35/40 | 86/92 | 104/150 | 1207.8 | 31.9 |
| Qwen3.5-2B Q5 | 124/150 | 48/50 | 53/60 | 23/40 | 91/92 | 126/150 | 1064.6 | 27.4 |
| LFM2.5-1.2B-Thinking Q4 | 47/150 | 5/50 | 5/60 | 37/40 | 28/92 | 22/150 | 2231.8 | 63.1 |

Engine control: Qwen 2B was 123/150 before the vocab patch and 124/150 after (**+1**). The 0.8B varied 82 → 81 → 78 across earlier identical-config runs, so ± a few cases is noise. The patch did not move Qwen.

## Why Qwen 2B, not MiniCPM5-2B

MiniCPM5-2B's +9 gold is almost entirely reject cases (35/40 vs 23/40). All 35 of those passes produced **no parseable JSON**; the gateway maps empty content to `invalid_request`, which matches gold. It never emitted the prompt's `{"category":"none","name":"invalid_request"}` envelope. The same silence failed two cases that needed a tool (`interrupt the mission`, `guided mode then hold position`).

Four more MiniCPM5-2B failures are malformed JSON on coordinate chains, extra `}` where `]` belongs:

```text
got:  ..."alt_m":15}}}]}
need: ..."alt_m":15}}]}
```

Replayed at 2048 / 3072 / 4096 tokens: byte-identical, `finish=stop`. Not a token-cap artifact. LFM-Thinking has the same defect on most tool calls (`{"tasks":[{"category":"drone","name":"arm"}}}`).

Qwen 2B: `json_valid` 126/150, FC 91/92, explicit refusals, no extra llama.cpp patch. That is the production trade.

## 0.8B vs 2B (same harness: sweep 3)

| | Qwen3.6-0.8B Q5 | Qwen3.5-2B Q5 | 2B − 0.8B |
|---|---:|---:|---:|
| gold | 78/150 | 123/150 | +45 |
| simple | 38/50 | 48/50 | +10 |
| multi-step | 24/60 | 53/60 | +29 |
| reject | 16/40 | 22/40 | +6 |
| json_valid | 116/150 | 127/150 | +11 |
| FC | 80/92 | 91/92 | +11 |
| prefill tok/s (cold) | 1243.7 | 1063.1 | 2B slower |
| decode tok/s (cold) | 36.8 | 27.5 | 2B slower |

The gap is multi-step. 0.8B run-to-run on this box was 82 → 81 → 78; 2B was 123 then 124. Treat ±4 as noise; +45 is not. 0.8B file kept: `Qwen3.6-0.8B-Q5_K_M.gguf`.

## Serving notes

- Binary: local llama.cpp b8185 (`2afcdb9`, 2026-03-02), `-ngl 99 -c 16384`.
- File: `Qwen3.5-2B-Q5_K_M.gguf`.
- Gateway `max_tokens`: 2048 (this repo). Instruct models stop well under that; 256 used to zero out reasoning models.
- MiniCPM5-2B needed upstream llama.cpp PR [#23384](https://github.com/ggml-org/llama.cpp/pull/23384) (`minicpm5` pre-tokenizer). That patch is **not** in this repository; it was a local llama.cpp edit, then reverted after the GGUF was deleted.

## Out of scope

- Repairing `}}}` → `]}` in the gateway parser.
- Prompt work to make MiniCPM emit explicit `invalid_request`.
- Rebuilding llama.cpp past b8185.
- The 8B MoE (does not fit this 16 GB unified-memory box next to TensorRT + the desktop).
- Re-running the deleted GGUFs.
