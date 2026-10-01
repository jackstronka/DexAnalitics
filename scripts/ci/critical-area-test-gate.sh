#!/usr/bin/env bash
# Critical-area test gate (doc/IMPLEMENTATION_PLAN_REGRESSION_RESILIENCE.md A8).
# A PR that changes a money / lineage / execution file must also change tests: a test file, or added
# test lines (#[test], assert*!, proptest!, insta::) in the Rust diff. A file merely *containing*
# `mod tests` does not count. Escape hatch for pure refactors: PR label `no-tests-needed`
# (SKIP_LABEL=1), reason belongs in the PR description.
set -euo pipefail

if [[ -z "${BASE_REF:-}" ]]; then
  echo "BASE_REF is required (example: main)."
  exit 2
fi

git fetch --no-tags --depth=1 origin "${BASE_REF}" >/dev/null 2>&1 || true
range="origin/${BASE_REF}...HEAD"
changed_files="$(git diff --name-only "${range}")"

if [[ -z "${changed_files}" ]]; then
  echo "No changed files detected."
  exit 0
fi

critical_files=()
has_test_file=0

while IFS= read -r file; do
  [[ -z "${file}" ]] && continue
  case "${file}" in
    crates/api/src/services/position_stream_lineage.rs|\
    crates/api/src/services/position_stream_pnl.rs|\
    crates/api/src/services/position_chain_history.rs|\
    crates/api/src/services/chain_economic_totals.rs|\
    crates/api/src/services/chain_portfolio.rs|\
    crates/api/src/services/wallet_gl_posting.rs|\
    crates/api/src/services/wallet_ledger*.rs|\
    crates/api/src/services/position_service.rs|\
    crates/api/src/services/position_executor.rs|\
    crates/api/src/handlers/positions.rs|\
    crates/data/src/wallet_session.rs|\
    crates/data/migrations/*|\
    crates/execution/src/strategy/rebalance.rs|\
    crates/execution/src/strategy/session_capital.rs|\
    web/src/pages/PositionCreate.tsx|\
    web/src/pages/PositionDetail.tsx)
      critical_files+=("${file}")
      ;;
  esac

  case "${file}" in
    */tests/*|\
    *_tests.rs|\
    */tests.rs|\
    *.test.ts|\
    *.test.tsx|\
    *.spec.ts|\
    *.spec.tsx|\
    *.snap)
      has_test_file=1
      ;;
  esac
done <<< "${changed_files}"

if [[ "${#critical_files[@]}" -eq 0 ]]; then
  echo "Critical area test gate passed (no critical files changed)."
  exit 0
fi

echo "Critical files changed:"
printf '  %s\n' "${critical_files[@]}"

added_test_lines=0
if [[ "${has_test_file}" -eq 0 ]]; then
  added_test_lines="$(git diff -U0 "${range}" -- '*.rs' \
    | grep -v '^+++' \
    | grep -E '^\+.*(#\[(tokio::)?test|assert(_eq|_ne|_matches)?!|proptest!|insta::assert)' \
    | wc -l | tr -d ' ')" || added_test_lines=0
fi

if [[ "${has_test_file}" -eq 1 || "${added_test_lines}" -gt 0 ]]; then
  echo "Critical area test gate passed (test file changed or ${added_test_lines} added test line(s))."
  exit 0
fi

if [[ "${SKIP_LABEL:-0}" == "1" ]]; then
  echo "Critical area test gate SKIPPED via PR label 'no-tests-needed' (justify in PR description)."
  exit 0
fi

echo "Critical area changed without accompanying tests."
echo "Add a regression/invariant test (test file or new #[test]/assert in the diff),"
echo "or for a pure refactor add PR label 'no-tests-needed' with the reason in the PR description."
exit 1
