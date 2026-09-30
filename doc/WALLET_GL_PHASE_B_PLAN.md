# Wallet GL — Faza B: kompletność journalu API

**Status:** wdrożenie v1 (2026-05-26)  
**Norma nadrzędna:** [`WALLET_GL.md`](WALLET_GL.md) §3  
**Powiązane:** [`WALLET_SESSION_GL_INTEGRATION_ANALYSIS.md`](WALLET_SESSION_GL_INTEGRATION_ANALYSIS.md) (lifecycle = źródło Δ; journal = audyt API)

---

## 1. Cel Fazy B

Każda operacja portfela/pozycji **inicjowana przez API** ma wpis w `wallet-ledger-events.jsonl` (+ dual-write Postgres) w schemacie **`pending` → `confirmed` / `failed`** z `correlation_id`.

**Nie jest celem Fazy B:** pełne delty tokenów dla każdego `kind` (to Faza C / lifecycle posting). Close/open principal idzie przez lifecycle → SESSION/CHAIN GL.

---

## 2. Tabela pokrycia (endpoint → journal)

| Endpoint | `kind` | Journal | Delty `confirmed` | Uwagi |
| -------- | ------ | ------- | ----------------- | ----- |
| `POST /positions` | `open_position` | ✅ | częściowo (pre-open caps) | `handlers/positions.rs` |
| `POST /positions/swap-before-open` | `swap_before_open` | ✅ | ✅ | |
| `DELETE /positions/{addr}` | `close_position` | ✅ | ❌ (lifecycle) | `position_close_ops.rs` |
| `POST /positions/{addr}/collect` | `collect_fees` | ✅ | ✅ | pre/post uncollected |
| `POST /positions/{addr}/decrease` | `decrease_liquidity` | ✅ | ❌ | audit do decode (C) |
| `POST /positions/{addr}/rebalance` | `rebalance_position` | ✅ | ❌ | bot lifecycle osobno |
| `POST /positions/close-all` | `close_position` | ✅ | ❌ | via `execute_manual_close_*`, source `api:positions:close-all` |
| `POST /wallets/transfer` | `transfer_sol` | ✅ | ✅ SOL | |
| `POST /wallets/convert-sol` | `convert_sol` | ✅ | ✅ SOL/WSOL | |
| `POST /tx/{op}/build` + `POST /tx/submit-signed` | mapowanie `op` → `kind` | ✅ v1 | ❌ | metadata z build + submit body |
| Bot executor (rebalance) | — | ⏸ poza B | lifecycle JSONL | świadomie bez journal API |

### Mapowanie `/tx/*/build` → `kind`

| Build path | `ledger_kind` |
| ---------- | ------------- |
| `/tx/open/build` | `open_position` |
| `/tx/increase/build` | `increase_liquidity` |
| `/tx/decrease/build` | `decrease_liquidity` |
| `/tx/collect/build` | `collect_fees` |
| `/tx/close/build` | `close_position` |

Submit: klient przekazuje `correlation_id`, `ledger_kind`, `wallet_pubkey`, opcjonalnie `pool_address`, `position_address`, `cost_session_id` (z build + kontekstu formularza).

---

## 3. Priorytety (wdrożone w tym PR)

| Priorytet | Task | Status |
| --------- | ---- | ------ |
| **P0** | Journal na `POST /tx/submit-signed` | ✅ |
| **P0** | `ledger_kind` w `BuildUnsignedTxResponse` | ✅ |
| **P0** | Testy regresji `known_ledger_kinds` + walidacja audit | ✅ |
| **P1** | close-all: jawny `source` w journalu | ✅ |
| **P2** | Bot executor → journal | ⏸ lifecycle wystarcza |
| **P2** | Delty close/decrease/rebalance z decode | Faza C |

---

## 4. Kryteria „done” Fazy B v1

- [x] Tabela pokrycia w tym dokumencie + link z `WALLET_GL.md`
- [x] `POST /tx/submit-signed` z audytem pending→confirmed/failed
- [x] `increase_liquidity` objęte przez ścieżkę tx (build + submit)
- [x] Testy jednostkowe: lista `kind`, walidacja audit submit
- [ ] E2E devnet per endpoint (operator; opcjonalnie później)

---

## 5. Env

Bez nowych flag. Journal respektuje `dry_run` (brak append) jak pozostałe handlery.

**keywords:** wallet_gl, wallet_ledger, Faza B, tx/submit-signed, increase_liquidity, journal coverage, correlation_id
