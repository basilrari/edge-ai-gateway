#!/usr/bin/env python3
"""Compare latency CSVs: decision accuracy + layer means (Q4 vs Q5)."""

from __future__ import annotations

import csv
import statistics
import sys
from collections import defaultdict
from pathlib import Path

EXPECTED = {
    "q001": ("reject", None),
    "q002": ("reject", None),
    "q003": ("reject", None),
    "q004": ("reject", None),
    "q005": ("simple", "human_detect"),
    "q006": ("simple", "flood_class"),
    "q007": ("simple", "arm"),
    "q008": ("simple", "return_to_home"),
    "q009": ("simple", "land_immediately"),
    "q010": ("multi", ["arm", "takeoff"]),
    "q011": ("multi", ["takeoff", "hover"]),
    "q012": ("multi", ["start_mission", "human_detect"]),
    "q013": ("multi", ["goto_location"]),
    "q014": ("multi", ["takeoff", "land_immediately"]),
    "q015": ("multi", ["mission_interrupt", "human_detect"]),
    "q016": ("multi", ["takeoff", "move_forward"]),
}


def decision_ok(qid: str, action: str) -> bool:
    cat, exp = EXPECTED[qid]
    if cat == "reject":
        return action == "invalid_request"
    if action == "invalid_request":
        return False
    if action.startswith("drone_http_ok:"):
        tool = action.split(":", 1)[1]
        if cat == "simple":
            return tool == exp
        return tool in exp
    if action.startswith("sequence_stopped_at_step_"):
        failed_tool = action.split("_tool_", 1)[1]
        if cat == "simple":
            return failed_tool == exp
        return failed_tool in exp
    return False


def load_csv(path: Path) -> list[dict]:
    rows = []
    with path.open(encoding="utf-8") as f:
        for r in csv.DictReader(f):
            r["llm_ms"] = float(r["llm_ms"])
            r["total_ms"] = float(r["total_ms"])
            r["success"] = r["success"] == "True"
            rows.append(r)
    return rows


def summarize(rows: list[dict]) -> dict:
    dec = sum(decision_ok(r["query_id"], r["action_taken"]) for r in rows)
    pipe = sum(r["success"] for r in rows)
    llm = [r["llm_ms"] for r in rows]
    total = [r["total_ms"] for r in rows]
    return {
        "n": len(rows),
        "decision_acc": dec / len(rows) if rows else 0,
        "decision_ok": dec,
        "pipeline_acc": pipe / len(rows) if rows else 0,
        "pipeline_ok": pipe,
        "llm_mean": statistics.mean(llm) if llm else 0,
        "llm_median": statistics.median(llm) if llm else 0,
        "total_mean": statistics.mean(total) if total else 0,
        "total_median": statistics.median(total) if total else 0,
    }


def main() -> int:
    pairs = [
        ("0.8B Q5", Path("latency_runs/layers_0.8b_wired/latency_results.csv")),
        ("0.8B Q4", Path("latency_runs/layers_0.8b_q4/latency_results.csv")),
        ("2B Q5", Path("latency_runs/layers_2b_wired/latency_results.csv")),
        ("2B Q4", Path("latency_runs/layers_2b_q4/latency_results.csv")),
    ]
    bench = Path(__file__).resolve().parent
    print(f"{'label':<10} {'decision':>10} {'pipeline':>10} {'llm_mean':>10} {'total_mean':>11}")
    print("-" * 55)
    for label, rel in pairs:
        path = bench / rel
        if not path.is_file():
            print(f"{label:<10} MISSING {path}")
            continue
        s = summarize(load_csv(path))
        print(
            f"{label:<10} {s['decision_ok']:>4}/{s['n']:<4} "
            f"{s['pipeline_ok']:>4}/{s['n']:<4} "
            f"{s['llm_mean']:>9.0f}ms {s['total_mean']:>10.0f}ms"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
