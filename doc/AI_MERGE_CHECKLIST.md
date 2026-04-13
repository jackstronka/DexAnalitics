# AI Merge Checklist (Regression-focused)

Use this checklist before merging bugfixes and feature PRs touching execution, lineage, or position UX.

- [ ] **Spec drift:** implementation still matches the intended behavior and API contract.
- [ ] **Data reuse first:** checked `doc/DATA_CATALOG.md` and reused existing tagged source before adding new snapshots/ingestion.
- [ ] **Bug -> test rule:** each `high`/`critical` bug has a regression or invariant test.
- [ ] **Critical path test coverage:** if critical files changed, test files or test modules changed too.
- [ ] **Invariant checks:** lineage continuity sanity is preserved (close/open rotation, baseline/end logic).
- [ ] **Error paths:** dry-run, unavailable executor/wallet, and partial-data paths are explicit in API/UI messages.
- [ ] **Shadow/diff check:** for the same fixture/data slice compare old/new metrics and explain material deltas.
- [ ] **Rollback safety:** change can be reverted or feature-flagged without data corruption.

Recommended command set for Rust/API changes:

```bash
cargo test -p clmm-lp-api position_stream_lineage -- --nocapture
cargo test -p clmm-lp-api
```
