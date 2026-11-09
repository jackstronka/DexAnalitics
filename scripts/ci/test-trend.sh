#!/usr/bin/env bash
# R4: hermetic unit tests for weekly trend aggregator.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT/tools"
python3 test_test_trend.py
echo "test-trend: unit ok"
