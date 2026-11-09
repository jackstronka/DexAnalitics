#!/usr/bin/env bash
# R5: subsystem protection unit tests + MASTER §2.2 check vs fixture report.
set -euo pipefail
cd "$(dirname "$0")/../.."
python3 tools/test_test_protection.py
python3 tools/test_protection.py \
  --report tools/fixtures/r5_report/report.json \
  --check-master
