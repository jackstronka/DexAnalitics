#!/usr/bin/env python3
"""Hermetic unit tests for tools/test_trend.py."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

import test_trend as tt

FIX = Path(__file__).resolve().parent / "fixtures" / "r4_reports"


class TrendTests(unittest.TestCase):
    def test_local_reports_flake_and_table(self) -> None:
        reports = tt.collect_local_reports(FIX)
        self.assertEqual(len(reports), 2)
        trend = tt.build_trend(reports, branch="main", window_days=14)
        self.assertEqual(trend.snapshots, 2)
        labels = {a["label"] for a in trend.areas}
        self.assertIn("Lineage / PnL", labels)
        self.assertIn("Domain math", labels)
        self.assertTrue(
            any("flaky_example" in f["id"] for f in trend.flake_candidates),
            trend.flake_candidates,
        )
        md = tt.render_markdown(trend)
        self.assertIn("Test trend", md)
        self.assertIn("Flake candidates", md)

    def test_cli_writes_outputs(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            out = Path(td)
            code = tt.main(
                [
                    "--reports-dir",
                    str(FIX),
                    "--out-dir",
                    str(out),
                    "--branch",
                    "main",
                    "--days",
                    "14",
                ]
            )
            self.assertEqual(code, 0)
            data = json.loads((out / "trend.json").read_text(encoding="utf-8"))
            self.assertEqual(data["snapshots"], 2)
            self.assertTrue((out / "trend.md").is_file())


if __name__ == "__main__":
    raise SystemExit(unittest.main())
