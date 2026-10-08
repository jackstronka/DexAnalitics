#!/usr/bin/env bash
# C5: high/critical BUGS.md Guards/tests names must exist in the repo (or explicit manual:).
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
cd "${root}"
python3 "${root}/tools/bugs_test_guard.py" --root "${root}" --bugs "${root}/doc/BUGS.md"
