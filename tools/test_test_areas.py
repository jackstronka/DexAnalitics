#!/usr/bin/env python3
"""Unit tests for tools/test_areas.py (hermetic, no cargo/network)."""

from __future__ import annotations

import io
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

import test_areas as ta

MAP = Path(__file__).resolve().parents[1] / "ci" / "test-areas.toml"


class NormalizeTests(unittest.TestCase):
    def test_nextest_oneline(self) -> None:
        self.assertEqual(
            ta.normalize_test_id(
                "clmm-lp-api openapi::tests::openapi_matches_committed_snapshot"
            ),
            "clmm-lp-api::openapi::tests::openapi_matches_committed_snapshot",
        )

    def test_web_path(self) -> None:
        self.assertEqual(
            ta.normalize_test_id(r"web\src\lib\whirlpoolTicks.test.ts"),
            "web/src/lib/whirlpoolTicks.test.ts",
        )


class AssignTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.areas = ta.load_areas(MAP)

    def test_lineage_golden(self) -> None:
        a = ta.assign(
            "clmm-lp-api::services::chain_economic_totals::tests::golden_9vhky_totals_computed_from_nodes",
            self.areas,
        )
        self.assertEqual(a.area_id, "lineage_pnl")
        self.assertFalse(a.fallback_only)

    def test_session_capital(self) -> None:
        a = ta.assign(
            "clmm-lp-execution::strategy::session_capital::tests::caps_cover",
            self.areas,
        )
        self.assertEqual(a.area_id, "ekonomia_gl")

    def test_openapi_over_api_root(self) -> None:
        a = ta.assign(
            "clmm-lp-api::openapi::tests::openapi_matches_committed_snapshot",
            self.areas,
        )
        self.assertEqual(a.area_id, "api_kontrakt")

    def test_web_ticks(self) -> None:
        a = ta.assign("web/src/lib/whirlpoolTicks.test.ts", self.areas)
        self.assertEqual(a.area_id, "domain_math")

    def test_unknown_api_service_falls_to_inne(self) -> None:
        a = ta.assign("clmm-lp-api::services::totally_new_module::tests::x", self.areas)
        self.assertEqual(a.area_id, "inne")
        self.assertTrue(a.fallback_only)

    def test_backtest_engine(self) -> None:
        a = ta.assign(
            "clmm-lp-cli::engine::golden_backtest::tests::golden_backtest_mini_run_single_ranking",
            self.areas,
        )
        self.assertEqual(a.area_id, "backtest_strategie")

    def test_orchestrator_gate(self) -> None:
        a = ta.assign(
            "clmm-lp-cli::orchestrator_gate::tests::no_go_on_bad_data",
            self.areas,
        )
        self.assertEqual(a.area_id, "decision_agent")


class CliSmokeTests(unittest.TestCase):
    def test_check_map(self) -> None:
        self.assertEqual(ta.main(["--check-map", "--map", str(MAP)]), 0)

    def test_strict_fails_on_fallback(self) -> None:
        with tempfile.NamedTemporaryFile(
            "w", encoding="utf-8", suffix=".txt", delete=False
        ) as fh:
            fh.write("clmm-lp-api services::totally_new_module::tests::x\n")
            path = Path(fh.name)
        try:
            buf = io.StringIO()
            with redirect_stdout(buf):
                code = ta.main(["--map", str(MAP), "--strict", "--file", str(path)])
            self.assertEqual(code, 1)
            self.assertIn("Inne", buf.getvalue())
        finally:
            path.unlink(missing_ok=True)

    def test_known_ids_strict_ok(self) -> None:
        with tempfile.NamedTemporaryFile(
            "w", encoding="utf-8", suffix=".txt", delete=False
        ) as fh:
            fh.write(
                "clmm-lp-api openapi::tests::openapi_matches_committed_snapshot\n"
                "clmm-lp-domain math::fee_math::tests::test_bps_to_decimal\n"
            )
            path = Path(fh.name)
        try:
            buf = io.StringIO()
            with redirect_stdout(buf):
                code = ta.main(["--map", str(MAP), "--strict", "--file", str(path)])
            self.assertEqual(code, 0, buf.getvalue())
        finally:
            path.unlink(missing_ok=True)


if __name__ == "__main__":
    raise SystemExit(unittest.main())
