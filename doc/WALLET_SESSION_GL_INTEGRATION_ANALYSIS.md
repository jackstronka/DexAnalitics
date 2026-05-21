# Wallet SESSION GL — analiza integracji i spięcie z istniejącymi źródłami

**Status:** accepted (architektura docelowa)  
**Date:** 2026-05-20  
**Powiązane:** [`WALLET_SESSION_GL_IMPLEMENTATION_PLAN.md`](WALLET_SESSION_GL_IMPLEMENTATION_PLAN.md), [`WALLET_GL.md` §2.2](WALLET_GL.md), [`DATA_CATALOG.md`](DATA_CATALOG.md), [`FUNCTIONAL_SPECIFICATION.md` §6.1](FUNCTIONAL_SPECIFICATION.md)

**keywords:** SESSION account, lifecycle, position_stream_ledger_rows, wallet_gl_posting, rebalance_session_id, cost_session_id, integration, analytics retention, policy-3A

---

## 1. Cel dokumentu

Opisuje **jak spiąć** nowe konta logiczne `SESSION:{session_id}` z danymi, które **już zbieramy** (lifecycle, lineage, wallet journal), bez drugiego rejestru transakcji i bez zamykania kont sesji po manual close (analityka historyczna).

---

## 2. Trzy warstwy danych (stan repo)

```text
┌─────────────────────────────────────────────────────────────────┐
│ ZAPIS (append-only)                                              │
│  Executor / CLI / API close → orca_position_lifecycle.jsonl      │
│  API positions (swap/open/close/collect) → wallet-ledger JSONL │
└────────────────────────────┬────────────────────────────────────┘
                             │
         ┌───────────────────┴───────────────────┐
         ▼                                       ▼
┌─────────────────────────┐           ┌──────────────────────────┐
│ position_stream_        │           │ wallet_gl_journal_event  │
│ ledger_rows (PG)        │           │ + wallet_gl_* (SESSION)  │
│ ingest co ~10s          │           │ hook z wallet journal    │
└───────────┬─────────────┘           └────────────┬─────────────┘
            │                                      │
            ▼                                      ▼
   lineage, stream-pnl,              GET /wallets/session-balances
   chain-history, bot-activity        (shadow read model)
```

| Warstwa | Ścieżka / tabela | Co zbiera | Id sesji |
| ------- | ---------------- | --------- | -------- |
| **Lifecycle (źródło prawdy tx)** | `data/ledger/orca_position_lifecycle.jsonl` → `position_stream_ledger_rows` | `signature`, `event`, `fee_payer_token_deltas`, `lp_collected_*`, `details.close_amount_*`, opłaty tx | `rebalance_session_id` |
| **Wallet journal (audyt API)** | `data/wallet-ledger-events.jsonl` → `wallet_gl_journal_event` | `kind`, `cost_session_id`, `deltas[]`, `correlation_id`, pending/confirmed/failed | `cost_session_id` |
| **SESSION GL (projekcja)** | `wallet_gl_account` / `wallet_gl_balance` / `wallet_gl_posting` | Saldo per mint na `SESSION:{uuid}` | ten sam UUID |

### Kod źródłowy (grep-friendly)

| Obszar | Pliki |
| ------ | ----- |
| Zapis lifecycle | `crates/protocols/src/ledger/tx_lifecycle.rs` |
| Ingest JSONL → PG | `crates/api/src/services/position_stream_performance.rs` (`ingest_lifecycle_rows_best_effort`) |
| Agregacja analityczna | `crates/api/src/services/position_stream_lineage.rs`, `position_stream_pnl.rs`, `position_chain_history.rs` |
| Budżet reopen `T` | `crates/execution/src/strategy/rebalance.rs` (`returned_*_raw`, `close_amounts_from_lifecycle_*`) |
| Wallet journal + posting v1 | `crates/api/src/services/wallet_ledger.rs`, `wallet_gl_posting.rs` |
| Ręczny close API | `crates/api/src/handlers/positions.rs` → `execute_full_close_only` (collect + close → wiersze lifecycle) |

---

## 3. Kluczowy wniosek

**Nie trzeba zbierać transakcji od zera.** Pełna historia LP / swap / close jest w **lifecycle**; lineage i executor już z niej korzystają (§6.1).

**SESSION GL** powinno być **materialized view** (projekcja) z `position_stream_ledger_rows`, a nie równoległym dziennikiem opartym głównie o wallet journal.

**Luka wdrożenia v1:** posting SESSION jest podpięty pod **wallet journal** (`deltas[]`). Przy ręcznym `close_position` journal często ma **puste delty**, podczas gdy lifecycle po `execute_full_close_only` ma `bot_collect_fees` + `bot_close_position` z `close_amount_*` i `lp_collected_*` — **o ile** przekazano `cost_session_id` / `rebalance_session_id`.

---

## 4. Docelowy podział ról

| Rola | Komponent | Uwagi |
| ---- | --------- | ----- |
| Źródło prawdy zdarzeń on-chain | `position_stream_ledger_rows` + JSONL | Już działa |
| Analityka łańcucha / IL / fee USD | lineage, chain-history, stream-pnl | **Bez przenoszenia** logiki do GL |
| Konto logiczne SESSION | `wallet_gl_balance` | Szybki bilans tokenów per cykl; **nigdy** auto-close po close pozycji |
| Audyt operacji API | wallet journal | Korelacja API; transfer/convert; uzupełnienie gdy brak wiersza lifecycle |
| Portfel UI (3A) | `effective-balances` | Bez zmian — wspólny portfel on-chain |

### Retention (decyzja operatora)

Po zamknięciu pozycji (ręcznym lub botowym) konta `SESSION:{session_id}` **nie są zamykane ani likwidowane** — salda zostają w PG do analizy. Nowy cykl życia = **nowy** UUID sesji, nie zerowanie starego konta.

---

## 5. Jedno pole sesji (klej)

```text
cost_session_id (API / wallet journal)
    ≡ rebalance_session_id (lifecycle / executor / pending-open)
    ≡ session_id (GL: SESSION:{uuid})
```

| Miejsce | Pole |
| ------- | ---- |
| API swap+open, close, collect | `cost_session_id` (query) |
| Executor `execute_full_close_only` | `ledger_session_id` → `rebalance_session_id` w JSONL |
| Bot / shell | `CLMM_REBALANCE_SESSION_ID` |

Bez `session_id` wiersze lifecycle istnieją, ale **nie da się** przypisać ruchu do konta SESSION.

---

## 6. Reguła liczenia Δ na SESSION (jedna funkcja, wielu konsumentów)

Wspólna funkcja koncepcyjna (do implementacji w `wallet_gl_posting.rs`):

**`session_mint_deltas_from_ledger_row(row) -> Vec<(mint, delta_raw_i128)>`**

### Priorytety per `event`

| `event` (lifecycle) | Δ na SESSION |
| ------------------- | ------------ |
| `bot_close_position`, `position_close` | `+close_amount_a_raw`, `+close_amount_b_raw` na mintach A/B puli (`pool_pubkey`); plus `+lp_collected_token_*_raw` gdy obecne |
| `bot_collect_fees` | `+lp_collected_token_a_raw`, `+lp_collected_token_b_raw` |
| `cli_swap`, `bot_swap_*` | z `fee_payer_token_deltas` (mint → UI → raw), minty puli/sesji |
| `bot_open_*`, `position_open` | depozyt: ujemne delty z `fee_payer_token_deltas` lub `open_amount_*` w `details` |
| `transfer_sol`, `convert_sol` | **pomijamy** w SESSION v1 (konto WALLET global) |

### Konwencja a rebalance / §6.1

- **Wariant A (norma bota):** close = principal + fee na wierszu close; `bot_collect_fees` osobno — suma wierszy sesji = pełny obieg portfela w cyklu (zgodne z `returned_*_raw` w executorze).
- **Wariant B (reconcile):** suma dodatnich `fee_payer_token_deltas` per mint dla wszystkich wierszy z `rebalance_session_id` — metryka „inflow do portfela” do porównania z A.

### Idempotencja

- Klucz posting: `event_id = lifecycle:{signature}` (`signature` UNIQUE w `position_stream_ledger_rows`).
- Re-ingest JSONL nie dublowuje salda.

---

## 7. Pipeline integracji (rekomendowany)

```text
Executor/CLI → lifecycle JSONL
       ↓
ingest_lifecycle_rows_best_effort (throttle ~10s)
       ↓
position_stream_ledger_rows (UPSERT BY signature)
       ↓
apply_session_postings_from_ledger_row (NOWY hook)
       ↓
wallet_gl_balance na SESSION:{id}  (konto nigdy nie zamykane)
```

**Wallet journal** — drugorzędnie:

- Posting z journala tylko gdy: brak posting dla tego `signature` **lub** `kind` bez odpowiednika w lifecycle (`transfer_sol`, `convert_sol`).
- Dla `close_position`: **nie** duplikować RPC diff w API — poczekać na wiersz lifecycle.

### Odczyt `GET /wallets/session-balances`

1. **Fast path:** `wallet_gl_balance`
2. **Fallback:** przelicz z `position_stream_ledger_rows WHERE rebalance_session_id = $1` (ta sama funkcja co posting)
3. Pola odpowiedzi: `source`, opcjonalnie `row_count`, `completeness`

---

## 8. Mapowanie konsumentów (bez duplikacji)

| Konsument | Źródło dziś | Po spięciu |
| --------- | ----------- | ---------- |
| Reopen, `T`, pending-open | lifecycle §6.1 w `rebalance.rs` | **Plan:** SESSION caps + `W_session` — [`WALLET_SESSION_CAPITAL_EXECUTOR_PLAN.md`](WALLET_SESSION_CAPITAL_EXECUTOR_PLAN.md) |
| Lineage, Fees zebrane, IL | `position_stream_ledger_rows` | **Bez zmian** |
| SESSION GL | wallet journal (słabe przy close) | **PSLR + ingest hook** |
| UI portfel (global) | RPC `effective-balances` | **Bez zmian** (3A); open z `cost_session_id` → opcjonalny preflight SESSION (plan P4) |
| Bot activity timeline | JSONL lifecycle | **Bez zmian** |

Executor **nie musi** znać GL — wystarczy nadal append lifecycle z `rebalance_session_id`.

---

## 9. Ryzyka i mitigacje

| Ryzyko | Mitigacja |
| ------ | --------- |
| Opóźnienie ingest ~10s | Akceptowalne dla analityki; opcjonalny trigger po materialize chain-history |
| Brak `session_id` w operacji | UI: wymuszony UUID na cykl; brak posting SESSION |
| Podwójne fee (collect + close) | Udokumentowana konwencja A; reconcile (faza 3) |
| Podwójne księgowanie journal + lifecycle | Jedno źródło Δ: **lifecycle pierwsze**; journal = audit / luki |
| Re-ingest całego pliku | Idempotencja po `signature` |
| 3A contention | SESSION = shadow; nie zastępuje `W` |

---

## 10. Czego nie robić

- Nie budować drugiego dekodowania tx w `close_position` handlerze (RPC diff).
- Nie zastępować `effective-balances` saldem SESSION.
- Nie mergować tabel lineage do `wallet_gl_*`.
- Nie zamykać / zerować `SESSION:*` po close ani po successful open (historia analityczna).

---

## 11. Powiązanie z wdrożonym kodem (2026-05-20)

| Element | Stan |
| ------- | ---- |
| Migracja `011_wallet_gl_session_accounts.sql` | ✅ |
| Posting z wallet journal (`wallet_gl_posting.rs` + hook) | ✅ (v1) |
| `GET /wallets/session-balances` | ✅ |
| Posting z lifecycle przy ingest | ✅ `apply_session_postings_from_lifecycle_json` |
| Fallback odczytu z PSLR | ✅ `read_session_balances_resolved` |
| Backfill historyczny | ❌ opcjonalny |

**keywords:** integration analysis, lifecycle-first posting, position_stream_ledger_rows, session-balances, analytics retention
