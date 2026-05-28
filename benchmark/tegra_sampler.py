#!/usr/bin/env python3
"""Optional Jetson tegrastats sampling for benchmark runs."""

from __future__ import annotations

import re
import subprocess
import time
from pathlib import Path
from typing import Any

TEGRASTATS = "/usr/bin/tegrastats"


def tegrastats_available() -> bool:
    return Path(TEGRASTATS).is_file()


class TegraSuiteLog:
    """Background tegrastats --logfile for the whole benchmark suite."""

    def __init__(self, log_path: Path):
        self.log_path = log_path
        self._proc: subprocess.Popen[str] | None = None

    def start(self) -> None:
        if not tegrastats_available():
            return
        self.log_path.parent.mkdir(parents=True, exist_ok=True)
        # Writes samples to log_path until process is terminated
        self._proc = subprocess.Popen(
            [TEGRASTATS, "--interval", "500", "--logfile", str(self.log_path)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            text=True,
        )
        time.sleep(0.3)

    def stop(self) -> None:
        if self._proc is None:
            return
        self._proc.terminate()
        try:
            self._proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self._proc.kill()
        self._proc = None


def parse_tegrastats_line(line: str) -> dict[str, Any]:
    """Best-effort parse of one tegrastats line (format varies by L4T)."""
    out: dict[str, Any] = {"raw": line.strip()}
    if not line.strip():
        return out
    # Power rails e.g. VDD_GPU_SOC 3456mW/1234mW
    for m in re.finditer(r"([A-Z0-9_]+)\s+(\d+)mW", line):
        out.setdefault("power_mw", {})[m.group(1)] = int(m.group(2))
    # GR3D usage / freq patterns seen on Jetson
    m = re.search(r"GR3D_FREQ\s+(\d+)%@[\d.]+", line)
    if m:
        out["gr3d_util_pct"] = int(m.group(1))
    m = re.search(r"GR3D_FREQ\s+([\d.]+)%", line)
    if m and "gr3d_util_pct" not in out:
        try:
            out["gr3d_util_pct"] = float(m.group(1))
        except ValueError:
            pass
    return out


def tegra_snapshot() -> dict[str, Any] | None:
    """Sample one line of tegrastats (short subprocess)."""
    if not tegrastats_available():
        return None
    try:
        r = subprocess.run(
            ["timeout", "1.5", TEGRASTATS, "--interval", "200"],
            capture_output=True,
            text=True,
            timeout=3,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    text = (r.stdout or "") + (r.stderr or "")
    lines = [ln for ln in text.splitlines() if ln.strip()]
    if not lines:
        return None
    return parse_tegrastats_line(lines[-1])


def summarize_log_file(log_path: Path) -> dict[str, Any]:
    """Aggregate power fields from a tegrastats --logfile output."""
    if not log_path.is_file():
        return {}
    lines = log_path.read_text(encoding="utf-8", errors="replace").splitlines()
    parsed = [parse_tegrastats_line(ln) for ln in lines if ln.strip()]
    if not parsed:
        return {"samples": 0}
    # Collect numeric series for VDD_GPU_SOC etc.
    keys: set[str] = set()
    for p in parsed:
        pm = p.get("power_mw")
        if isinstance(pm, dict):
            keys.update(pm.keys())
    series: dict[str, list[int]] = {k: [] for k in keys}
    gr3d: list[float] = []
    for p in parsed:
        pm = p.get("power_mw")
        if isinstance(pm, dict):
            for k, v in pm.items():
                series[k].append(v)
        if "gr3d_util_pct" in p and isinstance(p["gr3d_util_pct"], (int, float)):
            gr3d.append(float(p["gr3d_util_pct"]))
    def stat(xs: list[int]) -> dict[str, float]:
        if not xs:
            return {}
        return {"mean": sum(xs) / len(xs), "max": max(xs), "min": min(xs), "n": len(xs)}

    out: dict[str, Any] = {"samples": len(parsed), "rails_mw": {k: stat(v) for k, v in series.items()}}
    if gr3d:
        out["gr3d_util_pct"] = {"mean": sum(gr3d) / len(gr3d), "max": max(gr3d), "n": len(gr3d)}
    return out
