#!/usr/bin/env python3
"""Score bench_v1 cases against production POST /infer (LLM routing + apply outcome)."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
import uuid
from pathlib import Path
from typing import Any

from parse_cases import load_cases
from run_eval import (
    _category_for_case,
    _intent_match,
    _load_manifest,
    _params_match,
    _tasks_from_tool_json,
)


def _curl_infer(
    gateway: str,
    key: str,
    prompt: str,
    *,
    wait_ack: bool,
    timeout_sec: float,
) -> tuple[dict[str, Any] | None, str | None, int]:
    url = gateway.rstrip("/") + "/infer"
    payload = json.dumps({"Infer": {"prompt": prompt}})
    cmd = [
        "curl",
        "-sS",
        "-m",
        str(int(timeout_sec)),
        "-w",
        "\n__HTTP__%{http_code}",
        "-X",
        "POST",
        url,
        "-H",
        "Content-Type: application/json",
        "-H",
        f"Authorization: Bearer {key}",
        "-H",
        f"X-API-Key: {key}",
        "-H",
        f"x-request-id: bench-{uuid.uuid4()}",
        "-H",
        f"x-wait-for-ack: {'true' if wait_ack else 'false'}",
        "-d",
        payload,
    ]
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
    except Exception as e:
        return None, str(e), 0
    out = proc.stdout or proc.stderr or ""
    if "__HTTP__" not in out:
        return None, out.strip() or f"curl exit {proc.returncode}", 0
    body, _, code_str = out.rpartition("__HTTP__")
    try:
        code = int(code_str.strip())
    except ValueError:
        code = 0
    if code != 200:
        return None, f"HTTP {code}: {body.strip()[:300]}", code
    try:
        return json.loads(body), None, code
    except json.JSONDecodeError as e:
        return None, f"invalid JSON: {e}; body={body[:200]!r}", code


def _invalid_pass(expected: list[dict[str, Any]], resp: dict[str, Any], actual: list | None) -> bool:
    if _intent_match(expected, actual):
        return True
    action = resp.get("action_taken") or ""
    if action == "invalid_request" or action.startswith("none:"):
        drone_ok = [s for s in resp.get("drone_steps") or [] if s.get("ok")]
        return len(drone_ok) == 0
    if resp.get("category") == "none" and not resp.get("drone_steps"):
        return True
    return False


def _execution_ok(resp: dict[str, Any], category: str) -> bool:
    if category == "invalid":
        return _invalid_pass([], resp, _tasks_from_tool_json(resp.get("llm_tool_json")))
    if resp.get("state") == "ERROR":
        return False
    drone_steps = resp.get("drone_steps") or []
    model_steps = resp.get("model_steps") or []
    if drone_steps and any(not s.get("ok") for s in drone_steps):
        return False
    if model_steps and any(not s.get("ok") for s in model_steps):
        return False
    if category.startswith("multi") or category == "single_drone":
        if drone_steps:
            return all(s.get("ok") for s in drone_steps)
    return True


def main() -> int:
    ap = argparse.ArgumentParser(description="bench_v1 via POST /infer")
    ap.add_argument("--gateway", default="https://edge-ai.basilrari.com")
    ap.add_argument("--key", required=True)
    ap.add_argument("--release", default="bench_v1")
    ap.add_argument(
        "--categories",
        default="multi_short,multi_long,invalid",
        help="Comma-separated manifest categories",
    )
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--delay-sec", type=float, default=0.8)
    ap.add_argument("--wait-ack", action="store_true")
    ap.add_argument("--out-jsonl", required=True)
    ap.add_argument("--float-tol", type=float, default=1e-5)
    args = ap.parse_args()

    bench_root = Path(__file__).resolve().parent
    case_file = bench_root / "releases" / args.release / "cases.txt"
    manifest = _load_manifest(case_file)
    want = {c.strip() for c in args.categories.split(",") if c.strip()}
    cases = load_cases(case_file)
    filtered = [
        c
        for c in cases
        if _category_for_case(c.index, c.expected, manifest) in want
    ]
    if args.limit:
        filtered = filtered[: args.limit]

    out_path = Path(args.out_jsonl)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    rows: list[dict[str, Any]] = []
    with out_path.open("w", encoding="utf-8") as jf:
        for i, c in enumerate(filtered):
            cat = _category_for_case(c.index, c.expected, manifest)
            if i and args.delay_sec > 0:
                time.sleep(args.delay_sec)
            resp, err, http = _curl_infer(
                args.gateway,
                args.key,
                c.input_text,
                wait_ack=args.wait_ack,
                timeout_sec=120.0,
            )
            exp_tasks = c.expected["tasks"]
            actual = _tasks_from_tool_json(resp.get("llm_tool_json")) if resp else None
            json_valid = actual is not None
            intent = _intent_match(exp_tasks, actual) if resp else False
            params = _params_match(exp_tasks, actual, args.float_tol) if resp else False
            full = intent and params
            if cat == "invalid" and resp:
                full = _invalid_pass(exp_tasks, resp, actual)
            exec_ok = _execution_ok(resp, cat) if resp else False
            row = {
                "case_index": c.index,
                "category": cat,
                "input": c.input_text,
                "http": http,
                "error": err,
                "json_valid": json_valid,
                "intent_match": intent,
                "params_match": params,
                "full_match": full,
                "execution_ok": exec_ok,
                "state": resp.get("state") if resp else None,
                "action_taken": resp.get("action_taken") if resp else None,
                "drone_error": resp.get("drone_error") if resp else None,
                "expected_tasks": exp_tasks,
                "actual_tasks": actual,
                "drone_steps": resp.get("drone_steps") if resp else None,
                "model_steps": resp.get("model_steps") if resp else None,
            }
            rows.append(row)
            jf.write(json.dumps(row) + "\n")
            jf.flush()
            status = "PASS" if full and exec_ok and not err else "FAIL"
            print(
                f"CASE {c.index:3d} [{cat:11s}] {status} intent={intent} exec={exec_ok} action={row.get('action_taken')}",
                flush=True,
            )

    # summary
    by_cat: dict[str, list[dict[str, Any]]] = {}
    for r in rows:
        by_cat.setdefault(r["category"], []).append(r)
    print("\n=== Summary ===")
    for cat, rs in sorted(by_cat.items()):
        n = len(rs)
        print(
            cat,
            f"n={n}",
            f"intent={sum(1 for r in rs if r['intent_match'])}/{n}",
            f"full={sum(1 for r in rs if r['full_match'])}/{n}",
            f"exec={sum(1 for r in rs if r['execution_ok'])}/{n}",
            f"errors={sum(1 for r in rs if r['error'])}/{n}",
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
