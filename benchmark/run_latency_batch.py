#!/usr/bin/env python3
"""
Measure LLM + tool latency through production POST /infer.

Per query records (from gateway `pipeline`):
  - queue_ms, llm_ms, gateway_ms, drone_server_ms, drone_ack_wait_ms, model_server_ms
  - tool_ms (apply_total), total_ms (handler_total), client_ms

Usage:
  python3 run_latency_batch.py --gateway http://127.0.0.1:3000
  python3 run_latency_batch.py --file latency_queries.txt --repeats 2 --wait-ack
  python3 run_latency_batch.py --release bench_v1 --limit 20 --out latency_runs/tonight

Scale to the full 150-case set: point --file at llm_edge_test_cases_100.txt (Input: blocks)
or --release bench_v1 without --limit.
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
import uuid
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from parse_cases import load_cases


@dataclass
class LatencyRow:
    query_id: str
    category: str
    prompt: str
    run_index: int
    queue_ms: float
    llm_ms: float
    gateway_ms: float
    drone_server_ms: float
    drone_ack_wait_ms: float
    model_server_ms: float
    tool_ms: float
    total_ms: float
    client_ms: float
    success: bool
    action_taken: str
    request_id: str
    tool_count: int
    drone_error: str = ""


LAYER_COLUMNS = (
    "queue_ms",
    "llm_ms",
    "gateway_ms",
    "drone_server_ms",
    "drone_ack_wait_ms",
    "model_server_ms",
    "tool_ms",
    "total_ms",
    "client_ms",
)


def _post_infer(
    gateway: str,
    prompt: str,
    *,
    wait_ack: bool,
    ack_timeout_ms: int,
    timeout_sec: float,
) -> tuple[dict[str, Any], float]:
    """POST /infer; return (JSON body, client wall ms)."""
    url = gateway.rstrip("/") + "/infer"
    payload = {"Infer": {"prompt": prompt}}
    data = json.dumps(payload).encode("utf-8")
    headers = {
        "Content-Type": "application/json",
        "x-request-id": str(uuid.uuid4()),
    }
    if wait_ack:
        headers["x-wait-for-ack"] = "1"
        headers["x-ack-timeout-ms"] = str(ack_timeout_ms)

    req = urllib.request.Request(url, data=data, headers=headers, method="POST")
    t0 = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=timeout_sec) as resp:
            body = resp.read().decode("utf-8", errors="replace")
            client_ms = (time.perf_counter() - t0) * 1000.0
            return json.loads(body), client_ms
    except urllib.error.HTTPError as e:
        client_ms = (time.perf_counter() - t0) * 1000.0
        err_body = e.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"HTTP {e.code}: {err_body[:500]}") from e


def _extract_timing(resp: dict[str, Any]) -> dict[str, float]:
    """Map gateway ApiResponse.pipeline → per-layer millisecond fields."""
    pipe = resp.get("pipeline") or {}
    llm_ms = float(pipe.get("llm_ms") or 0)
    if llm_ms <= 0:
        llm_ms = float(pipe.get("llm_http_ms", 0) or 0) + float(pipe.get("llm_parse_ms", 0) or 0)
    if llm_ms <= 0 and resp.get("llm_latency_ms"):
        llm_ms = float(resp["llm_latency_ms"])

    drone_server_ms = float(pipe.get("drone_server_ms") or 0)
    if drone_server_ms <= 0:
        drone_server_ms = sum(
            float(s.get("drone_http_ms") or 0) for s in (resp.get("drone_steps") or [])
        )

    model_server_ms = float(pipe.get("model_server_ms") or 0)
    if model_server_ms <= 0:
        model_server_ms = sum(
            float(s.get("elapsed_ms") or 0) for s in (resp.get("model_steps") or [])
        )

    tool_ms = float(pipe.get("apply_total_ms") or 0)
    if tool_ms <= 0:
        tool_ms = drone_server_ms + model_server_ms

    total_ms = float(pipe.get("handler_total_ms") or resp.get("latency_ms") or 0)
    tools = resp.get("tools") or []
    tool_count = len(tools) if isinstance(tools, list) else 0

    return {
        "queue_ms": float(pipe.get("queue_wait_ms") or 0),
        "llm_ms": llm_ms,
        "gateway_ms": float(pipe.get("gateway_ms") or 0),
        "drone_server_ms": drone_server_ms,
        "drone_ack_wait_ms": float(pipe.get("drone_ack_wait_ms") or 0),
        "model_server_ms": model_server_ms,
        "tool_ms": tool_ms,
        "total_ms": total_ms,
        "tool_count": float(tool_count),
    }


def _is_success(resp: dict[str, Any]) -> bool:
    action = str(resp.get("action_taken") or "")
    if action in ("parse_failed",) or action.startswith("llm_http_failed") or action.startswith("llm_parse_failed"):
        return False
    if resp.get("drone_error"):
        return False
    return True


def _load_query_file(path: Path) -> list[tuple[str, str]]:
    """Parse category|prompt lines or benchmark Input: blocks."""
    text = path.read_text(encoding="utf-8")
    if "Input:" in text and "Expected output:" in text:
        return [( "bench", c.input_text) for c in load_cases(path)]

    out: list[tuple[str, str]] = []
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if "|" in line:
            cat, prompt = line.split("|", 1)
            out.append((cat.strip(), prompt.strip()))
        else:
            out.append(("simple", line))
    return out


def _percentile(sorted_vals: list[float], p: float) -> float:
    if not sorted_vals:
        return 0.0
    if len(sorted_vals) == 1:
        return sorted_vals[0]
    k = (len(sorted_vals) - 1) * (p / 100.0)
    lo = int(k)
    hi = min(lo + 1, len(sorted_vals) - 1)
    w = k - lo
    return sorted_vals[lo] * (1 - w) + sorted_vals[hi] * w


def _stats(xs: list[float]) -> dict[str, float]:
    if not xs:
        return {"n": 0, "mean": 0.0, "median": 0.0, "p95": 0.0}
    s = sorted(xs)
    return {
        "n": len(s),
        "mean": statistics.mean(s),
        "median": statistics.median(s),
        "p95": _percentile(s, 95),
    }


def _print_stats_table(title: str, rows: list[LatencyRow]) -> None:
    print(f"\n{title}")
    print(f"{'layer':<18} {'mean':>10} {'median':>10} {'p95':>10}")
    print("-" * 50)
    for col in LAYER_COLUMNS:
        st = _stats([getattr(r, col) for r in rows])
        print(f"{col:<18} {st['mean']:10.1f} {st['median']:10.1f} {st['p95']:10.1f}")
    ok = sum(1 for r in rows if r.success)
    print(f"{'success':<18} {ok}/{len(rows)} ({100.0 * ok / len(rows) if rows else 0:.0f}%)")


def _print_by_category(rows: list[LatencyRow]) -> None:
    buckets: dict[str, list[LatencyRow]] = {}
    for r in rows:
        buckets.setdefault(r.category, []).append(r)
    for cat in sorted(buckets):
        _print_stats_table(f"Category: {cat} ({len(buckets[cat])} runs)", buckets[cat])


def _write_csv(path: Path, rows: list[LatencyRow]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(
            f,
            fieldnames=[
                "query_id",
                "category",
                "prompt",
                "run_index",
                "queue_ms",
                "llm_ms",
                "gateway_ms",
                "drone_server_ms",
                "drone_ack_wait_ms",
                "model_server_ms",
                "tool_ms",
                "total_ms",
                "client_ms",
                "success",
                "action_taken",
                "request_id",
                "tool_count",
                "drone_error",
            ],
        )
        w.writeheader()
        for r in rows:
            w.writerow(
                {
                    "query_id": r.query_id,
                    "category": r.category,
                    "prompt": r.prompt,
                    "run_index": r.run_index,
                    "queue_ms": f"{r.queue_ms:.2f}",
                    "llm_ms": f"{r.llm_ms:.2f}",
                    "gateway_ms": f"{r.gateway_ms:.2f}",
                    "drone_server_ms": f"{r.drone_server_ms:.2f}",
                    "drone_ack_wait_ms": f"{r.drone_ack_wait_ms:.2f}",
                    "model_server_ms": f"{r.model_server_ms:.2f}",
                    "tool_ms": f"{r.tool_ms:.2f}",
                    "total_ms": f"{r.total_ms:.2f}",
                    "client_ms": f"{r.client_ms:.2f}",
                    "success": r.success,
                    "action_taken": r.action_taken,
                    "request_id": r.request_id,
                    "tool_count": r.tool_count,
                    "drone_error": r.drone_error,
                }
            )


def main() -> int:
    ap = argparse.ArgumentParser(description="Batch LLM+tool latency via POST /infer")
    ap.add_argument(
        "--file",
        type=Path,
        default=Path(__file__).resolve().parent / "latency_queries.txt",
        help="Query file (category|prompt lines) or benchmark case file",
    )
    ap.add_argument(
        "--release",
        default="",
        help="Use releases/<id>/cases.txt instead of --file",
    )
    ap.add_argument("--gateway", default="http://127.0.0.1:3000")
    ap.add_argument("--out", default=".", help="Directory for latency_results.csv")
    ap.add_argument("--csv", default="latency_results.csv", help="CSV filename under --out")
    ap.add_argument("--limit", type=int, default=0, help="Max queries (0 = all)")
    ap.add_argument("--repeats", type=int, default=1, help="Runs per query")
    ap.add_argument("--wait-ack", action="store_true", help="Set x-wait-for-ack: 1 on /infer")
    ap.add_argument("--ack-timeout-ms", type=int, default=3000)
    ap.add_argument("--timeout-sec", type=float, default=180.0)
    ap.add_argument("--model-tag", default="", help="Label for this run (e.g. qwen-0.8b)")
    ap.add_argument("--sleep-ms", type=int, default=0, help="Pause between runs")
    args = ap.parse_args()

    bench_root = Path(__file__).resolve().parent
    if args.release:
        case_file = bench_root / "releases" / args.release / "cases.txt"
    else:
        case_file = args.file

    if not case_file.is_file():
        print(f"error: query file not found: {case_file}", file=sys.stderr)
        return 2

    queries = _load_query_file(case_file)
    if args.limit and args.limit > 0:
        queries = queries[: args.limit]

    if not queries:
        print("error: no queries loaded", file=sys.stderr)
        return 2

    out_dir = Path(args.out)
    rows: list[LatencyRow] = []

    print(f"Gateway: {args.gateway}")
    if args.model_tag:
        print(f"Model: {args.model_tag}")
    print(f"Queries: {len(queries)} x {args.repeats} repeat(s) = {len(queries) * args.repeats} runs")
    print(f"wait_for_ack: {args.wait_ack}")
    print()

    for qi, (category, prompt) in enumerate(queries, start=1):
        query_id = f"q{qi:03d}"
        prompt_short = prompt if len(prompt) <= 60 else prompt[:57] + "..."
        for run_index in range(args.repeats):
            try:
                resp, client_ms = _post_infer(
                    args.gateway,
                    prompt,
                    wait_ack=args.wait_ack,
                    ack_timeout_ms=args.ack_timeout_ms,
                    timeout_sec=args.timeout_sec,
                )
                t = _extract_timing(resp)
                row = LatencyRow(
                    query_id=query_id,
                    category=category,
                    prompt=prompt,
                    run_index=run_index,
                    queue_ms=t["queue_ms"],
                    llm_ms=t["llm_ms"],
                    gateway_ms=t["gateway_ms"],
                    drone_server_ms=t["drone_server_ms"],
                    drone_ack_wait_ms=t["drone_ack_wait_ms"],
                    model_server_ms=t["model_server_ms"],
                    tool_ms=t["tool_ms"],
                    total_ms=t["total_ms"],
                    client_ms=client_ms,
                    success=_is_success(resp),
                    action_taken=str(resp.get("action_taken") or ""),
                    request_id=str(resp.get("request_id") or ""),
                    tool_count=int(t["tool_count"]),
                    drone_error=str(resp.get("drone_error") or ""),
                )
            except Exception as e:
                row = LatencyRow(
                    query_id=query_id,
                    category=category,
                    prompt=prompt,
                    run_index=run_index,
                    queue_ms=0.0,
                    llm_ms=0.0,
                    gateway_ms=0.0,
                    drone_server_ms=0.0,
                    drone_ack_wait_ms=0.0,
                    model_server_ms=0.0,
                    tool_ms=0.0,
                    total_ms=0.0,
                    client_ms=0.0,
                    success=False,
                    action_taken=f"error:{e}",
                    request_id="",
                    tool_count=0,
                    drone_error=str(e),
                )

            rows.append(row)
            status = "ok" if row.success else "FAIL"
            print(
                f"[{status}] {query_id} run={run_index} {category:<10} "
                f"llm={row.llm_ms:6.0f} gw={row.gateway_ms:4.0f} "
                f"drone={row.drone_server_ms:4.0f} model={row.model_server_ms:3.0f} "
                f"total={row.total_ms:6.0f}ms  {prompt_short}"
            )

            if args.sleep_ms > 0:
                time.sleep(args.sleep_ms / 1000.0)

    csv_path = out_dir / args.csv
    _write_csv(csv_path, rows)

    _print_stats_table(f"Overall ({len(rows)} runs)", rows)
    _print_by_category(rows)

    print(f"\nWrote {csv_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
