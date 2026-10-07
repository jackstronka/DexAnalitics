# AI Merge Checklist (Regression-focused)

Use this checklist before merging bugfixes and feature PRs touching execution, lineage, or position UX.

- [ ] **Spec drift:** implementation still matches the intended behavior and API contract.
- [ ] **Data reuse first:** checked `doc/DATA_CATALOG.md` and reused existing tagged source before adding new snapshots/ingestion.
- [ ] **Bug -> test rule:** each `high`/`critical` bug has a regression or invariant test.
- [ ] **Critical path test coverage:** if critical files changed (list in `scripts/ci/critical-area-test-gate.sh`), the diff adds/changes tests — a file merely containing `mod tests` does not count. Label `no-tests-needed` only for pure refactors, reason in PR body.
- [ ] **Test integrity** (`.cursor/rules/test-integrity.mdc`): no deleted/weakened assertions, no `#[ignore]` or wider tolerances to get green; every failing test classified (`stale_test` / `non_hermetic` / `economic_regression` / `assertion_weakened` / `infra`).
- [ ] **Golden delta:** if any snapshot / fixture / `openapi.json` changed, the PR description has a "Golden delta: what changed and why" section (paste the table from `python tools/golden_delta.py --git-base origin/main` / CI job `golden_delta`). Job `golden_delta` fails without that section (D1); number diffs alone do not.
- [ ] **Hermetic tests:** new tests use no network, no repo `data/`, env only via `test_env::EnvGuard` (CI runs tests without network).
- [ ] **Plan test section:** a new or changed plan document has "Testy i kryteria regresji" (`doc/templates/TEST_SECTION.md`).
- [ ] **`doc/TESTS.md` catalog:** new test file / golden / integration harness / `#[ignore]` / web `*.test.ts` / new suite → same-PR update of the catalog (what it guards + how to run). Skip only when adding another unit test in an already-described module.
- [ ] **Invariant checks:** lineage continuity sanity is preserved (close/open rotation, baseline/end logic).
- [ ] **Error paths:** dry-run, unavailable executor/wallet, and partial-data paths are explicit in API/UI messages.
- [ ] **Shadow/diff check:** for the same fixture/data slice compare old/new metrics and explain material deltas.
- [ ] **Rollback safety:** change can be reverted or feature-flagged without data corruption.

Recommended command set for Rust/API changes:

```bash
cargo test -p clmm-lp-api position_stream_lineage -- --nocapture
cargo test -p clmm-lp-api
```
