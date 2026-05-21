# Multi-wallet Manual Runbook

Purpose: operate multi-wallet safely without extra automation/features.

## 1) Configure storage paths

Set in `.env` on API host:

- `CLMM_WALLETS_DIR_PRIMARY`
- `CLMM_WALLETS_DIR_SECONDARY`

Keep both folders on different disks/locations when possible.

## 2) Configure transfer policy (recommended)

Optional guardrails:

- `CLMM_WALLET_TRANSFER_MIN_LAMPORTS` (dust guard; default 1_000_000 = 0.001 SOL)
- `CLMM_WALLET_TRANSFER_MAX_LAMPORTS` (optional max per tx)
- `CLMM_WALLET_TRANSFER_SOURCE_ALLOWLIST` (CSV wallet ids)
- `CLMM_WALLET_TRANSFER_ALLOWLIST` (CSV recipient pubkeys)

Notes:

- Recipient allowlist always includes pubkeys from wallet files in stores.
- Source allowlist is strict only when env is set; empty env means any local wallet id is allowed.

## 3) Manual operating flow

1. Open Wallet page and verify wallet replication status:
   - `healthy` -> OK
   - `degraded` -> run `POST /wallets/reconcile`
   - `conflict` -> manual fix required (do not auto-merge)
2. Create wallet if needed (`POST /wallets/create`).
3. Set active signer (`POST /wallets/active-signer`) for tx operations.
4. Execute transfer (`POST /wallets/transfer`) with validated source/destination.
5. Re-check balances and signer state.

## 4) Conflict handling (manual)

When status is `conflict`:

1. Compare both wallet files out-of-band.
2. Decide trusted source (primary or secondary).
3. Copy trusted file to the other store.
4. Re-run `POST /wallets/reconcile`.

Do not transfer from conflicted wallet ids until resolved.

## 6) Bulk close all positions (planned)

When using **Close all** on the Positions screen (see [`POSITIONS_CLOSE_ALL_IMPLEMENTATION_PLAN.md`](POSITIONS_CLOSE_ALL_IMPLEMENTATION_PLAN.md)):

1. Positions opened under **different wallet keypairs** are closed using **each position's on-chain owner**, not necessarily the current active signer.
2. Only wallets present in `CLMM_WALLETS_DIR_*` can be used for server-side close; others appear as **skipped** in the batch summary.
3. Before a large batch, verify replication is **healthy** and you have enough **native SOL per signer** for tx fees.
4. Prefer pausing linked strategies for the batch (default in spec) to avoid rebalance during mass close.

## 5) Security hygiene

- Never commit wallet JSON files to git.
- Restrict folder ACLs (only operator/API user).
- Keep storage encrypted at rest (e.g. BitLocker).
- Never log private key material.

