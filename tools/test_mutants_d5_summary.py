#!/usr/bin/env python3
"""Unit tests for tools/mutants_d5_summary.py (D5)."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from mutants_d5_summary import count_outcomes, main, survive_pct


class SummaryUnit(unittest.TestCase):
    def test_summary_block(self) -> None:
        t = count_outcomes(
            {"summary": {"caught": 9, "missed": 1, "unviable": 0, "timeout": 0}}
        )
        self.assertEqual(t["caught"], 9)
        self.assertEqual(t["missed"], 1)
        self.assertAlmostEqual(survive_pct(t) or -1, 10.0)

    def test_fixture_cli_ok(self) -> None:
        root = Path(__file__).resolve().parent / "fixtures" / "d5_outcomes"
        with tempfile.TemporaryDirectory() as td:
            out = Path(td) / "s.json"
            md = Path(td) / "s.md"
            rc = main(
                [
                    "--root",
                    str(root),
                    "--out",
                    str(out),
                    "--md",
                    str(md),
                    "--max-survive-pct",
                    "10.1",
                ]
            )
            self.assertEqual(rc, 0)
            data = json.loads(out.read_text(encoding="utf-8"))
            self.assertTrue(data["ok"])
            self.assertEqual(data["modules"][0]["module"], "wallet_session")

    def test_fails_when_over_threshold(self) -> None:
        root = Path(__file__).resolve().parent / "fixtures" / "d5_outcomes"
        rc = main(["--root", str(root), "--max-survive-pct", "5"])
        self.assertEqual(rc, 1)


if __name__ == "__main__":
    unittest.main()
