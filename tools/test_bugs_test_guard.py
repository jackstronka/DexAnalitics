#!/usr/bin/env python3
"""Hermetic unit tests for tools/bugs_test_guard.py (no git, no network)."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from bugs_test_guard import (
    TestIndex,
    build_test_index,
    check_entries,
    extract_guard_names,
    parse_high_critical_entries,
)

SAMPLE = """# BUGS

### BUG-20260101-01 — has a real test

status: open
severity: high
reported_by: ai
first_seen: 2026-01-01
fixed_in:
keywords: sample

- **Symptom:** x
- **Root cause:** y
- **Fix:** z
- **Guards/tests:** `orphan_close_detected_only_for_registry_close_without_lifecycle_close`
- **Paths:** `a.rs`

---

### BUG-20260101-02 — cargo check is not a test

status: fixed
severity: critical
reported_by: ai
first_seen: 2026-01-01
fixed_in: local
keywords: sample

- **Symptom:** x
- **Root cause:** y
- **Fix:** z
- **Guards/tests:** `cargo check -p clmm-lp-api`
- **Paths:** `b.rs`

---

### BUG-20260101-03 — explicit manual exception

status: open
severity: high
reported_by: user
first_seen: 2026-01-01
fixed_in:
keywords: sample

- **Symptom:** x
- **Root cause:** y
- **Fix:** z
- **Guards/tests:** manual: reproduce on /positions/new after balances settle.
- **Paths:** `c.tsx`

---

### BUG-20260101-04 — medium is ignored

status: open
severity: medium
reported_by: ai
first_seen: 2026-01-01
fixed_in:
keywords: sample

- **Symptom:** x
- **Root cause:** y
- **Fix:** z
- **Guards/tests:** none
- **Paths:** `d.rs`

---

### BUG-20260101-05 — named test missing from repo

status: fixed
severity: high
reported_by: ai
first_seen: 2026-01-01
fixed_in: local
keywords: sample

- **Symptom:** x
- **Root cause:** y
- **Fix:** z
- **Guards/tests:** `this_test_does_not_exist_anywhere`
- **Paths:** `e.rs`
"""


class ParseTests(unittest.TestCase):
    def test_only_high_critical(self) -> None:
        entries = parse_high_critical_entries(SAMPLE)
        self.assertEqual(
            [e.bug_id for e in entries],
            [
                "BUG-20260101-01",
                "BUG-20260101-02",
                "BUG-20260101-03",
                "BUG-20260101-05",
            ],
        )

    def test_extract_backtick_and_cargo_filter(self) -> None:
        names = extract_guard_names(
            "`cargo test -p clmm-lp-data --test session_gl_integration` (4/4)"
        )
        self.assertIn("session_gl_integration", names.names)
        self.assertFalse(names.manual)

    def test_manual_flag(self) -> None:
        names = extract_guard_names("manual: cold load /positions/new")
        self.assertTrue(names.manual)
        self.assertEqual(names.names, ())

    def test_ignores_cargo_check(self) -> None:
        names = extract_guard_names("`cargo check -p clmm-lp-execution`")
        self.assertEqual(names.names, ())


class CheckTests(unittest.TestCase):
    def test_present_missing_manual(self) -> None:
        index = TestIndex(
            fn_names=frozenset(
                {"orphan_close_detected_only_for_registry_close_without_lifecycle_close"}
            ),
            file_stems=frozenset(),
        )
        entries = parse_high_critical_entries(SAMPLE)
        violations = check_entries(entries, index)
        ids = {v.bug_id for v in violations}
        self.assertNotIn("BUG-20260101-01", ids)
        self.assertIn("BUG-20260101-02", ids)
        self.assertNotIn("BUG-20260101-03", ids)
        self.assertIn("BUG-20260101-05", ids)
        miss = next(v for v in violations if v.bug_id == "BUG-20260101-05")
        self.assertEqual(miss.missing, ("this_test_does_not_exist_anywhere",))

    def test_one_existing_name_is_enough(self) -> None:
        index = TestIndex(
            fn_names=frozenset({"golden_reopen_sizing_table"}),
            file_stems=frozenset(),
        )
        from bugs_test_guard import BugEntry

        entries = [
            BugEntry(
                bug_id="BUG-20990101-01",
                severity="high",
                guards="`golden_reopen_sizing_table`; row `must_not_follow_smaller_wallet`",
            )
        ]
        self.assertEqual(check_entries(entries, index), [])

    def test_index_from_temp_tree(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            rs = root / "crates" / "api" / "src" / "foo.rs"
            rs.parent.mkdir(parents=True)
            rs.write_text(
                "#[cfg(test)]\nmod tests {\n    #[test]\n    fn golden_reopen_sizing_table() {}\n}\n",
                encoding="utf-8",
            )
            integ = root / "crates" / "data" / "tests" / "session_gl_integration.rs"
            integ.parent.mkdir(parents=True)
            integ.write_text("#[test]\nfn session_gl_lifecycle_posting_matches_pslr_and_caps() {}\n", encoding="utf-8")
            web = root / "web" / "src" / "lib" / "api.gen.test.ts"
            web.parent.mkdir(parents=True)
            web.write_text("it('HealthResponse sample matches the generated schema', () => {})\n", encoding="utf-8")
            index = build_test_index(root)
            self.assertTrue(index.contains("golden_reopen_sizing_table"))
            self.assertTrue(index.contains("session_gl_integration"))
            self.assertTrue(index.contains("session_gl_lifecycle_posting_matches_pslr_and_caps"))
            self.assertTrue(index.contains("HealthResponse sample matches the generated schema"))

            nested = root / "crates" / "execution" / "src" / "rebalance.rs"
            nested.parent.mkdir(parents=True, exist_ok=True)
            nested.write_text(
                "mod swap_mix_sol_first_tests {\n    #[test]\n    fn native_only() {}\n}\n",
                encoding="utf-8",
            )
            index2 = build_test_index(root)
            self.assertTrue(index2.contains("swap_mix_sol_first"))


if __name__ == "__main__":
    unittest.main()
