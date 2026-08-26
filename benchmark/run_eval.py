#!/usr/bin/env python3
"""
Run LLM benchmark suite against gateway `POST /eval` (requires `cargo build --features eval`).

Scores strict (llm_tool_json_raw) vs effective (llm_tool_json) accuracy against Expected output.
"""

from __future__ import annotations

import argparse
import csv
import json
import statistics
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

from parse_cases import load_cases
from classify_case import classify_case
from tegra_sampler import TegraSuiteLog, summarize_log_file, tegra_snapshot, tegrastats_available


def _post_json(url: str, payload: dict[str, Any], timeout_sec: float = 130.0) -> dict[str, Any]:
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        url,
        data=data,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout_sec) as resp:
            body = resp.read().decode("utf-8", errors="replace")
            return json.loads(body)
    except urllib.error.HTTPError as e:
        err_body = e.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"HTTP {e.code}: {err_body[:500]}") from e


def _tasks_from_tool_json(s: str | None) -> list[dict[str, Any]] | None:
    if not s:
        return None
    try:
        obj = json.loads(s)
        t = obj.get("tasks")
        if isinstance(t, list):
            return t
    except json.JSONDecodeError:
        return None
    return None


def _intent_match(expected: list[dict[str, Any]], actual: list[dict[str, Any]] | None) -> bool:
    if actual is None:
        return False
    if len(expected) != len(actual):
        return False
    for e, a in zip(expected, actual):
        if e.get("category") != a.get("category") or e.get("name") != a.get("name"):
            return False
    return True


def _json_close(a: Any, b: Any, tol: float) -> bool:
    if a is None and b is None:
        return True
    if a is None or b is None:
        if a in (None, {}, []) and b in (None, {}, []):
            return True
        return False
    if type(a) is not type(b):
        if isinstance(a, (int, float)) and isinstance(b, (int, float)):
            return abs(float(a) - float(b)) <= tol
        return False
    if isinstance(a, dict):
        if set(a.keys()) != set(b.keys()):
            return False
        return all(_json_close(a[k], b[k], tol) for k in a)
    if isinstance(a, list):
        if len(a) != len(b):
            return False
        return all(_json_close(x, y, tol) for x, y in zip(a, b))
    if isinstance(a, float) or isinstance(b, float):
        return abs(float(a) - float(b)) <= tol
    if isinstance(a, (int, str, bool)):
        return a == b
    return a == b


def _params_match(
    expected: list[dict[str, Any]], actual: list[dict[str, Any]] | None, tol: float
) -> bool:
    if actual is None or len(expected) != len(actual):
        return False
    for e, a in zip(expected, actual):
        ep = e.get("params")
        ap = a.get("params")
        if ep is None:
            ep = {}
        if ap is None:
            ap = {}
        if not _json_close(ep, ap, tol):
            return False
    return True


def _load_manifest(case_file: Path) -> dict[str, Any] | None:
    """If cases live under releases/<id>/, load manifest.json for category labels."""
    parent = case_file.parent
    manifest_path = parent / "manifest.json"
    if manifest_path.is_file():
        return json.loads(manifest_path.read_text(encoding="utf-8"))
    return None


def _category_for_case(
    case_index: int, expected: dict[str, Any], manifest: dict[str, Any] | None
) -> str:
    if manifest and "cases" in manifest:
        for entry in manifest["cases"]:
            if entry.get("index") == case_index:
                return str(entry.get("category", "unknown"))
    return classify_case(expected)


def _by_category_summary(rows: list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    buckets: dict[str, list[dict[str, Any]]] = {}
    for r in rows:
        cat = r.get("category") or "unknown"
        buckets.setdefault(cat, []).append(r)

    out: dict[str, dict[str, Any]] = {}
    for cat, rs in sorted(buckets.items()):
        n = len(rs)
        full = sum(
            1
            for r in rs
            if r.get("json_valid")
            and r.get("intent_match_effective")
            and r.get("params_match_effective")
        )
        out[cat] = {
            "cases": n,
            "full_match_rate": full / n if n else 0.0,
            "json_valid_rate": sum(1 for r in rs if r.get("json_valid")) / n if n else 0.0,
            "intent_effective_rate": sum(1 for r in rs if r.get("intent_match_effective"))
            / n
            if n
            else 0.0,
        }
    return out


def _percentile(sorted_vals: list[float], p: float) -> float:
    if not sorted_vals:
        return 0.0
    if len(sorted_vals) == 1:
        return float(sorted_vals[0])
    k = (len(sorted_vals) - 1) * (p / 100.0)
    lo = int(k)
    hi = min(lo + 1, len(sorted_vals) - 1)
    w = k - lo
    return sorted_vals[lo] * (1 - w) + sorted_vals[hi] * w


def main() -> int:
    ap = argparse.ArgumentParser(description="Benchmark gateway /eval against a case file")
    ap.add_argument("--file", help="Path to case file (Input:/Expected output: blocks)")
    ap.add_argument(
        "--release",
        default="",
        help="Shortcut: releases/<id>/cases.txt (e.g. bench_v1)",
    )
    ap.add_argument("--gateway", default="http://127.0.0.1:3000", help="Gateway base URL")
    ap.add_argument("--out", required=True, help="Output directory for results")
    ap.add_argument("--limit", type=int, default=0, help="Max cases (0 = all)")
    ap.add_argument("--float-tol", type=float, default=1e-5, help="Tolerance for numeric params")
    ap.add_argument("--tegra", action="store_true", help="Sample tegrastats (Jetson)")
    ap.add_argument(
        "--mode",
        choices=("decision", "e2e"),
        default="decision",
        help="decision=POST /eval (no drone apply); e2e=POST /eval/e2e (SITL ACK)",
    )
    ap.add_argument(
        "--sitl-token",
        default="",
        help="EVAL_SITL_TOKEN value (required for --mode e2e)",
    )
    ap.add_argument("--ack-timeout-ms", type=int, default=3000, help="E2E ACK timeout")
    args = ap.parse_args()

    bench_root = Path(__file__).resolve().parent
    if args.release:
        case_file = bench_root / "releases" / args.release / "cases.txt"
    elif args.file:
        case_file = Path(args.file)
    else:
        print("error: provide --file or --release", file=sys.stderr)
        return 2

    if not case_file.is_file():
        print(f"error: case file not found: {case_file}", file=sys.stderr)
        return 2

    manifest = _load_manifest(case_file)
    bench_id = manifest.get("id") if manifest else None

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    base = args.gateway.rstrip("/")
    eval_url = base + ("/eval/e2e" if args.mode == "e2e" else "/eval")

    if args.mode == "e2e" and not args.sitl_token.strip():
        print("error: --mode e2e requires --sitl-token (gateway EVAL_SITL_TOKEN)", file=sys.stderr)
        return 2

    cases = load_cases(case_file)
    if args.limit and args.limit > 0:
        cases = cases[: args.limit]

    tegra_log = out / "tegra_suite.log"
    suite: TegraSuiteLog | None = None
    if args.tegra:
        if not tegrastats_available():
            print("warning: tegrastats not found; continuing without --tegra", file=sys.stderr)
        else:
            suite = TegraSuiteLog(tegra_log)
            suite.start()
            print(f"tegrastats logging to {tegra_log}", file=sys.stderr)

    jsonl_path = out / "results.jsonl"
    failures_path = out / "failures.csv"

    rows: list[dict[str, Any]] = []
    fail_rows: list[dict[str, Any]] = []

    with jsonl_path.open("w", encoding="utf-8") as jf:
        for c in cases:
            exp_tasks = c.expected["tasks"]
            category = _category_for_case(c.index, c.expected, manifest)
            t0 = time.perf_counter()
            tegra_before = tegra_snapshot() if args.tegra and tegrastats_available() else None
            try:
                if args.mode == "e2e":
                    resp = _post_json(
                        eval_url,
                        {
                            "prompt": c.input_text,
                            "target": "sitl",
                            "wait_for": "ack",
                            "ack_timeout_ms": args.ack_timeout_ms,
                            "safety_token": args.sitl_token.strip(),
                        },
                        timeout_sec=180.0,
                    )
                else:
                    resp = _post_json(eval_url, {"prompt": c.input_text})
            except Exception as e:
                row = {
                    "case_index": c.index,
                    "category": category,
                    "input": c.input_text,
                    "expected": c.expected,
                    "error": str(e),
                    "json_valid": False,
                    "llm_latency_ms": None,
                    "e2e_parse_ms": None,
                }
                jf.write(json.dumps(row, ensure_ascii=False) + "\n")
                rows.append(row)
                fail_rows.append({**row, "reason": "http_or_eval_error"})
                continue

            e2e_ms = int((time.perf_counter() - t0) * 1000)
            tegra_after = tegra_snapshot() if args.tegra and tegrastats_available() else None

            strict_tasks = _tasks_from_tool_json(resp.get("llm_tool_json_raw"))
            eff_tasks = _tasks_from_tool_json(resp.get("llm_tool_json"))
            if args.mode == "e2e":
                strict_tasks = _tasks_from_tool_json(resp.get("llm_tool_json"))
                eff_tasks = strict_tasks

            jv = bool(resp.get("json_valid")) if args.mode == "decision" else bool(
                resp.get("llm_tool_json") or resp.get("tools")
            )
            is_strict = _intent_match(exp_tasks, strict_tasks)
            is_eff = _intent_match(exp_tasks, eff_tasks)
            ps_strict = _params_match(exp_tasks, strict_tasks, args.float_tol) if is_strict else False
            ps_eff = _params_match(exp_tasks, eff_tasks, args.float_tol) if is_eff else False

            row = {
                "case_index": c.index,
                "category": category,
                "input": c.input_text,
                "expected": c.expected,
                "mode": args.mode,
                "eval_response": resp,
                "e2e_client_ms": e2e_ms,
                "json_valid": jv,
                "intent_match_strict": is_strict,
                "intent_match_effective": is_eff,
                "params_match_strict": ps_strict,
                "params_match_effective": ps_eff,
                "tegra_before": tegra_before,
                "tegra_after": tegra_after,
            }
            jf.write(json.dumps(row, ensure_ascii=False) + "\n")
            rows.append(row)
            if not (is_eff and ps_eff and jv):
                fail_rows.append(
                    {
                        "case_index": c.index,
                        "input": c.input_text[:200],
                        "json_valid": jv,
                        "intent_strict": is_strict,
                        "intent_effective": is_eff,
                        "params_strict": ps_strict,
                        "params_effective": ps_eff,
                        "parse_error": resp.get("parse_error"),
                        "action_taken": resp.get("action_taken"),
                    }
                )

    if suite:
        suite.stop()
        tegra_summary = summarize_log_file(tegra_log)
        (out / "tegra_summary.json").write_text(
            json.dumps(tegra_summary, indent=2), encoding="utf-8"
        )
    else:
        tegra_summary = {}

    n = len(rows)
    n_json = sum(1 for r in rows if r.get("json_valid"))
    n_is = sum(1 for r in rows if r.get("intent_match_strict"))
    n_ie = sum(1 for r in rows if r.get("intent_match_effective"))
    n_ps = sum(1 for r in rows if r.get("params_match_strict"))
    n_pe = sum(1 for r in rows if r.get("params_match_effective"))

    llm_ms = [
        float(r["eval_response"]["llm_latency_ms"])
        for r in rows
        if r.get("eval_response") and "llm_latency_ms" in r["eval_response"]
    ]
    e2e_ms = [
        float(r["eval_response"]["e2e_parse_ms"])
        for r in rows
        if r.get("eval_response")
        and "e2e_parse_ms" in r["eval_response"]
        and args.mode == "decision"
    ]
    handler_ms = [
        float(r["eval_response"].get("latency_ms", 0))
        for r in rows
        if r.get("eval_response") and args.mode == "e2e"
    ]
    pipeline_ack = [
        float(r["eval_response"].get("pipeline", {}).get("prompt_to_final_ack_ms", 0))
        for r in rows
        if r.get("eval_response")
        and r["eval_response"].get("pipeline")
        and r["eval_response"]["pipeline"].get("prompt_to_final_ack_ms") is not None
    ]

    def lat_stats(xs: list[float]) -> dict[str, float]:
        if not xs:
            return {}
        s = sorted(xs)
        return {
            "mean": statistics.mean(s),
            "p50": _percentile(s, 50),
            "p95": _percentile(s, 95),
            "max": max(s),
        }

    summary = {
        "cases": n,
        "benchmark_id": bench_id,
        "mode": args.mode,
        "eval_url": eval_url,
        "case_file": str(case_file),
        "json_valid_rate": n_json / n if n else 0.0,
        "intent_strict_rate": n_is / n if n else 0.0,
        "intent_effective_rate": n_ie / n if n else 0.0,
        "params_strict_rate": n_ps / n if n else 0.0,
        "params_effective_rate": n_pe / n if n else 0.0,
        "params_strict_of_intent_strict": n_ps / n_is if n_is else None,
        "params_effective_of_intent_effective": n_pe / n_ie if n_ie else None,
        "llm_latency_ms": lat_stats(llm_ms),
        "e2e_parse_ms_gateway": lat_stats(e2e_ms),
        "e2e_handler_ms": lat_stats(handler_ms),
        "prompt_to_final_ack_ms": lat_stats(pipeline_ack),
        "e2e_client_ms": lat_stats([float(r["e2e_client_ms"]) for r in rows if "e2e_client_ms" in r]),
        "by_category": _by_category_summary(rows),
        "tegra_suite": tegra_summary,
    }
    (out / "summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")

    with (out / "summary.csv").open("w", newline="", encoding="utf-8") as cf:
        w = csv.writer(cf)
        w.writerow(["metric", "value"])
        for k, v in summary.items():
            if isinstance(v, (dict, list)):
                v = json.dumps(v)
            w.writerow([k, v])

    if fail_rows:
        with failures_path.open("w", newline="", encoding="utf-8") as ff:
            keys = sorted({k for fr in fail_rows for k in fr})
            dw = csv.DictWriter(ff, fieldnames=keys)
            dw.writeheader()
            dw.writerows(fail_rows)

    print(json.dumps(summary, indent=2))
    print(f"\nWrote {jsonl_path} and summary files under {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
