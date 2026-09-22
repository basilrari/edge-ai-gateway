# bench_v1 LLM comparison (Jetson)

Local measurement of candidate GGUFs against the 150-case `bench_v1` set, scored as BFCL-style AST match (`full_match`) plus a separate flight-controller acceptance count. Runs were on this Jetson, through the production `/infer` path, with SITL on `labpc`.

**Serving choice: `Qwen3.5-0.8B-sar-sft-Q4_K_M` (FT e3, think-off).** Sweep 3/4 originally picked orig Qwen3.5-2B Q5 over MiniCPM5-2B (empty-reject JSON, extra `}` on long `goto_location` chains). Sweep 5/6 then beat orig 2B with this 0.8B SFT.

Orig 0.8B Q5 and orig 2B Q5 stay on disk as fallbacks. The other sweep GGUFs (MiniCPM5, LFM2.5) were deleted after this write-up.

There is **no Qwen3.6 0.8B or 2B release**. Orig 0.8B is `Qwen3.5-0.8B-Q5_K_M.gguf`. Orig 2B is `Qwen3.5-2B-Q5_K_M.gguf`. FT 0.8B is `Qwen3.5-0.8B-sar-sft-Q4_K_M.gguf`.

Later SAR JSON LoRA/full SFT numbers (Smol 360M, Falcon-H1-Tiny 90M, Qwen 0.8B/2B think on/off) are in **Sweep 5**. That run is **llama-server only**, not `/infer`, and has **no FC column**. Do not mix those gold counts with sweep 3/4.

**Sweep 6** is FT 0.8B e3 on a **split-apply** harness: llama-server for JSON, then gateway `ApplyTool` one step at a time with FC ACK and a per-case reset. Not `Infer` auto-apply. Do not mix its gold with Sweep 5 or sweep 3/4. After a GoPro overlay, exec is **132/150**; gold is still **134/150**.

Machine-readable copy: [llm-bench-v1-scores.json](llm-bench-v1-scores.json). Use `qwen_08b_vs_2b_use_this_pair` for the `/infer` 0.8B vs 2B pair. Use `sft_runs` for Sweep 5. Use `split_apply_runs` for Sweep 6.

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
| Qwen3.5-0.8B Q5 | 78/150 | 38/50 | 24/60 | 16/40 | 80/92 | 1243.7 | 36.8 | default template |
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

| | Qwen3.5-0.8B Q5 | Qwen3.5-2B Q5 | 2B − 0.8B |
|---|---:|---:|---:|
| gold | 78/150 | 123/150 | +45 |
| simple | 38/50 | 48/50 | +10 |
| multi-step | 24/60 | 53/60 | +29 |
| reject | 16/40 | 22/40 | +6 |
| json_valid | 116/150 | 127/150 | +11 |
| FC | 80/92 | 91/92 | +11 |
| prefill tok/s (cold) | 1243.7 | 1063.1 | 2B slower |
| decode tok/s (cold) | 36.8 | 27.5 | 2B slower |

The gap is multi-step. 0.8B run-to-run on this box was 82 → 81 → 78; 2B was 123 then 124. Treat ±4 as noise; +45 is not. Orig 0.8B file: `Qwen3.5-0.8B-Q5_K_M.gguf`.

## Sweep 5 — SAR JSON SFT (llama-server, not `/infer`)

Same 150 `bench_v1` gold, scored on this Jetson through `llama-server` `/v1/chat/completions` (temperature 0). **No gateway, no drone apply, no FC column.** System prompt is the live SAR prompt used at train time. Train set is 750 synthetic chat rows; **none of the 150 bench prompts**.

Thinking-off is the comparable mode (Qwen `enable_thinking=false`; Smol/Falcon have no thinking). Qwen `max_tokens` 2048; Smol/Falcon 256. Prefill/decode in the table are **cold**, first request after load, with that SAR prompt.

SFT recipe unless noted: LoRA r=16 α=32 `all-linear`, 3 epochs, lr 2e-4, on an RTX PRO 5000 Blackwell. Falcon-H1-Tiny cannot LoRA `out_proj` (PEFT Mamba block), so that run is a **full** 91M finetune, lr 1e-4, batch 1.

### Think-off

| model | gold | json | simple | multi | reject | cold prefill | cold decode | s/case |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| orig Qwen3.5-0.8B Q5 | 79/150 | 142/150 | 38/50 | 29/60 | 12/40 | 1440 | 38.4 | 1.09 |
| **FT Qwen3.5-0.8B Q4 e3** | **135/150** | 150/150 | 49/50 | 50/60 | 36/40 | 1551 | 41.1 | 1.01 |
| orig Qwen3.5-2B Q5 | 120/150 | 150/150 | 48/50 | 52/60 | 20/40 | 1056 | 27.0 | 1.87 |
| orig SmolLM2-360M Q4 | 28/150 | 132/150 | 1/50 | 2/60 | 25/40 | 3918 | 77.4 | 0.52 |
| FT SmolLM2-360M Q4 e3 | 85/150 | 149/150 | 30/50 | 32/60 | 23/40 | 4056 | 75.3 | 0.38 |
| **FT SmolLM2-360M Q4 e10** | **105/150** | 148/150 | 39/50 | 33/60 | 33/40 | 4073 | 81.6 | 0.37 |
| orig Falcon-H1-Tiny-90M Q4 | 2/150 | 4/150 | 0/50 | 0/60 | 2/40 | 1587 | — | 2.47 |
| **FT Falcon-H1-Tiny-90M Q4 e3** | **112/150** | 150/150 | 48/50 | 39/60 | 25/40 | 1582 | 24.5 | 1.32 |
| FT Qwen3.5-0.8B Q4 e10 | 126/150 | 150/150 | 47/50 | 42/60 | 37/40 | 1273 | 40.2 | 1.00 |

FT 0.8B e10 vs e3 is **−9 gold** (multi-step 50 → 42). Train loss 0.0077 on 750 rows is below e3's 0.0204; that extra fit did not transfer. Orig Falcon decode tok/s is omitted: most generations were 1-token dumps, so llama.cpp's tok/s is not a speed.

Qwen orig 0.8B here is **79**, not the sweep-3 **78** — different harness (llama-server vs `/infer`, thinking kwargs, no FC). Treat them as separate tables.

### Think-on (Qwen only)

| model | gold | json | simple | multi | reject | s/case | mean pred. tokens |
|---|---:|---:|---:|---:|---:|---:|---:|
| orig 0.8B Q5 | 56/150 | 79/150 | 29/50 | 10/60 | 17/40 | 27.95 | 1063 |
| FT 0.8B Q4 e3 | 119/150 | 142/150 | 50/50 | 44/60 | 25/40 | 5.15 | 196 |
| FT 0.8B Q4 e10 | 120/150 | 146/150 | 49/50 | 39/60 | 32/40 | 3.38 | 123 |
| orig 2B Q5 | 93/150 | 105/150 | 34/50 | 36/60 | 23/40 | 38.75 | 1031 |

Thinking-on hurts the base Qwen checkpoints (empty/truncated JSON). It is not the serving mode. FT e10 think-on is **120**, vs think-off **126** and e3 think-on **119** — extra epochs did not recover the think-off drop.

### Train losses (750-row set)

| run | method | epochs | train loss | wall |
|---|---|---:|---:|---|
| Qwen3.5-0.8B | LoRA | 3 | 0.0204 | ~22 min |
| Qwen3.5-0.8B | LoRA | 10 | 0.0077 | ~72 min |
| SmolLM2-360M | LoRA | 3 | 0.0729 | ~5 min |
| SmolLM2-360M | LoRA | 10 | 0.0235 | ~17 min |
| Falcon-H1-Tiny-90M | full FT | 3 | 0.1355 | ~30 min |

Smol e10 vs e3 is **+20 gold** (simple and reject; multi-step 32 → 33). Qwen 0.8B e10 vs e3 is **−9 gold** think-off and **+1** think-on. Falcon 90M is slower than Smol 360M on this llama.cpp because each of 24 layers runs attention **and** Mamba2 **and** FFN; Q4 still leaves 169 F32 SSM/conv tensors.

This sweep does **not** change the production GGUF. FT 0.8B e3 is 135/150 on llama-server think-off vs orig 2B 120/150 on the same harness; that is not an `/infer`+FC re-bench.

## Sweep 6 — FT 0.8B e3 split-apply (llama-server + ApplyTool + FC)

Same 150 gold, **Qwen3.5-0.8B-sar-sft-Q4_K_M** think-off. Each case: drone-http reset to GUIDED / disarmed / landed, llama-server `/v1/chat/completions`, then **one** `POST /infer` `ApplyTool` per tool with `x-wait-for-ack: 1`, then telemetry settle. Never `{"Infer":...}`. SITL on `labpc` via MAVProxy.

`drone_ack_wait_ms` is a **subset** of `drone_server_ms` (the HTTP call waits for ACK). Query RTT = LLM wall + ApplyTool client walls; **reset and settle are excluded**.

Gold **134/150** (json 150/150; simple 49/50, multi 49/60, reject 36/40). Unchanged after the camera overlay. Sweep 5 think-off on the same GGUF was 135/150 with no apply.

First pass, GoPro USB preview was not streaming: exec **93/150** (40 model/camera, 9 drone apply, 8 LLM `invalid_request` so no apply). After starting GoPro stream, those **40** were retried on the same harness: **39/40** exec. Overlay exec **132/150**. Not a second full 150.

### Layer times (ms, n=150)

| clock | mean | p50 | p95 | notes |
|---|---:|---:|---:|---|
| query RTT | 1179 | 811 | 2646 | LLM + ApplyTool; no reset |
| LLM wall | 1061 | 770 | 2289 | llama-server HTTP |
| LLM prompt+decode | 1034 | 744 | 2260 | `prompt_ms` + `predicted_ms` |
| ApplyTool client | 118 | 28 | 417 | 0 on reject no-ops |
| gateway residual | 0.2 | 0 | 1 | handler − drone − model |
| drone HTTP (incl ACK) | 31 | 10 | 103 | 0 on reject |
| ACK wait (subset) | 29 | 8 | 98 | inside drone HTTP |
| model server | 83 | 0 | 329 | camera mostly failing fast |
| FC reset | 2707 | 40 | 12003 | land after airborne cases |
| inter-step settle | 1836 | 2 | 5009 | wait alt/armed/mode |
| case wall | 5721 | 1134 | 18184 | reset + RTT + settle |

On the **85** cases that applied a drone tool, ACK wait mean **51** ms (p50 45, p95 116); drone HTTP mean **54** ms. Multi-step query RTT mean **1679** ms (LLM 1539). Reject query RTT mean **712** ms (LLM only).

The layer table above is the camera-down 150. On the 40 camera retries, model-server mean **292** ms (p50 288, p95 724). Do not read the 83 ms overall model mean as live inference.

### Camera overlay (40 cases only)

USB GoPro was enumerated; UDP `:8554` had no frames until `stream/start`. Then `human_detect` / `flood_seg` / `flood_class` all returned `ok`. Overlay: gold still 37/40 on that subset; exec 39/40.

**r040** (`detect people and retry camera streams`): gold is `invalid_request` because the prompt says if *any* asked action has no tool, refuse the whole request. `camera_stream` is not a model tool (allowlist is `human_detect`, `flood_seg`, `flood_class`). `/camera/stream` is a gateway MJPEG proxy, not an LLM tool. The model still emitted `human_detect` then invented `camera_stream`. Split-apply scores that raw JSON and `ApplyTool`s it, so `human_detect` ran and `camera_stream` returned `tool_not_allowed`. Gateway `validate_llm_tasks` would have turned the whole list into `invalid_request` and would not have applied `human_detect`; that path was not re-measured here.

### Gold misses (16)

| id | cat | gold | got |
|---|---|---|---|
| s033 | simple | `return_to_home` | `invalid_request` (`RTL`) |
| m002 | multi | arm → takeoff → goto 23.56,120.47 | `invalid_request` |
| m003 | multi | takeoff `{altitude_m:30}` then goto alt 30 | takeoff with no height param |
| m007 | multi | pause → human_detect | `invalid_request` |
| m011 | multi | takeoff `{altitude_m:20}` then goto alt 20 | takeoff with no height param |
| m013 | multi | arm → takeoff 20 m → land | `invalid_request` |
| m031 | multi | takeoff 12 m, goto alt 15 | goto used alt 12 |
| m034 | multi | start_mission → pause | `invalid_request` |
| m041 | multi | takeoff `{altitude_m:15}` then goto | takeoff with no height param |
| m042 | multi | arm → takeoff → loiter | `invalid_request` |
| m044 | multi | pause → resume | `invalid_request` |
| m058 | multi | RTL → land | `invalid_request` |
| r020 | reject | `invalid_request` | `set_current_waypoint` with `seq=23.56` |
| r025 | reject | `invalid_request` | `human_detect` (`search for people and orbit the field`) |
| r030 | reject | `invalid_request` | `flood_class` (`what class is the flood`) |
| r040 | reject | `invalid_request` | `human_detect` → `camera_stream` |

Eight of those are false `invalid_request` (gold wanted tools). Four are reject cases that still emitted tools. Four are fly-to height/param mismatches.

### Exec misses after overlay (18)

Gold-true, FC/model refused (9):

- pause needs AUTO + a running mission (reset leaves GUIDED/disarmed): s016, s041, m015, m033, m051
- resume with no pause snapshot: s017, s042, m026
- r040: `camera_stream` not allowed (after `human_detect` succeeded)

Gold-false, nothing useful applied (9): s033, m002, m007, m013, m034, m042, m044, m058 (`invalid_request` / no apply); r020 (`set_current_waypoint` 502, `seq` must be u64).

Pause/resume 502s were not retried. They match the gold tool list; the vehicle is just not in AUTO with a mission after reset.

## Serving notes

- Binary: local llama.cpp b8185 (`2afcdb9`, 2026-03-02), `-ngl 99 -c 16384`.
- File: `Qwen3.5-0.8B-sar-sft-Q4_K_M.gguf` (FT e3, `--chat-template-kwargs '{"enable_thinking":false}'`). Orig 2B Q5 and orig 0.8B Q5 remain on disk.
- Gateway `max_tokens`: 2048 (this repo). Instruct models stop well under that; 256 used to zero out reasoning models.
- MiniCPM5-2B needed upstream llama.cpp PR [#23384](https://github.com/ggml-org/llama.cpp/pull/23384) (`minicpm5` pre-tokenizer). That patch is **not** in this repository; it was a local llama.cpp edit, then reverted after the GGUF was deleted.

## Out of scope

- Repairing `}}}` → `]}` in the gateway parser.
- Prompt work to make MiniCPM emit explicit `invalid_request`.
- Rebuilding llama.cpp past b8185.
- The 8B MoE (does not fit this 16 GB unified-memory box next to TensorRT + the desktop).
- Re-running the deleted GGUFs.
- Switching production from FT 0.8B e3 to Smol / Falcon.
- Re-running Sweep 5 through production `/infer` with FC ACKs.
- EXL3 / EXL2 quants.
