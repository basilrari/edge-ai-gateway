#!/usr/bin/env python3
"""Rebuild failures.csv from an existing results.jsonl (adds input + expected/actual JSON)."""

from __future__ import annotations

import argparse
import csv
import json
from pathlib import Path


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("run_dir", help="Benchmark run directory containing results.jsonl")
    ap.add_argument(
        "-o",
        "--output",
        default="",
        help="Output CSV path (default: <run_dir>/failures.csv)",
    )
    args = ap.parse_args()

    run = Path(args.run_dir)
    jsonl = run / "results.jsonl"
    if not jsonl.is_file():
        raise SystemExit(f"missing {jsonl}")

    out = Path(args.output) if args.output else run / "failures.csv"

    fields = [
        "case_index",
        "input",
        "expected_json",
        "actual_json",
        "action_taken",
        "json_valid",
        "intent_effective",
        "intent_strict",
        "params_effective",
        "params_strict",
        "parse_error",
        "error",
    ]

    rows: list[dict] = []
    for line in jsonl.open(encoding="utf-8"):
        r = json.loads(line)
        if (
            r.get("intent_match_effective")
            and r.get("params_match_effective")
            and r.get("json_valid")
        ):
            continue
        resp = r.get("eval_response") or {}
        actual = resp.get("llm_tool_json") or resp.get("llm_tool_json_raw") or ""
        rows.append(
            {
                "case_index": r["case_index"],
                "input": r["input"],
                "expected_json": json.dumps(
                    r["expected"], separators=(",", ":"), ensure_ascii=False
                ),
                "actual_json": actual,
                "action_taken": resp.get("action_taken"),
                "json_valid": r.get("json_valid"),
                "intent_effective": r.get("intent_match_effective"),
                "intent_strict": r.get("intent_match_strict"),
                "params_effective": r.get("params_match_effective"),
                "params_strict": r.get("params_match_strict"),
                "parse_error": resp.get("parse_error"),
                "error": r.get("error"),
            }
        )

    with out.open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=fields, extrasaction="ignore")
        w.writeheader()
        w.writerows(rows)

    print(f"Wrote {len(rows)} rows to {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
