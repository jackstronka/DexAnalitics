# Bugbot — test integrity (D3 / G6)

Project-specific PR review rules for Cursor Bugbot. Editor rules in `.cursor/rules/*.mdc` do **not** apply to Bugbot; this file does.

Canonical policy: `doc/IMPLEMENTATION_PLAN_REGRESSION_RESILIENCE.md` §3–§4 and `.cursor/rules/test-integrity.mdc`.

When any rule below matches, **leave a PR comment** naming the class `assertion_weakened` (or `economic_regression` for money goldens) and the file/hunk. Do not treat these as style nits.

## Always flag

1. **Weakened assertion** — a check becomes less strict without a documented product change, e.g.:
   - concrete expected value / `assert_eq!` / `assert_ne!` / snapshot assert replaced by `is_ok()`, `is_err()`, `is_some()`, `is_none()`, bare `assert!(true)`, or removed assert
   - tighter tolerance / epsilon / decimal places loosened (wider `abs_diff`, larger `approx`, fewer dp) to make CI green
   - Vitest/Jest: `toEqual` / `toBeCloseTo` replaced by weaker matchers, or expected numbers deleted
2. **Deleted or emptied golden / snapshot / fixture** — removal or gutting of `*.snap`, files under `**/tests/fixtures/**`, `**/snapshots/**`, or committed expected JSON used as golden, unless the PR description has a **Golden delta:** section that justifies the change (and money diffs are explicitly approved)
3. **New or broadened `#[ignore]`** on a previously active test (Rust) or equivalent skip (`it.skip`, `describe.skip`, `test.skip`, `xit`) used to silence a failure — especially without `manual:` reason for real-data / network repair jobs
4. **Vacuous early `return` in a test** — `return;` / `return Ok(());` / early exit before the assertions that previously guarded behavior (including “skip if missing DB/env” that turns into a silent pass when `CLMM_REQUIRE_DB_TESTS=1` expects panic/fail)

## Also flag (related)

- Editing golden expected numbers / `.snap` content to match a buggy new result **without** Golden delta + human rationale (`economic_regression`)
- Critical-path code change (see `scripts/ci/critical-area-test-gate.sh`) with **no** accompanying test change and no PR label `no-tests-needed` + reason
- Replacing a named regression test with a compile-only check (`cargo check`, `tsc`) presented as coverage

## Do not flag

- Pure renames / move of a test that keeps the same assertions
- Adding `#[ignore = "manual: ..."]` on a **new** hermetic-impossible repair/RPC job that was never a green CI test
- Legitimate golden updates that include a **Golden delta:** section explaining product intent (still surface money Δ for human review; do not block solely on presence of the section)

## Comment shape (preferred)

```text
assertion_weakened: <path> — <what got weaker>
Allowed fix: restore assertion strength, or get explicit GO + Golden delta for intentional economic change.
```
