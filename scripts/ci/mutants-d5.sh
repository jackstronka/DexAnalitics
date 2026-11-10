#!/usr/bin/env bash
# D5: one-shot cargo-mutants on economic aggregators / reopen sizing.
# Requires: cargo install --locked cargo-mutants
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
# Git Bash / WSL often lack cargo on PATH when launched from Windows shells.
if ! command -v cargo >/dev/null 2>&1; then
  if [[ -x "${HOME}/.cargo/bin/cargo" ]]; then
    export PATH="${HOME}/.cargo/bin:${PATH}"
  elif [[ -x "/c/Users/${USERNAME:-}/.cargo/bin/cargo.exe" ]]; then
    export PATH="/c/Users/${USERNAME}/.cargo/bin:${PATH}"
  fi
fi
OUT="${MUTANTS_OUT:-target/mutants-d5}"
mkdir -p "$OUT"

EXTRA_FLAGS=()
JOBS_ARGS=()
# Windows without Developer Mode cannot create symlinks (os error 1314).
# --in-place cannot be combined with --jobs.
if [[ "${OSTYPE:-}" == msys* || "${OSTYPE:-}" == cygwin* || "${OS:-}" == Windows_NT ]]; then
  if [[ "${MUTANTS_IN_PLACE:-1}" != "0" ]]; then
    echo "note: Windows → cargo mutants --in-place (set MUTANTS_IN_PLACE=0 to use copy+symlink)"
    EXTRA_FLAGS+=(--in-place)
  else
    JOBS_ARGS+=(--jobs "${MUTANTS_JOBS:-2}")
  fi
else
  JOBS_ARGS+=(--jobs "${MUTANTS_JOBS:-2}")
fi

run_one() {
  local name="$1"
  shift
  local dest="$OUT/$name"
  mkdir -p "$dest"
  echo "=== mutants D5: $name ==="
  # cargo-mutants exits 2 when any mutant is missed — continue other scopes;
  # `mutants_d5_summary.py` enforces the <10% survive gate at the end.
  set +e
  cargo mutants \
    --output "$dest" \
    --timeout "${MUTANTS_TIMEOUT:-180}" \
    "${JOBS_ARGS[@]}" \
    "${EXTRA_FLAGS[@]}" \
    "$@"
  local rc=$?
  set -e
  echo "=== done $name → $dest (cargo-mutants exit=$rc) ==="
}

# Aggregators in wallet_session (B2 / C3).
run_one wallet_session \
  -p clmm-lp-data \
  -f wallet_session.rs \
  -F 'aggregate_session_sums|aggregate_chain_sums|cap_open_debits'

# Chain headline / refresh totals (B1 / C3).
run_one chain_economic_totals \
  -p clmm-lp-api \
  -f chain_economic_totals.rs \
  -F 'refresh_lineage_totals|sync_chain_economic_totals|chain_headline_end_nav|lineage_node_end_nav|close_nav_usd_from_raw|reconcile_stream_pnl'

# Session capital portfolio clamps.
run_one session_capital \
  -p clmm-lp-execution \
  -f session_capital.rs \
  -F 'apply_portfolio_caps_to_wallet_raw|clamp_deposit_quote_to_portfolio'

# Reopen sizing targets (B3) — rebalance.rs is large; filter by name.
run_one reopen_sizing \
  -p clmm-lp-execution \
  -f rebalance.rs \
  -F 'target_usd_' \
  --exclude-re 'from_env|max_slippage_bps|profitability_mode|est_tx_cost_lamports'

python3 tools/mutants_d5_summary.py --root "$OUT" --out "$OUT/summary.json" --md "$OUT/summary.md"
echo ""
echo "=== D5 summary ==="
cat "$OUT/summary.md"
