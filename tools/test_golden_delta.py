#!/usr/bin/env python3
"""Hermetic unit tests for tools/golden_delta.py (no git, no network)."""

from __future__ import annotations

import tempfile
import unittest
from decimal import Decimal
from pathlib import Path

from golden_delta import (
    diff_maps,
    fmt_dec,
    parse_snap_json,
    pct_change,
    render_table,
    rows_from_texts,
    try_number,
    walk_numeric,
)

SNAP = """---
source: crates/api/src/services/example.rs
expression: snapshot
---
{
  "headline": {
    "net_pnl_usd": "3.039781",
    "tx_fees_usd": "0.053128"
  },
  "rebalance_count": 1
}
"""


class ParseTests(unittest.TestCase):
    def test_parse_insta_header_then_json(self) -> None:
        obj = parse_snap_json(SNAP)
        assert obj["headline"]["net_pnl_usd"] == "3.039781"
        assert obj["rebalance_count"] == 1

    def test_ignores_iso_dates_and_bools(self) -> None:
        obj = {
            "window_start_utc": "2026-04-01T00:00:56Z",
            "ok": True,
            "label": "static",
        }
        self.assertEqual(walk_numeric(obj), {})

    def test_walk_numeric_paths(self) -> None:
        obj = parse_snap_json(SNAP)
        got = walk_numeric(obj)
        self.assertEqual(got["headline.net_pnl_usd"], Decimal("3.039781"))
        self.assertEqual(got["headline.tx_fees_usd"], Decimal("0.053128"))
        self.assertEqual(got["rebalance_count"], Decimal(1))


class DiffTests(unittest.TestCase):
    def test_table_sorted_by_abs_delta(self) -> None:
        old = walk_numeric(parse_snap_json(SNAP))
        new = dict(old)
        new["headline.net_pnl_usd"] = Decimal("1.000000")
        new["headline.tx_fees_usd"] = Decimal("0.053129")
        rows = diff_maps("b1", old, new)
        table = render_table(rows)
        self.assertIn("Golden delta", table)
        first_metric = [
            line for line in table.splitlines() if line.startswith("| `b1`")
        ][0]
        self.assertIn("headline.net_pnl_usd", first_metric)
        self.assertIn("-2.039781", first_metric)
        self.assertIn("Was", table.splitlines()[2] or table)

    def test_added_and_removed_leaves(self) -> None:
        old_text = SNAP
        new_obj = {
            "headline": {"net_pnl_usd": "3.039781", "fees_usd": "1.5"},
        }
        new_text = "---\nsource: x\n---\n" + __import__("json").dumps(new_obj)
        rows = rows_from_texts("fx", old_text, new_text)
        metrics = {r[2] for r in rows}
        self.assertIn("headline.tx_fees_usd", metrics)
        self.assertIn("headline.fees_usd", metrics)
        self.assertIn("rebalance_count", metrics)

    def test_identical_is_empty(self) -> None:
        rows = rows_from_texts("fx", SNAP, SNAP)
        self.assertEqual(rows, [])
        self.assertIn("no numeric leaf changes", render_table(rows))

    def test_pct_and_fmt(self) -> None:
        self.assertEqual(fmt_dec(Decimal("3.1000")), "3.1")
        self.assertEqual(pct_change(Decimal("2"), Decimal("1")), "-50%")
        self.assertEqual(pct_change(Decimal("0"), Decimal("1")), "-")
        self.assertIsNone(try_number("2026-04-01"))
        self.assertEqual(try_number("8.50000000"), Decimal("8.50000000"))

    def test_file_pair_roundtrip(self) -> None:
        from io import StringIO
        from unittest.mock import patch

        with tempfile.TemporaryDirectory() as tmp:
            old_p = Path(tmp) / "old.snap"
            new_p = Path(tmp) / "new.snap"
            old_p.write_text(SNAP, encoding="utf-8")
            new_p.write_text(
                SNAP.replace("3.039781", "3.000000"), encoding="utf-8"
            )
            from golden_delta import main

            with patch("sys.stdout", StringIO()) as buf:
                rc = main(
                    ["--old", str(old_p), "--new", str(new_p), "--fixture", "b1"]
                )
            self.assertEqual(rc, 0)
            self.assertIn("headline.net_pnl_usd", buf.getvalue())


if __name__ == "__main__":
    unittest.main()
