# Wallet SESSION accounts in GL — decision & implementation plan

**Status:** accepted (faza 0–5a + P4 ✅; P5 docs ✅; E2E devnet z flagą — operator; **5b** deferred)  
**Date:** 2026-05-20  
**Powiązane:** [`WALLET_SESSION_GL_INTEGRATION_ANALYSIS.md`](WALLET_SESSION_GL_INTEGRATION_ANALYSIS.md) (analiza spięcia), [`WALLET_GL.md` §2.2](WALLET_GL.md#22-konto-logiczne-per-cykl-życia-pozycji-rebalance_session_id--norma-docelowa), [`FUNCTIONAL_SPECIFICATION.md` §5.2](FUNCTIONAL_SPECIFICATION.md), [`DATA_CATALOG.md`](DATA_CATALOG.md), [`WALLET_SESSION_CAPITAL_EXECUTOR_PLAN.md`](WALLET_SESSION_CAPITAL_EXECUTOR_PLAN.md) (produktyzacja SESSION w executor / reopen)

---

## 1. Decyzja produktowa (krok 3)

### Wybrana koncepcja: **Hybrid A — sub-ledger GL `SESSION:{session_id}`**

| Odrzucone (na tę fazę) | Powód |
| ---------------------- | ----- |
| Osobny keypair / adres per cykl | Operacyjnie ciężkie; to multi-wallet, nie cykl życia |
| Vault on-chain per strategia | Greenfield; poza obecnym Orca executor |
| Tylko replay lifecycle (bez GL) | Już mamy — nie daje salda „na wyciągnięcie ręki” bez materializacji |
| Twarda rezerwacja SPL (koniec 3A) | Osobna decyzja **po** shadow read model + reconcile |
| Drugi rejestr tx (RPC diff w API close) | Lifecycle już zbiera close/collect/swap |

### Warianty wewnątrz GL (rozstrzygnięte)

| Wariant | Opis | Wybór |
| ------- | ---- | ----- |
| **C1** | Jedno konto `SESSION:{uuid}` + wymiar `mint` w saldzie | **✅ Domyślny** |
| **C2** | `SESSION:{uuid}:SPL:{mint}` | Równoważne; nie w v1 |
| **C3** | Tylko `WALLET:{owner}` | Za mało |
| **C4** | SESSION + LP w jednym PR | LP **poza** v1 posting |

**Ustalenie v1:** tylko **`SESSION:{session_id}`** (bez kont `LP:*` w posting). Pełna analiza architektury: [`WALLET_SESSION_GL_INTEGRATION_ANALYSIS.md`](WALLET_SESSION_GL_INTEGRATION_ANALYSIS.md).

### Relacja z policy 3A

- SESSION = **shadow / analityka**; executor: `T` z lifecycle §6.1 + `W` z `effective-balances`.
- Bez blokady mintów do fazy 5 (osobna decyzja produktowa).

---

## 2. Stan wyjściowy i po fazie 0–2 (2026-05-20)

| Warstwa | Było | Jest (kod) | Docelowo (faza 1b+) |
| ------- | ---- | ---------- | ------------------- |
| Journal API | `wallet_gl_journal_event` | ✅ dual-write JSONL + PG | audyt API; nie główne Δ przy close |
| Lifecycle | JSONL + ingest → PSLR | ✅ (istniejące) | **źródło posting SESSION** |
| SESSION schema | — | ✅ migracja `011_*` | — |
| Posting SESSION | — | ✅ z wallet journal (`deltas[]`) | ✅ z **PSLR** przy ingest |
| Read API | — | ✅ `GET /wallets/session-balances` | + fallback z PSLR |
| Lineage / PnL | ✅ | ✅ bez zmian | konsument, nie duplikat GL |
| UI panel sesji | — | ✅ | faza 2b |

---

## 3. Docelowy model kont (chart of accounts)

| `account_type` | `account_code` | Znaczenie |
| -------------- | -------------- | --------- |
| `wallet` | `WALLET:{owner}` | Agregat portfela (przyszłość) |
| `session` | `SESSION:{session_id}` | Kapitał cyklu — **retention**, bez auto-close |
| `lp_position` | `LP:{position_pubkey}` | Poza v1 posting |
| `system` | `TX_FEE`, … | Przyszłość |

**Mapowanie id:** `cost_session_id` ≡ `rebalance_session_id` ≡ `session_id` w GL.

---

## 4. Reguły posting SESSION (v1)

Źródło Δ: **`position_stream_ledger_rows`** (preferowane); wallet journal tylko dla luk / transfer / convert.

| `event` (lifecycle) | SESSION |
| ------------------- | ------- |
| `bot_close_position`, `position_close` | `+close_amount_*` + `+lp_collected_*` (minty puli) |
| `bot_collect_fees` | `+lp_collected_*` |
| `cli_swap`, `bot_swap_*` | `fee_payer_token_deltas` → raw |
| `bot_open_*`, `position_open` | ujemne depozyty |
| `transfer_sol`, `convert_sol` | — (WALLET, nie SESSION v1) |

**Idempotencja:** `wallet_gl_posting.event_id = lifecycle:{signature}`.

Szczegóły i warianty A/B: [`WALLET_SESSION_GL_INTEGRATION_ANALYSIS.md` §6](WALLET_SESSION_GL_INTEGRATION_ANALYSIS.md#6-reguła-liczenia-δ-na-session-jedna-funkcja-wielu-konsumentów).

---

## 5. Plan implementacji (fazy)

### Faza 0 — Schema + docs ✅

- [x] Migracja `011_wallet_gl_session_accounts.sql`
- [x] `DATA_CATALOG.md`, `WALLET_GL.md` §2.2 retention
- [x] Ten plan + [`WALLET_SESSION_GL_INTEGRATION_ANALYSIS.md`](WALLET_SESSION_GL_INTEGRATION_ANALYSIS.md)

---

### Faza 1a — Posting z wallet journal ✅ (shadow v1)

- [x] `wallet_gl_posting.rs` + hook w `wallet_ledger::append_wallet_ledger_event`
- [x] Unit testy mapowania delt
- [ ] Pełne delty przy `close_position` w journal — **świadomie odroczone** (lifecycle-first)

**Ograniczenie:** ręczny close często ma puste `deltas[]` w journal; SESSION nie rośnie do fazy 1b.

---

### Faza 1b — Posting z lifecycle (priorytet) ✅

**Cel:** SESSION aktualizowane z danych, które **już zbieramy**.

**Kod:**

- [x] `session_mint_deltas_from_lifecycle_json()` w `wallet_gl_posting.rs`
- [x] Hook w `ingest_lifecycle_rows_best_effort` (`position_stream_performance.rs`)
- [x] Minty A/B z `details.token_mint_*` (+ fallback klucze `fee_payer_token_deltas`)
- [x] `event_id = lifecycle:{signature}`; idempotencja po `wallet_gl_posting.event_id`
- [x] Journal posting pomija wiersz gdy lifecycle posting już istnieje dla tej sygnatury

**Testy:**

- [x] unit: `bot_close_position`, `bot_collect_fees`, `lifecycle_posting_event_id`
- [x] integracja DB (`crates/data/tests/session_gl_integration.rs`, `DATABASE_URL`)

**Kryterium done:** ręczny close z `cost_session_id` → po ingest widać minty w `GET /wallets/session-balances`.

---

### Faza 2a — Read API ✅

- [x] `GET /api/v1/wallets/session-balances?session_id=&owner=`
- [x] `source=gl_session_shadow`

---

### Faza 2b — Read fallback + UI ✅ (panel; telemetry — planowane)

- [x] `session-balances`: fallback przelicz z PSLR gdy GL puste (`read_session_balances_resolved`)
- [x] UI: `SessionBalancesPanel` — Wallet ledger (filtr sesji), Position detail (ostatnia sesja), Positions stranded (select sesji)
- [ ] Executor: log `session_notional_usd` vs `T` (telemetria tylko)

---

### Faza 3 — Reconcile SESSION ✅

- [x] `POST /wallets/reconcile-session-gl?session_id=&owner=`
- [x] GL vs PSLR + `last_close_returned` (ostatni close) + `gaps[]`

---

### Faza 4 — Backfill ✅

- [x] `POST /wallets/session-balances/backfill?session_id=&limit=`
- [x] Replay PSLR → SESSION (idempotent)

---

### Faza 5 — Produktyzacja executor (5a ✅, P4 ✅, P5 ✅)

**Plan wdrożenia:** [`WALLET_SESSION_CAPITAL_EXECUTOR_PLAN.md`](WALLET_SESSION_CAPITAL_EXECUTOR_PLAN.md)

- [x] P0: `clmm-lp-data::wallet_session` — wspólny odczyt GL / PSLR / JSONL
- [x] P1–P2: executor — kapsy `min(RPC, SESSION)`, §2.2 na capped notional, flaga `CLMM_REOPEN_USE_SESSION_CAPITAL` (default off)
- [x] P3: API `StrategyExecutor` → `set_session_database` przy starcie strategii
- [x] P4 (opc.): UI `PositionCreate` preflight vs session-balances
- [x] P5: aktualizacja §2.1 / §5.2 functional spec + rollout runbook (`WALLET_GL.md` §6, `ORCA_RUNBOOK.md`, `DATA_CATALOG.md`)

**Poza v1:** twarda rezerwacja on-chain (**5b**)

---

## 6. Kolejność PR (zaktualizowana po analizie)

```text
PR0  ✅ 011 schema + docs + journal posting v1 + session-balances
PR-A     lifecycle posting w ingest_lifecycle_rows + testy
PR-B     session-balances fallback z PSLR
PR-C     backfill historyczny (opcjonalny)
PR-D     UI panel + executor telemetry
PR-E     reconcile-session endpoint
```

Każdy PR: mały; **nie** zmienia §5 / 3A bez flagi.

---

## 7. Flag i env

| Env | Domyślnie | Znaczenie |
| --- | --------- | --------- |
| `CLMM_WALLET_GL_SESSION_POSTING` | on | Wyłącz: `0`/`false` |
| `CLMM_WALLET_GL_SESSION_READ` | on | Wyłącz GET session-balances |
| `CLMM_WALLET_GL_SESSION_LIFECYCLE_POSTING` | on (plan) | Wyłącz posting z PSLR ingest |
| `CLMM_WALLET_GL_SESSION_RECONCILE` | off | Włącz reconcile endpoint |

---

## 8. Ryzyka i mitigacje

| Ryzyko | Mitigacja |
| ------ | --------- |
| Journal vs lifecycle double post | Lifecycle pierwsze; journal tylko bez `signature` / transfer |
| Ingest delay | OK dla analityki; opcjonalny eager trigger |
| collect + close fee overlap | Konwencja A + reconcile |
| Brak session_id | UI wymusza UUID na cykl |

Pełna lista: [`WALLET_SESSION_GL_INTEGRATION_ANALYSIS.md` §9](WALLET_SESSION_GL_INTEGRATION_ANALYSIS.md#9-ryzyka-i-mitigacje).

---

## 9. Decyzje zamknięte (operator 2026-05-20)

1. **Źródło prawdy tx:** lifecycle → `position_stream_ledger_rows` (nie duplikować zbierania).
2. **SESSION GL:** projekcja / analityka; **nigdy** auto-close ani likwidacja po close pozycji.
3. **Bez zerowania SESSION** po successful open — nowy cykl = nowy UUID.
4. **LP konta:** poza v1 posting.
5. **Posting close:** z lifecycle (`close_amount_*`, `lp_collected_*`), nie głównie z pustego wallet journal.

---

## 10. Changelog

| Data | Zmiana |
| ---- | ------ |
| 2026-05-20 | Utworzenie planu; Hybrid A / C1 |
| 2026-05-20 | Faza 0–2a: schema 011, journal posting, session-balances API |
| 2026-05-20 | Analiza integracji → `WALLET_SESSION_GL_INTEGRATION_ANALYSIS.md`; plan PR-A..E lifecycle-first |
| 2026-05-20 | PR-A/B: lifecycle posting on ingest, PSLR fallback read, unit tests |
