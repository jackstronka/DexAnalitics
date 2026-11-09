#!/usr/bin/env python3
"""Unit tests for tools/verify_fast.py (E1)."""

from __future__ import annotations

import unittest

from verify_fast import needs_web, package_for_path, select_packages


class SelectPackages(unittest.TestCase):
    def test_domain_pulls_reverse_deps(self) -> None:
        pkgs, full, web = select_packages(["crates/domain/src/lib.rs"])
        self.assertFalse(full)
        self.assertFalse(web)
        self.assertIn("clmm-lp-domain", pkgs)
        self.assertIn("clmm-lp-api", pkgs)
        self.assertIn("clmm-lp-cli", pkgs)
        self.assertIn("clmm-lp-execution", pkgs)

    def test_api_only(self) -> None:
        pkgs, full, web = select_packages(["crates/api/src/handlers/health.rs"])
        self.assertEqual(pkgs, ["clmm-lp-api"])
        self.assertFalse(full)
        self.assertFalse(web)

    def test_openapi_triggers_web(self) -> None:
        pkgs, full, web = select_packages(["crates/api/openapi.json"])
        self.assertTrue(web)
        self.assertIn("clmm-lp-api", pkgs)

    def test_web_only(self) -> None:
        pkgs, full, web = select_packages(["web/src/lib/api.ts"])
        self.assertEqual(pkgs, [])
        self.assertFalse(full)
        self.assertTrue(web)

    def test_root_cargo_full(self) -> None:
        pkgs, full, web = select_packages(["Cargo.toml"])
        self.assertTrue(full)
        self.assertGreaterEqual(len(pkgs), 8)

    def test_package_for_path(self) -> None:
        self.assertEqual(package_for_path("crates/data/src/wallet_session.rs"), "clmm-lp-data")
        self.assertIsNone(package_for_path("doc/TESTS.md"))

    def test_needs_web(self) -> None:
        self.assertTrue(needs_web(["web/src/App.tsx"]))
        self.assertTrue(needs_web(["crates/api/openapi.json"]))
        self.assertFalse(needs_web(["crates/domain/src/lib.rs"]))


if __name__ == "__main__":
    unittest.main()
