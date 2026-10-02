#!/usr/bin/env bash
# R3: human-readable golden delta vs BASE_REF (default: origin/main).
# Prints Markdown, appends to GITHUB_STEP_SUMMARY, optionally comments on the PR.
# Does not fail the job when numbers change (D1 will require a Golden delta section).
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
    exit 0
  fi
  body="$(printf '%s\n\n%s\n' '<!-- golden-delta -->' "${report}")"
  if command -v gh >/dev/null 2>&1; then
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
