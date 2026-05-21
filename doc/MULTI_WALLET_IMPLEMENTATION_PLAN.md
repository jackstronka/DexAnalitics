# Multi-wallet Implementation Plan (WIP)

Status: in progress  
Date: 2026-04-27

## Goal

Deliver secure multi-wallet management in API + Web with:

- redundant wallet storage (primary + secondary directories),
- active signer selection for transaction paths,
- native SOL transfer between wallets,
- clear operator diagnostics (replication health, signer source, transfer result).

## Scope agreed in session

1. Dual-directory wallet storage (`CLMM_WALLETS_DIR_PRIMARY`, `CLMM_WALLETS_DIR_SECONDARY`) with fallback to `CLMM_WALLETS_DIR`.
2. Wallet management endpoints:
   - `POST /wallets/create`
   - `GET /wallets/active-signer`
   - `POST /wallets/active-signer`
   - `POST /wallets/transfer`
3. Wallet list diagnostics:
   - `replication_status` (`healthy`, `degraded`, `conflict`)
   - presence flags per store
   - file fingerprint
4. Wallet UI support for:
   - storage visibility
   - create wallet
   - active signer switch
   - SOL transfer

## What is already implemented

### Backend

- Config and state:
  - Added `wallets_dir_primary` and `wallets_dir_secondary` in API config.
  - Added `active_signer_wallet_id` in app state.
- Wallet handlers:
  - `/wallets` now aggregates primary + secondary directories.
  - Added create, active-signer get/set, and transfer endpoints.
  - Added replication metadata in wallet entries.
- Signer routing:
  - Position executor now prefers active signer (when set), then env fallback.
- API schema/routing:
  - OpenAPI and route registrations updated.
- Tests:
  - Added endpoint reachability checks for new wallet endpoints.

### Frontend

- API client expanded with new wallet contracts.
- Wallet page updated with:
  - storage info and replication badges,
  - create wallet form,
  - active signer controls,
  - SOL transfer form and result display.

## Validation done

- `cargo check -p clmm-lp-api` passed.
- `npx tsc --noEmit` in `web/` passed.
- No linter errors in touched files.

## Next steps (when work resumes)

### Priority 1

1. Add `POST /wallets/reconcile`:
   - detect missing side,
   - copy wallet file from healthy side to missing side,
   - return per-wallet repair summary.
2. Add stronger transfer guardrails:
   - max lamports per transfer (env-configurable),
   - optional recipient allowlist,
   - richer audit event payload.

### Priority 2

3. Harden active signer behavior:
   - clear endpoint for reverting to env fallback,
   - explicit source metadata in signer-dependent responses.
4. Improve conflict handling:
   - optional strict mode blocking signer use when `conflict`.

### Priority 3

5. UX/i18n polish on new Wallet controls (labels, hints, error copy).
6. Add focused tests:
   - dual-store conflict/degraded scenarios,
   - reconcile flow,
   - transfer validation matrix.

### Priority 4 — bulk close all (positions UI)

Cross-feature: [`POSITIONS_CLOSE_ALL_IMPLEMENTATION_PLAN.md`](POSITIONS_CLOSE_ALL_IMPLEMENTATION_PLAN.md).

1. **`resolve_close_signer_for_position`:** map `owner_pubkey` (on-chain + registry) → `wallet_id` from `/wallets` stores.
2. **Batch worker:** per-wallet groups; load keypair from file **without** mutating global `active_signer` for the whole API process.
3. **Skip policy:** positions whose owner is not in API wallet storage → `skipped_unmanaged_signer` (Phantom / external keypair).
4. **Optional:** parallel worker tasks per wallet group (after P1 sequential MVP).

## Operational notes

- Two directories improve availability but do not replace host hardening.
- Recommended baseline:
  - NTFS ACL restrictions on wallet folders,
  - encrypted volume (BitLocker),
  - no secret key material in logs,
  - periodic restore/reconcile drill.

## Suggested commit split (for later)

1. `feat(api): add multi-store wallet management and active signer endpoints`
2. `feat(web): add wallet create/signer/transfer controls`
3. `feat(api): add wallet reconcile endpoint and transfer guardrails`

