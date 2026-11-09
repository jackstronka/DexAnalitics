#!/usr/bin/env bash
# R1: validate ci/test-areas.toml + hermetic unit tests for the classifier.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
python3 tools/test_areas.py --check-map
(
  cd tools
  python3 test_test_areas.py
)
echo "test-areas: ok"
