#!/usr/bin/env python3
"""Bucket benchmark cases for stratified scoring (bench_v1 mix)."""

from __future__ import annotations

from typing import Any

CATEGORIES = (
    "invalid",
    "single_model",
    "single_drone",
    "multi_short",
    "multi_long",
)


def classify_expected_tasks(tasks: list[dict[str, Any]]) -> str:
    if not tasks:
        return "invalid"
    if len(tasks) == 1 and tasks[0].get("category") == "none":
        return "invalid"
    if len(tasks) == 1 and tasks[0].get("category") == "model":
        return "single_model"
    if len(tasks) == 1 and tasks[0].get("category") == "drone":
        return "single_drone"
    if len(tasks) <= 3:
        return "multi_short"
    return "multi_long"


def classify_case(expected: dict[str, Any]) -> str:
    tasks = expected.get("tasks")
    if not isinstance(tasks, list):
        raise ValueError("expected must contain a tasks array")
    return classify_expected_tasks(tasks)
