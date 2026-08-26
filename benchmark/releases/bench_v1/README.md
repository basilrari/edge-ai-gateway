# bench_v1 — stratified LLM router benchmark

Fixed **100** prompts with **20 cases per category** (no hand-picking at run time).

| Category | Meaning | E2E drone apply |
|----------|---------|-----------------|
| `invalid` | Single `invalid_request` | No tools should run |
| `single_model` | One model tool | Model step only (orchestrator) |
| `single_drone` | One drone tool | May ACK on SITL e2e |
| `multi_short` | 2–3 ordered steps | May ACK per drone step |
| `multi_long` | 4–5 ordered steps | May ACK per drone step |

## Pass rule (decision / `POST /eval`)

A case **passes** when all of:

- `json_valid` — gateway parsed `{"tasks":[...]}`
- `intent_match_effective` — same categories and tool `name`s in order
- `params_match_effective` — param objects match within `--float-tol`

## Pass rule (e2e / `POST /eval/e2e`)

Same **LLM JSON** scoring as decision. Additionally:

- **`invalid`**: production path must not apply drone tools (check `action_taken` / no `drone_error` from spurious apply).
- **Drone categories**: optional FC `COMMAND_ACK` timing in `pipeline` (gateway `EVAL_SITL_TOKEN`, loopback drone URL).

Regenerate this release from the legacy pool + fill-ins:

```bash
cd benchmark
python3 build_bench_v1.py
```

## Run

```bash
python3 run_eval.py \
  --release bench_v1 \
  --gateway http://127.0.0.1:3000 \
  --out results/bench_v1_decision_001
```

SITL e2e (full suite — long; use `--limit` for smoke):

```bash
python3 run_eval.py \
  --mode e2e \
  --release bench_v1 \
  --sitl-token "$EVAL_SITL_TOKEN" \
  --gateway http://127.0.0.1:3000 \
  --out results/bench_v1_e2e_001
```

`summary.json` includes **`by_category`** rates for regression tracking.

Printable PDF (all 100 inputs + expected JSON):

```bash
python3 generate_pdf.py   # writes bench_v1_cases.pdf in this directory
```

## Versioning

- **Do not edit `cases.txt` in place** for a published score — bump to `bench_v2` and rebuild.
- `manifest.json` is the machine-readable gold list (inputs + expected JSON + category).
