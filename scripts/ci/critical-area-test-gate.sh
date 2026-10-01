#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${BASE_REF:-}" ]]; then
  echo "BASE_REF is required (example: main)."
  exit 2
fi

git fetch --no-tags --depth=1 origin "${BASE_REF}" >/dev/null 2>&1 || true
changed_files="$(git diff --name-only "origin/${BASE_REF}...HEAD")"

if [[ -z "${changed_files}" ]]; then
  echo "No changed files detected."
  exit 0
fi

is_critical_change=0
has_test_change=0

while IFS= read -r file; do
  [[ -z "${file}" ]] && continue
  case "${file}" in
    crates/api/src/services/position_stream_lineage.rs|\
    crates/api/src/services/position_service.rs|\
    crates/api/src/services/position_executor.rs|\
    crates/api/src/handlers/positions.rs|\
    crates/execution/src/strategy/rebalance.rs|\
    web/src/pages/PositionCreate.tsx|\
    web/src/pages/PositionDetail.tsx)
      is_critical_change=1
      ;;
  esac

  case "${file}" in
    */tests/*|\
    *test*.rs|\
    *test*.ts|\
    *test*.tsx|\
    *spec*.ts|\
    *spec*.tsx)
      has_test_change=1
      ;;
  esac
done <<< "${changed_files}"

if [[ "${has_test_change}" -eq 0 ]]; then
  while IFS= read -r rust_file; do
    [[ -z "${rust_file}" ]] && continue
    if rg -n "^[[:space:]]*#\[test\]|^[[:space:]]*mod tests" "${rust_file}" >/dev/null 2>&1; then
      has_test_change=1
      break
    fi
  done <<< "$(printf '%s\n' "${changed_files}" | rg "\.rs$" || true)"
fi

if [[ "${is_critical_change}" -eq 1 && "${has_test_change}" -eq 0 ]]; then
  echo "Critical area changed without accompanying tests."
  echo "Add regression/invariant tests before merge."
  exit 1
fi

echo "Critical area test gate passed."
