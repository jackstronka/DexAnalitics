#!/usr/bin/env bash
# R3 + D1: human-readable golden delta vs BASE_REF (default: origin/main).
# Prints Markdown, appends to GITHUB_STEP_SUMMARY, optionally comments on the PR.
# Number diffs do not fail the job. Missing "Golden delta:" in the PR body does
# when gated paths changed (*.snap, tests/fixtures, snapshots/, openapi.json).
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
cd "${root}"

base="${BASE_REF:-origin/main}"
if [[ "${base}" != origin/* && "${base}" != */* ]]; then
  base="origin/${base}"
fi

git fetch --no-tags --depth=1 origin "${base#origin/}" >/dev/null 2>&1 || true

if ! report="$(python3 "${root}/tools/golden_delta.py" --git-base "${base}")"; then
  echo "golden-delta: generator failed" >&2
  exit 1
fi

printf '%s\n' "${report}"

if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
  printf '%s\n' "${report}" >> "${GITHUB_STEP_SUMMARY}"
fi

if [[ -n "${GITHUB_TOKEN:-}" && -n "${PR_NUMBER:-}" ]]; then
  export GH_TOKEN="${GITHUB_TOKEN}"
  if printf '%s\n' "${report}" | grep -q 'files changed vs'; then
    echo "golden-delta: no snap changes; skip PR comment"
  elif command -v gh >/dev/null 2>&1; then
    body="$(printf '%s\n\n%s\n' '<!-- golden-delta -->' "${report}")"
    existing="$(gh api "repos/${GITHUB_REPOSITORY}/issues/${PR_NUMBER}/comments" \
      --jq '.[] | select(.body | contains("<!-- golden-delta -->")) | .id' \
      | head -n 1 || true)"
    if [[ -n "${existing}" ]]; then
      gh api -X PATCH "repos/${GITHUB_REPOSITORY}/issues/comments/${existing}" \
        -f body="${body}" >/dev/null
      echo "golden-delta: updated PR comment ${existing}"
    else
      gh pr comment "${PR_NUMBER}" --body "${body}"
      echo "golden-delta: posted PR comment"
    fi
  fi
fi

# D1: require a Golden delta section when gated files changed.
body_file=""
cleanup_body=0
if [[ -n "${PR_BODY_FILE:-}" ]]; then
  body_file="${PR_BODY_FILE}"
elif [[ -n "${PR_BODY:-}" ]]; then
  body_file="$(mktemp)"
  cleanup_body=1
  printf '%s\n' "${PR_BODY}" > "${body_file}"
elif [[ "${GITHUB_ACTIONS:-}" == "true" && -n "${GITHUB_TOKEN:-}" && -n "${PR_NUMBER:-}" ]] \
    && command -v gh >/dev/null 2>&1; then
  body_file="$(mktemp)"
  cleanup_body=1
  gh api "repos/${GITHUB_REPOSITORY}/pulls/${PR_NUMBER}" --jq '.body // empty' \
    > "${body_file}"
fi

if [[ -n "${body_file}" ]]; then
  set +e
  python3 "${root}/tools/golden_delta.py" --require-pr-section \
    --git-base "${base}" --pr-body-file "${body_file}"
  rc=$?
  set -e
  if [[ "${cleanup_body}" -eq 1 ]]; then
    rm -f "${body_file}"
  fi
  exit "${rc}"
fi

if [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
  echo "golden-delta: CI D1 check needs PR body (PR_BODY, PR_BODY_FILE, or gh)" >&2
  exit 1
fi

echo "golden-delta: skip D1 section check (no PR body)"
