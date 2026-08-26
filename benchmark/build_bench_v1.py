#!/usr/bin/env python3
"""
Build benchmark/releases/bench_v1/ (100 cases, 20 per category).

Sources the legacy pool (llm_edge_test_cases_100.txt) plus authored fill-ins
where the pool is short on single_drone / multi_long / single_model.
"""

from __future__ import annotations

import json
from pathlib import Path

from classify_case import CATEGORIES, classify_case
from parse_cases import Case, load_cases

ROOT = Path(__file__).resolve().parent
RELEASE = ROOT / "releases" / "bench_v1"
SOURCE = ROOT / "llm_edge_test_cases_100.txt"
PER_CAT = 20

# Authored fill-ins (input, expected tasks list) — not in legacy pool or under-represented.
FILL_INS: dict[str, list[tuple[str, list[dict]]]] = {
    "single_drone": [
        ("return home now", [{"category": "drone", "name": "return_to_home"}]),
        ("land immediately", [{"category": "drone", "name": "land_immediately"}]),
        ("pause the mission and hold", [{"category": "drone", "name": "mission_interrupt"}]),
        ("resume the mission", [{"category": "drone", "name": "mission_resume"}]),
        ("go to waypoint index 1", [{"category": "drone", "name": "mission_set_current", "params": {"seq": 1}}]),
        ("execute the mission", [{"category": "drone", "name": "start_mission"}]),
        ("RTL now", [{"category": "drone", "name": "return_to_home"}]),
        ("land right now", [{"category": "drone", "name": "land_immediately"}]),
        ("skip to waypoint 4", [{"category": "drone", "name": "mission_set_current", "params": {"seq": 4}}]),
        ("interrupt AUTO mission", [{"category": "drone", "name": "mission_interrupt"}]),
        ("continue the mission after hold", [{"category": "drone", "name": "mission_resume"}]),
        ("run the mission on the drone", [{"category": "drone", "name": "start_mission"}]),
    ],
    "single_model": [
        ("classify the image", [{"category": "model", "name": "flood_class"}]),
        ("run human detection", [{"category": "model", "name": "human_detect"}]),
        ("segment flood water", [{"category": "model", "name": "flood_seg"}]),
        ("flood classification please", [{"category": "model", "name": "flood_class"}]),
    ],
    "multi_short": [
        (
            "fly to 23.563206, 120.477799",
            [
                {"category": "drone", "name": "arm"},
                {"category": "drone", "name": "takeoff"},
                {
                    "category": "drone",
                    "name": "goto_location",
                    "params": {"lat_deg": 23.563206, "lon_deg": 120.477799, "alt_m": 15},
                },
            ],
        ),
    ],
    "multi_long": [
        (
            "take off to 30 m, fly to 23.5, 120.5 at 30 m, then detect people and classify flood severity",
            [
                {"category": "drone", "name": "arm"},
                {"category": "drone", "name": "takeoff", "params": {"altitude_m": 30}},
                {"category": "drone", "name": "goto_location", "params": {"lat_deg": 23.5, "lon_deg": 120.5, "alt_m": 30}},
                {"category": "model", "name": "human_detect"},
                {"category": "model", "name": "flood_class"},
            ],
        ),
        (
            "fly to 22.9, 120.3 at 35 m, then highlight flooded areas and search for survivors",
            [
                {"category": "drone", "name": "arm"},
                {"category": "drone", "name": "takeoff", "params": {"altitude_m": 35}},
                {"category": "drone", "name": "goto_location", "params": {"lat_deg": 22.9, "lon_deg": 120.3, "alt_m": 35}},
                {"category": "model", "name": "flood_seg"},
                {"category": "model", "name": "human_detect"},
            ],
        ),
        (
            "from the ground, go to 24.1, 121.2 at 45 m, then circle search and detect people",
            [
                {"category": "drone", "name": "arm"},
                {"category": "drone", "name": "takeoff", "params": {"altitude_m": 45}},
                {"category": "drone", "name": "goto_location", "params": {"lat_deg": 24.1, "lon_deg": 121.2, "alt_m": 45}},
                {"category": "model", "name": "human_detect"},
            ],
        ),
        (
            "arm, take off to 25 m, go to 37.0, -122.0 at 25 m, then run flood segmentation",
            [
                {"category": "drone", "name": "arm"},
                {"category": "drone", "name": "takeoff", "params": {"altitude_m": 25}},
                {"category": "drone", "name": "goto_location", "params": {"lat_deg": 37.0, "lon_deg": -122.0, "alt_m": 25}},
                {"category": "model", "name": "flood_seg"},
            ],
        ),
        (
            "launch to 20 m, fly to 23.56, 120.47 at 20 m, then detect people",
            [
                {"category": "drone", "name": "arm"},
                {"category": "drone", "name": "takeoff", "params": {"altitude_m": 20}},
                {"category": "drone", "name": "goto_location", "params": {"lat_deg": 23.56, "lon_deg": 120.47, "alt_m": 20}},
                {"category": "model", "name": "human_detect"},
            ],
        ),
        (
            "take off to 15 m, goto 23.563206, 120.477799 at 15 m, then classify flood type",
            [
                {"category": "drone", "name": "arm"},
                {"category": "drone", "name": "takeoff", "params": {"altitude_m": 15}},
                {
                    "category": "drone",
                    "name": "goto_location",
                    "params": {"lat_deg": 23.563206, "lon_deg": 120.477799, "alt_m": 15},
                },
                {"category": "model", "name": "flood_class"},
            ],
        ),
        (
            "take off, fly to 25.04, 121.56 at 20 m, highlight flooded areas",
            [
                {"category": "drone", "name": "arm"},
                {"category": "drone", "name": "takeoff", "params": {"altitude_m": 20}},
                {"category": "drone", "name": "goto_location", "params": {"lat_deg": 25.04, "lon_deg": 121.56, "alt_m": 20}},
                {"category": "model", "name": "flood_seg"},
            ],
        ),
        (
            "go to 23.7, 120.9 at 25 m then classify flood severity",
            [
                {"category": "drone", "name": "arm"},
                {"category": "drone", "name": "takeoff", "params": {"altitude_m": 25}},
                {"category": "drone", "name": "goto_location", "params": {"lat_deg": 23.7, "lon_deg": 120.9, "alt_m": 25}},
                {"category": "model", "name": "flood_class"},
            ],
        ),
        (
            "fly to 23.5, 120.5 at 30 m, detect people and classify flood severity",
            [
                {"category": "drone", "name": "arm"},
                {"category": "drone", "name": "takeoff", "params": {"altitude_m": 30}},
                {"category": "drone", "name": "goto_location", "params": {"lat_deg": 23.5, "lon_deg": 120.5, "alt_m": 30}},
                {"category": "model", "name": "human_detect"},
                {"category": "model", "name": "flood_class"},
            ],
        ),
        (
            "from ground: 23.6, 120.4 at 30 m then detect people",
            [
                {"category": "drone", "name": "arm"},
                {"category": "drone", "name": "takeoff", "params": {"altitude_m": 30}},
                {"category": "drone", "name": "goto_location", "params": {"lat_deg": 23.6, "lon_deg": 120.4, "alt_m": 30}},
                {"category": "model", "name": "human_detect"},
            ],
        ),
        (
            "take off to 12 m, goto 23.5, 120.5 at 12 m, detect humans",
            [
                {"category": "drone", "name": "arm"},
                {"category": "drone", "name": "takeoff", "params": {"altitude_m": 12}},
                {"category": "drone", "name": "goto_location", "params": {"lat_deg": 23.5, "lon_deg": 120.5, "alt_m": 12}},
                {"category": "model", "name": "human_detect"},
            ],
        ),
        (
            "arm, takeoff to 18 m, goto 23.56, 120.47 at 18 m, then human_detect",
            [
                {"category": "drone", "name": "arm"},
                {"category": "drone", "name": "takeoff", "params": {"altitude_m": 18}},
                {"category": "drone", "name": "goto_location", "params": {"lat_deg": 23.56, "lon_deg": 120.47, "alt_m": 18}},
                {"category": "model", "name": "human_detect"},
            ],
        ),
        (
            "launch to 30 m, reposition to 24.0, 121.0 at 30 m, flood segmentation",
            [
                {"category": "drone", "name": "arm"},
                {"category": "drone", "name": "takeoff", "params": {"altitude_m": 30}},
                {"category": "drone", "name": "goto_location", "params": {"lat_deg": 24.0, "lon_deg": 121.0, "alt_m": 30}},
                {"category": "model", "name": "flood_seg"},
            ],
        ),
        (
            "take off to 22 m, fly to 37.12, -122.1 at 22 m, then detect people",
            [
                {"category": "drone", "name": "arm"},
                {"category": "drone", "name": "takeoff", "params": {"altitude_m": 22}},
                {"category": "drone", "name": "goto_location", "params": {"lat_deg": 37.12, "lon_deg": -122.1, "alt_m": 22}},
                {"category": "model", "name": "human_detect"},
            ],
        ),
    ],
}


def _case_key(c: Case) -> tuple[str, str]:
    return (c.input_text.strip(), json.dumps(c.expected["tasks"], sort_keys=True))


def _pick(pool: list[Case], cat: str, n: int, used: set[tuple[str, str]]) -> list[Case]:
    out: list[Case] = []
    for c in pool:
        if classify_case(c.expected) != cat:
            continue
        k = _case_key(c)
        if k in used:
            continue
        out.append(c)
        used.add(k)
        if len(out) >= n:
            break
    return out


def _fill_cases(cat: str, n: int, used: set[tuple[str, str]]) -> list[Case]:
    out: list[Case] = []
    for inp, tasks in FILL_INS.get(cat, []):
        if len(out) >= n:
            break
        expected = {"tasks": tasks}
        k = (inp.strip(), json.dumps(tasks, sort_keys=True))
        if k in used:
            continue
        used.add(k)
        out.append(Case(index=0, input_text=inp, expected=expected))
    return out


def build() -> list[tuple[str, Case]]:
    pool = load_cases(SOURCE)
    used: set[tuple[str, str]] = set()
    ordered: list[tuple[str, Case]] = []

    for cat in CATEGORIES:
        picked = _pick(pool, cat, PER_CAT, used)
        if len(picked) < PER_CAT:
            picked.extend(_fill_cases(cat, PER_CAT - len(picked), used))
        if len(picked) < PER_CAT:
            raise SystemExit(f"bench_v1: need {PER_CAT} for {cat}, got {len(picked)}")
        for c in picked[:PER_CAT]:
            ordered.append((cat, c))

    return ordered


def format_cases(ordered: list[tuple[str, Case]]) -> str:
    parts: list[str] = []
    n = 0
    last_cat = None
    for cat, c in ordered:
        if cat != last_cat:
            parts.append(f"--- bench_v1: {cat} (20 cases) ---")
            last_cat = cat
        n += 1
        parts.append(f"CASE {n} [{cat}]")
        parts.append(f"Input: {c.input_text}")
        parts.append(
            "Expected output: "
            + json.dumps(c.expected, separators=(",", ":"), ensure_ascii=False)
        )
        parts.append("")
    return "\n".join(parts).rstrip() + "\n"


def main() -> int:
    ordered = build()
    RELEASE.mkdir(parents=True, exist_ok=True)
    cases_path = RELEASE / "cases.txt"
    cases_path.write_text(format_cases(ordered), encoding="utf-8")

    manifest = {
        "id": "bench_v1",
        "version": 1,
        "description": "Stratified SAR LLM router benchmark (100 cases, 20 per category).",
        "categories": list(CATEGORIES),
        "per_category": PER_CAT,
        "pass_rule": {
            "decision": "json_valid AND intent_match_effective AND params_match_effective",
            "e2e": "same LLM JSON as decision; invalid cases must not apply drone tools; drone cases may require ACK per gateway config",
        },
        "prompt_ref": "../src/llm.rs SAR_SYSTEM_PROMPT",
        "cases_file": "cases.txt",
        "cases": [
            {
                "index": i + 1,
                "category": cat,
                "input": c.input_text,
                "expected": c.expected,
            }
            for i, (cat, c) in enumerate(ordered)
        ],
    }
    (RELEASE / "manifest.json").write_text(
        json.dumps(manifest, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    print(f"Wrote {cases_path} and {RELEASE / 'manifest.json'} ({len(ordered)} cases)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
