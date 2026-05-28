# LLM edge benchmark (`/eval`)

Benchmarks the gateway LLM tool path **without** applying drone commands (safe for 100+ prompts).

## Prerequisites

1. **llama-server** running (e.g. `http://127.0.0.1:8080`).
2. **Gateway** built with the `eval` feature and restarted:

   ```bash
   cd /path/to/gateway
   cargo build --release --features eval
   ```

   Default production builds omit `/eval`:

   ```bash
   cargo build --release
   ```

3. Copy your case file onto the machine (e.g. `llm_edge_test_cases_100.txt`). Format per block:

   ```text
   Input: your prompt here
   Expected output: {"tasks":[...]}

   Input: next prompt
   Expected output: {"tasks":[...]}
   ```

   Separate blocks with a **blank line** (or a line of `---`).

## Run

From this directory:

```bash
python3 run_eval.py \
  --file llm_edge_test_cases_100.txt \
  --gateway http://127.0.0.1:3000 \
  --out results/run_001
```

Options:

| Flag | Meaning |
|------|---------|
| `--limit N` | Only first N cases (smoke test) |
| `--float-tol` | Numeric tolerance for param comparison (default `1e-5`) |
| `--tegra` | Jetson only: background `tegrastats` log + per-case snapshots |

Smoke (2 built-in cases):

```bash
python3 run_eval.py --file sample_smoke.txt --gateway http://127.0.0.1:3000 --out results/smoke --limit 2
```

## Outputs

| File | Content |
|------|---------|
| `results.jsonl` | One JSON object per case: input, expected, full `eval_response`, scores |
| `summary.json` / `summary.csv` | Aggregate rates + latency mean / p50 / p95 |
| `failures.csv` | Rows where effective intent+params+`json_valid` did not all pass |
| `tegra_suite.log` | Raw `tegrastats` samples (with `--tegra`) |
| `tegra_summary.json` | Parsed aggregates from the suite log |

## Metrics

- **LLM latency** — `eval_response.llm_latency_ms` (HTTP time to chat completions only).
- **E2E parse** — `eval_response.e2e_parse_ms` (gateway: LLM + parse + normalize for response).
- **Client E2E** — `e2e_client_ms` in JSONL (includes network; optional extra signal).
- **JSON valid rate** — fraction with `json_valid == true`.
- **Intent / params** — strict uses `llm_tool_json_raw`; effective uses `llm_tool_json` (after altitude normalization).

## Removing eval from the codebase

1. Delete `src/eval.rs` and remove `#[cfg(feature = "eval")] mod eval` from `src/main.rs`.
2. Remove the `eval` feature and `merge(eval_router())` from `Cargo.toml` / `src/server.rs`.
3. Optionally delete this `benchmark/` directory.

The shared LLM helper remains in `src/llm_decision.rs` (used by `/infer`).

## Manual `curl`

```bash
curl -sS http://127.0.0.1:3000/eval \
  -H 'Content-Type: application/json' \
  -d '{"prompt":"hi"}' | jq .
```

(Only works when the gateway binary was built with `--features eval`.)
