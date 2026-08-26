#!/usr/bin/env python3
"""Generate a print-ready PDF of bench_v1 cases from manifest.json."""

from __future__ import annotations

import json
from pathlib import Path

from fpdf import FPDF

HERE = Path(__file__).resolve().parent
MANIFEST = HERE / "manifest.json"
OUT = HERE / "bench_v1_cases.pdf"

CATEGORY_TITLES = {
    "invalid": "Invalid / no action (invalid_request)",
    "single_model": "Single model tool",
    "single_drone": "Single drone tool",
    "multi_short": "Multi-step (2-3 tasks)",
    "multi_long": "Multi-step (4-5 tasks)",
}


class BenchPDF(FPDF):
    def __init__(self) -> None:
        super().__init__(orientation="P", unit="mm", format="A4")
        self.set_auto_page_break(auto=True, margin=18)
        self._section = ""

    def header(self) -> None:
        if self.page_no() == 1:
            return
        self.set_font("Helvetica", "I", 8)
        self.set_text_color(100, 100, 100)
        self.cell(0, 6, f"SAR LLM bench_v1  |  {self._section}", align="L")
        self.ln(4)

    def footer(self) -> None:
        self.set_y(-12)
        self.set_font("Helvetica", "I", 8)
        self.set_text_color(120, 120, 120)
        self.cell(0, 8, f"Page {self.page_no()}", align="C")


def _sanitize(text: str) -> str:
    """FPDF core fonts are Latin-1; replace problematic chars."""
    return (
        text.replace("\u2013", "-")
        .replace("\u2014", "-")
        .replace("\u2018", "'")
        .replace("\u2019", "'")
        .replace("\u201c", '"')
        .replace("\u201d", '"')
        .encode("latin-1", errors="replace")
        .decode("latin-1")
    )


def main() -> None:
    data = json.loads(MANIFEST.read_text(encoding="utf-8"))
    cases = data["cases"]

    pdf = BenchPDF()
    pdf.add_page()

    # Title page
    pdf.set_font("Helvetica", "B", 22)
    pdf.set_text_color(20, 40, 80)
    pdf.ln(25)
    pdf.cell(0, 12, "SAR Drone Gateway", align="C", new_x="LMARGIN", new_y="NEXT")
    pdf.set_font("Helvetica", "B", 16)
    pdf.cell(0, 10, "LLM Benchmark: bench_v1", align="C", new_x="LMARGIN", new_y="NEXT")
    pdf.ln(8)
    pdf.set_font("Helvetica", "", 11)
    pdf.set_text_color(60, 60, 60)
    pdf.multi_cell(
        0,
        6,
        _sanitize(
            "100 prompts with gold expected JSON (tasks array). "
            "20 cases per category: invalid, single model, single drone, "
            "multi short, multi long."
        ),
        align="C",
    )
    pdf.ln(6)
    pdf.set_font("Helvetica", "", 9)
    pdf.multi_cell(
        0,
        5,
        _sanitize(
            f"Pass (decision): {data['pass_rule']['decision']}\n"
            f"Prompt contract: gateway SAR_SYSTEM_PROMPT (see llm.rs)"
        ),
        align="C",
    )

    last_cat = None
    for c in cases:
        cat = c["category"]
        if cat != last_cat:
            pdf.add_page()
            pdf._section = CATEGORY_TITLES.get(cat, cat)
            pdf.set_font("Helvetica", "B", 14)
            pdf.set_text_color(20, 40, 80)
            pdf.cell(0, 10, _sanitize(pdf._section), new_x="LMARGIN", new_y="NEXT")
            pdf.set_draw_color(20, 40, 80)
            pdf.line(10, pdf.get_y(), 200, pdf.get_y())
            pdf.ln(6)
            last_cat = cat

        idx = c["index"]
        inp = _sanitize(c["input"])
        exp = json.dumps(c["expected"], separators=(",", ":"), ensure_ascii=False)
        exp = _sanitize(exp)

        # Case header
        pdf.set_font("Helvetica", "B", 10)
        pdf.set_text_color(0, 0, 0)
        pdf.set_fill_color(240, 244, 248)
        pdf.cell(0, 7, f"  Case {idx}  [{cat}]", new_x="LMARGIN", new_y="NEXT", fill=True)
        pdf.ln(2)

        pdf.set_font("Helvetica", "B", 9)
        pdf.set_text_color(80, 80, 80)
        pdf.cell(18, 5, "Input:")
        pdf.set_font("Helvetica", "", 9)
        pdf.set_text_color(0, 0, 0)
        pdf.multi_cell(0, 5, inp)
        pdf.ln(1)

        pdf.set_font("Helvetica", "B", 9)
        pdf.set_text_color(80, 80, 80)
        pdf.cell(0, 5, "Expected output:", new_x="LMARGIN", new_y="NEXT")
        pdf.set_font("Courier", "", 7.5)
        pdf.set_text_color(30, 30, 30)
        pdf.multi_cell(0, 3.8, exp)
        pdf.ln(5)

    pdf.output(OUT)
    print(f"Wrote {OUT} ({OUT.stat().st_size // 1024} KiB)")


if __name__ == "__main__":
    main()
