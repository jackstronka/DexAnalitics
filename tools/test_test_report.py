#!/usr/bin/env python3
"""Hermetic unit tests for tools/test_report.py."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

import test_report as tr

ROOT = Path(__file__).resolve().parents[1]
FIX = Path(__file__).resolve().parent / "fixtures"


class ParseTests(unittest.TestCase):
    def test_nextest_ids_and_fail_class(self) -> None:
        cases = tr.parse_junit(FIX / "r2_nextest_junit.xml")
        self.assertEqual(len(cases), 3)
        areas = tr.ta.load_areas(tr.DEFAULT_MAP)
        tagged = [tr.tag_case(c, areas) for c in cases]
        by_id = {c.test_id: c for c in tagged}
        g = by_id[
            "clmm-lp-api::services::chain_economic_totals::tests::golden_9vhky_totals_computed_from_nodes"
        ]
        self.assertEqual(g.status, "pass")
        self.assertEqual(g.area_id, "lineage_pnl")
        self.assertTrue(g.is_golden)

        f = by_id[
            "clmm-lp-api::services::chain_economic_totals::tests::net_pnl_identity_after_refresh"
        ]
        self.assertEqual(f.status, "fail")
        self.assertEqual(f.fail_class, "economic_regression")
        self.assertTrue(f.is_niezm)

    def test_vitest_path(self) -> None:
        cases = tr.parse_junit(FIX / "r2_vitest_junit.xml")
        areas = tr.ta.load_areas(tr.DEFAULT_MAP)
        tagged = [tr.tag_case(c, areas) for c in cases]
        self.assertTrue(all(c.area_id == "domain_math" for c in tagged))
        self.assertTrue(all(c.test_id.startswith("web/src/lib/whirlpoolTicks") for c in tagged))


class ReportTests(unittest.TestCase):
    def test_markdown_and_json_roundtrip(self) -> None:
        areas = tr.ta.load_areas(tr.DEFAULT_MAP)
        cases = []
        for p in (FIX / "r2_nextest_junit.xml", FIX / "r2_vitest_junit.xml"):
            cases.extend(tr.tag_case(c, areas) for c in tr.parse_junit(p))
        report = tr.build_report(cases, git_ref="test/r2", commit="abc1234")
        md = tr.render_markdown(report)
        self.assertIn("PASS", md)
        self.assertIn("FAIL", md)
        self.assertIn("Lineage / PnL", md)
        self.assertIn("economic_regression", md)
        self.assertEqual(report.fail_n, 1)
        self.assertGreaterEqual(report.pass_n, 3)

        with tempfile.TemporaryDirectory() as td:
            out = Path(td)
            code = tr.main(
                [
                    "--junit",
                    str(FIX / "r2_nextest_junit.xml"),
                    "--junit",
                    str(FIX / "r2_vitest_junit.xml"),
                    "--out-dir",
                    str(out),
                    "--git-ref",
                    "test/r2",
                    "--commit",
                    "abc1234",
                ]
            )
            self.assertEqual(code, 1)  # has failures
            data = json.loads((out / "report.json").read_text(encoding="utf-8"))
            self.assertEqual(data["fail_n"], 1)
            self.assertTrue((out / "report.md").is_file())


if __name__ == "__main__":
    raise SystemExit(unittest.main())
