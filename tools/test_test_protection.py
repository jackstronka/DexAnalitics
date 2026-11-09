#!/usr/bin/env python3
"""Unit tests for tools/test_protection.py (R5)."""

from __future__ import annotations

import json
import unittest
from pathlib import Path

from test_protection import (
    LEVELS,
    area_level,
    build_master_section,
    compute_levels,
    load_map,
    max_level,
    parse_master_protection,
)

REPO = Path(__file__).resolve().parents[1]
MAP = REPO / "ci" / "subsystem-protection.toml"
FIXTURE = REPO / "tools" / "fixtures" / "r5_report" / "report.json"
MASTER = REPO / "doc" / "MASTER_IMPLEMENTATION_PLAN.md"


class AreaLevelTests(unittest.TestCase):
    def test_ranking(self) -> None:
        self.assertEqual(area_level({"tests": 0, "golden": 0, "niezm": 0}), "brak")
        self.assertEqual(area_level({"tests": 5, "golden": 0, "niezm": 0}), "unit")
        self.assertEqual(area_level({"tests": 5, "golden": 1, "niezm": 0}), "golden")
        self.assertEqual(
            area_level({"tests": 5, "golden": 1, "niezm": 1}), "golden+niezmiennik"
        )
        self.assertEqual(max_level(["unit", "golden", "brak"]), "golden")

    def test_trend_field_aliases(self) -> None:
        self.assertEqual(
            area_level({"tests_last": 10, "golden_last": 2, "niezm_last": 1}),
            "golden+niezmiennik",
        )


class MapAndFixtureTests(unittest.TestCase):
    def test_map_loads(self) -> None:
        specs = load_map(MAP)
        names = [s.name for s in specs]
        self.assertIn("Live bot Orca", names)
        self.assertIn("Shadow strategies (ROADMAP)", names)
        close = next(s for s in specs if s.name == "Close-all")
        self.assertEqual(close.force, "unit")

    def test_fixture_levels(self) -> None:
        specs = load_map(MAP)
        report = json.loads(FIXTURE.read_text(encoding="utf-8"))
        areas = {str(a["label"]): a for a in report["areas"]}
        rows = compute_levels(specs, areas)
        by_name = {n: lv for n, lv, _ in rows}
        self.assertEqual(by_name["Wallet GL + SESSION"], "golden+niezmiennik")
        self.assertEqual(by_name["Stream lineage / PnL"], "golden+niezmiennik")
        self.assertEqual(by_name["Live bot Orca"], "golden")
        self.assertEqual(by_name["Backtest / optimize"], "golden")
        self.assertEqual(by_name["Dashboard (web)"], "unit")
        self.assertEqual(by_name["Close-all"], "unit")
        self.assertEqual(by_name["Shadow strategies (ROADMAP)"], "brak")
        self.assertEqual(by_name["Decision layer / orkiestrator"], "unit")
        self.assertEqual(by_name["Event parsing / fee truth"], "unit")


class MasterRoundtripTests(unittest.TestCase):
    def test_parse_and_check_repo_master(self) -> None:
        text = MASTER.read_text(encoding="utf-8")
        parsed = parse_master_protection(text)
        self.assertGreaterEqual(len(parsed), 10)
        for lv in parsed.values():
            self.assertIn(lv, LEVELS)

        specs = load_map(MAP)
        report = json.loads(FIXTURE.read_text(encoding="utf-8"))
        areas = {str(a["label"]): a for a in report["areas"]}
        expected = {n: lv for n, lv, _ in compute_levels(specs, areas)}
        self.assertEqual(parsed, expected)

    def test_write_rebuilds_column(self) -> None:
        text = MASTER.read_text(encoding="utf-8")
        levels = parse_master_protection(text)
        # Flip one level and rebuild
        flipped = dict(levels)
        flipped["Close-all"] = "brak"
        rebuilt = build_master_section(text, flipped)
        self.assertEqual(parse_master_protection(rebuilt)["Close-all"], "brak")
        # Restore via expected levels
        restored = build_master_section(rebuilt, levels)
        self.assertEqual(parse_master_protection(restored), levels)


if __name__ == "__main__":
    unittest.main()
