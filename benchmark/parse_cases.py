#!/usr/bin/env python3
"""Parse benchmark case files: Input: ... / Expected output: {...} blocks."""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Any


@dataclass
class Case:
    index: int
    input_text: str
    expected: dict[str, Any]


_INPUT_RE = re.compile(r"^\s*Input:\s*(.+?)(?=\s*Expected output:)", re.MULTILINE | re.DOTALL)
_EXPECTED_RE = re.compile(
    r"Expected output:\s*(\{[\s\S]*\})\s*$",
    re.MULTILINE | re.DOTALL,
)


def _split_blocks(text: str) -> list[str]:
    text = text.strip()
    if not text:
        return []
    # Blank-line separated blocks; also tolerate "---" separators
    parts = re.split(r"\n\s*\n|(?:^|\n)\s*---+\s*(?:\n|$)", text)
    return [p.strip() for p in parts if p.strip()]


def load_cases(path: str | Path) -> list[Case]:
    raw = Path(path).read_text(encoding="utf-8", errors="replace")
    blocks = _split_blocks(raw)
    cases: list[Case] = []
    for idx, block in enumerate(blocks, start=1):
        m_in = _INPUT_RE.search(block)
        m_out = _EXPECTED_RE.search(block)
        if not m_in or not m_out:
            raise ValueError(
                f"Block {idx}: missing Input: or Expected output: (first 200 chars):\n{block[:200]!r}"
            )
        input_text = m_in.group(1).strip()
        json_str = m_out.group(1).strip()
        try:
            expected = json.loads(json_str)
        except json.JSONDecodeError as e:
            raise ValueError(f"Block {idx}: invalid JSON in Expected output: {e}") from e
        if "tasks" not in expected or not isinstance(expected["tasks"], list):
            raise ValueError(f"Block {idx}: Expected output must be a JSON object with a 'tasks' array")
        cases.append(Case(index=idx, input_text=input_text, expected=expected))
    return cases
