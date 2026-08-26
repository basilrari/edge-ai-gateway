# LLM edge benchmark (`/eval` and `/eval/e2e`)

## Latency batch (`POST /infer`)

For tonight’s 15–20 query runs (or the full case set later), use **`run_latency_batch.py`** against the **production** infer path — no `eval` feature required.

```bash
cd gateway/benchmark
python3 run_latency_batch.py --gateway http://127.0.0.1:3000
python3 run_latency_batch.py --file latency_queries.txt --repeats 2 --wait-ack --out latency_runs/tonight
# Scale up: --release bench_v1   or   --file llm_edge_test_cases_100.txt
```

Each run logs per-query `llm_ms`, `tool_ms`, `total_ms` (from gateway `pipeline`) plus client wall time. Outputs:

- **`latency_results.csv`** — raw rows (expand to 150+ cases by changing `--file` / `--release`)
- **Summary table** — mean / median / P95 overall and by category (`simple`, `multi-step`, `reject`)

Edit **`latency_queries.txt`** (`category|prompt` per line) for your test set. Gateway also emits structured `action=query_latency` log lines for aggregation.

---

LLM decisions are driven by **`SAR_SYSTEM_PROMPT`** in [`../src/llm.rs`](../src/llm.rs) (example-first `{"tasks":[...]}` router, max 5 steps). Case **Expected output** must match that contract (`invalid_request` for no-op).

Two modes:

| Mode | Endpoint | Drone apply | Use |
|------|----------|-------------|-----|
| **decision** (default) | `POST /eval` | No | LLM JSON accuracy + LLM timing |
| **e2e** | `POST /eval/e2e` | Yes (SITL, ACK wait) | Full pipeline through drone-http + FC `COMMAND_ACK` |

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

3. Copy your case file onto the machine (e.g. `llm_edge_test_cases_100.txt` or `e2e_sitl_smoke.txt`). Format per block:

   ```text
   Input: your prompt here
   Expected output: {"tasks":[...]}

   Input: next prompt
   Expected output: {"tasks":[...]}
   ```

   Separate blocks with a **blank line** (or a line of `---`).

## Standard release: `bench_v1`

Stratified **100** cases (20 × invalid, single model, single drone, multi short, multi long). See [`releases/bench_v1/README.md`](releases/bench_v1/README.md).

```bash
python3 build_bench_v1.py   # regenerate cases.txt + manifest.json from legacy pool
python3 run_eval.py \
  --release bench_v1 \
  --gateway http://127.0.0.1:3000 \
  --out results/bench_v1_run_001
```

Legacy unstratified pool: `llm_edge_test_cases_100.txt`.

## Run

From this directory:

```bash
python3 run_eval.py \
  --file llm_edge_test_cases_100.txt \
  --gateway http://127.0.0.1:3000 \
  --out results/run_001
```

SITL end-to-end (requires `EVAL_SITL_TOKEN` on gateway, loopback `DRONE_SERVER_URL`, SITL + drone-http):

```bash
export EVAL_SITL_TOKEN=your-secret
python3 run_eval.py \
  --mode e2e \
  --sitl-token your-secret \
  --file e2e_sitl_smoke.txt \
  --gateway http://127.0.0.1:3000 \
  --out results/e2e_001
```

Options:

| Flag | Meaning |
|------|---------|
| `--release` | Use `releases/<id>/cases.txt` (e.g. `bench_v1`) |
| `--file` | Explicit case file (required if `--release` omitted) |
| `--mode` | `decision` (default) or `e2e` |
| `--sitl-token` | Required for `e2e` (must match gateway `EVAL_SITL_TOKEN`) |
| `--ack-timeout-ms` | FC ACK wait per drone step in e2e mode (default 3000) |
| `--limit N` | Only first N cases (smoke test) |
| `--float-tol` | Numeric tolerance for param comparison (default `1e-5`) |

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

| `--tegra` | Jetson only: background `tegrastats` log + per-case snapshots |

## Metrics

- **LLM latency** — `llm_latency_ms` / `llm_http_ms` + `llm_parse_ms` on decision responses.
- **E2E parse** — `e2e_parse_ms` (decision mode only).
- **E2E handler** — `latency_ms` + `pipeline` on `/eval/e2e` (gateway through drone ACK).
- **Client E2E** — `e2e_client_ms` in JSONL.
- **Prompt → final ACK** — `pipeline.prompt_to_final_ack_ms` when ACK wait is enabled.

Production UI and `POST /infer` return the same `pipeline`, `drone_steps`, and client headers (`x-client-dispatch-ms`, optional `x-wait-for-ack`).

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
