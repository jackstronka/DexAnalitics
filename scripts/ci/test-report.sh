#!/usr/bin/env bash
# R2: build per-area test report from one or more JUnit XML files.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

OUT_DIR="${TEST_REPORT_OUT:-target/test-report}"
mkdir -p "$OUT_DIR"

args=()
for f in "$@"; do
  if [[ -f "$f" ]]; then
    args+=(--junit "$f")
  else
    echo "test-report: skip missing $f" >&2
  fi
done

if [[ ${#args[@]} -eq 0 ]]; then
  echo "test-report: no junit files provided" >&2
  exit 1
fi

# Always exit 0 for CI summary aggregation even when tests failed earlier;
# the rust/web jobs already gate on test status.
set +e
python3 tools/test_report.py "${args[@]}" --out-dir "$OUT_DIR"
set -e

if [[ -n "${GITHUB_STEP_SUMMARY:-}" && -f "$OUT_DIR/report.md" ]]; then
  {
    echo "## Test report (R2)"
    echo
    cat "$OUT_DIR/report.md"
  } >> "$GITHUB_STEP_SUMMARY"
fi

echo "test-report: wrote $OUT_DIR/report.md and report.json"
