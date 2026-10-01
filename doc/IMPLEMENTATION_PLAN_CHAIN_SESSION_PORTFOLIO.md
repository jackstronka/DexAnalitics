# Plan implementacji — portfel łańcucha sesji (`chain_session_id`)

**Status:** plan (implementacja po **GO** operatora)  
**Data:** 2026-05-26  
**keywords:** chain_session_portfolio, chain_session_id, CHAIN GL, wallet_session, executor, SessionBalancesPanel, backfill, migration, phantom USDC

**Norma produktowa:** [`CHAIN_SESSION_PORTFOLIO.md`](CHAIN_SESSION_PORTFOLIO.md)

**Powiązane:** [`WALLET_SESSION_GL_IMPLEMENTATION_PLAN.md`](WALLET_SESSION_GL_IMPLEMENTATION_PLAN.md), [`WALLET_SESSION_GL_INTEGRATION_ANALYSIS.md`](WALLET_SESSION_GL_INTEGRATION_ANALYSIS.md), [`WALLET_SESSION_CAPITAL_EXECUTOR_PLAN.md`](WALLET_SESSION_CAPITAL_EXECUTOR_PLAN.md), [`FUNCTIONAL_SPECIFICATION.md`](FUNCTIONAL_SPECIFICATION.md), [`doc/BUGS.md`](BUGS.md) (phantom USDC, BUG-20260521-04)

---

## 1. Streszczenie

| | Dziś (2026-05-26) | Docelowo |
| - | ----------------- | -------- |
| Id sesji | `rebalance_session_id` **nowy co rebalance** (~17 w długim łańcuchu) | **`chain_session_id` stały** na cały cykl + rebalance id jako pod-zdarzenie |
| Konto GL | `SESSION:{rebalance_session_id}` | **`CHAIN:{chain_session_id}`** (+ opcjonalnie SESSION per rebalance do debug) |
| UI „Portfel sesji” | Ostatni rebalance; mylące $0.09 + ujemne USDC | Panel **portfel łańcucha** + NAV LP + start cyklu |
| Open / swap | Często z **globalnego** portfela; GL debetuje pełny open | Open **≤ saldo CHAIN**; swap credit przed open debit |
| Historia | lifecycle + lineage osobno | **Jedna oś** `chain_session_id` + pełny audyt |
| Wynik vs ~$10 start | Lineage net PnL (osobny model) | Portfel łańcucha + reconcile z lineage |

---

## 2. Co już mamy (reuse)

### 2.1 Dane i ingest

| Asset | Lokalizacja | Reuse |
| ----- | ----------- | ----- |
| Lifecycle JSONL | `data/ledger/orca_position_lifecycle.jsonl` | Źródło prawdy zdarzeń; dodać pole `chain_session_id` |
| PSLR | `position_stream_ledger_rows` | Ingest + backfill posting |
| Lineage / chain PDAs | `position_stream_edges`, `stream-lineage` | Wyprowadzenie `chain_session_id` z anchor / strategii |
| Chain-history materializacja | `position_chain_history_*` | Kotwica łańcucha, lista PDA |
| Wallet journal | `wallet_gl_journal_event` | Secondary; nie główne Δ |

### 2.2 Księgowanie SESSION (per rebalance)

| Asset | Lokalizacja | Reuse |
| ----- | ----------- | ----- |
| Reguły Δ | `crates/data/src/wallet_session.rs` → `session_mint_deltas_from_lifecycle_json` | **Ta sama logika** na konto CHAIN |
| Posting | `crates/api/src/services/wallet_gl_posting.rs` | Wzorzec `apply_session_postings_from_lifecycle_row` |
| Read API | `GET /wallets/session-balances` | Wzorzec pod `GET /wallets/chain-portfolio` |
| Reconcile | `POST /wallets/reconcile-session-gl` | Rozszerzyć o CHAIN vs suma rebalance |
| Backfill | `POST /wallets/session-balances/backfill` | Backfill CHAIN z PSLR |
| Migracja GL | `011_wallet_gl_session_accounts.sql` | Nowa migracja `account_type=chain` |
| UI panel | `web/src/components/SessionBalancesPanel.tsx` | Nowy `ChainPortfolioPanel` lub tryb „łańcuch” |
| Metryki open | `compute_session_open_start_from_lifecycle_rows` | **Chain start snapshot** (pierwszy open w `chain_session_id`) |
| Testy mintów | `close_without_details_mints_uses_pool_address_*` | Te same guardy dla CHAIN |

### 2.3 Executor / rebalance

| Asset | Lokalizacja | Reuse |
| ----- | ----------- | ----- |
| `T` / `returned_*_raw` | `FUNCTIONAL_SPECIFICATION.md` §6.1, `rebalance.rs` | Bez zmian semantyki |
| `target_usd_for_reopen_sizing` | `rebalance.rs` | Podpiąć pod saldo CHAIN zamiast tylko prev_end |
| Flag `CLMM_REOPEN_USE_SESSION_CAPITAL` | executor | Nowa flaga lub rozszerzenie: **`CLMM_REOPEN_USE_CHAIN_PORTFOLIO=1`** |
| Pending-open | `pending-open-recovery.json` | Propagacja `chain_session_id` |

### 2.4 Analityka równoległa (bez usuwania)

| Asset | Uwaga |
| ----- | ----- |
| `stream-lineage` / net PnL | Zostaje; reconcile z portfelem łańcucha w fazie 3 |
| `SessionBalancesPanel` na ostatnim rebalance | Może zostać jako „szczegóły rebalance” (zaawansowane) |

---

## 3. Luki (gap analysis)

| # | Luka | Dowód / symptom |
| - | ---- | ---------------- |
| G1 | Brak `chain_session_id` w lifecycle | 17 UUID na jeden łańcuch rotacji |
| G2 | Brak konta GL `CHAIN:*` | Tylko `SESSION:{rebalance}` |
| G3 | Phantom debit USDC przy open | SESSION `1f33b923…`: USDC −2.08, pre_open USDC ~0.003 |
| G4 | UI myli rebalance SESSION z całym cyklem | Operator: $0.09 + $4.62 vs start $10 |
| G5 | Open finansowany z globalnego portfela bez GL credit | `fee_payer_token_deltas` open −2.08 USDC |
| G6 | Brak tx fee posting na konto cyklu | λ tylko w lineage totals |
| G7 | Brak API „historia cyklu” | Trzeba składać ręcznie z ledger + lineage |
| G8 | Executor nie czyta salda CHAIN | `clmm-lp-execution` bez PG |

---

## 4. Fazy implementacji (małe PR)

### Faza 0 — Dokumentacja i kontrakt (✅ ten PR docs)

- [x] [`CHAIN_SESSION_PORTFOLIO.md`](CHAIN_SESSION_PORTFOLIO.md)
- [x] Ten plan
- [ ] Akceptacja operatora (GO)
- [ ] Krótki wskaźnik w [`WALLET_GL.md`](WALLET_GL.md) §2.2 → link do portfela łańcucha

### Faza 1 — Identyfikator i zapis w lifecycle (bez zmiany executora)

**Cel:** każdy nowy wiersz lifecycle ma `chain_session_id`; istniejące łańcuchy — backfill heurystyczny.

| Task | Pliki |
| ---- | ----- |
| Pole `chain_session_id` w lifecycle `details` + PSLR kolumna lub JSON | migracja PG, ingest w `position_stream_performance.rs` |
| Przypisanie przy pierwszym open | `rebalance.rs`, `handlers/positions.rs` (operator open), strategy executor |
| Propagacja przy rotacji | ten sam ID co parent lineage / strategy session |
| Backfill historyczny | bin/CLI: dla anchor PDA → stream-lineage chain → jeden UUID na cały łańcuch |
| Test | rotacja 2 PDA ten sam `chain_session_id` |

**Done:** `grep chain_session_id` w PSLR dla znanej pozycji testowej (np. `9vhKYHA…` anchor).

### Faza 2 — Konto GL `CHAIN:{id}` + posting

**Cel:** saldo portfela łańcucha = suma Δ jak SESSION, ale po `chain_session_id`.

| Task | Pliki |
| ---- | ----- |
| Migracja `wallet_gl_account.account_type = 'chain'` | `crates/data/migrations/0xx_*.sql` |
| `chain_mint_deltas_from_lifecycle_json` lub param `scope` w istniejącej funkcji | `wallet_session.rs` |
| `apply_chain_postings_from_lifecycle_row` | `wallet_gl_posting.rs` |
| Hook ingest (obok SESSION) | `position_stream_performance.rs` |
| `GET /wallets/chain-portfolio?chain_session_id=` | `handlers/wallets.rs`, `models.rs`, OpenAPI |
| `POST …/chain-portfolio/backfill` | jak session backfill |
| Test regresji phantom USDC | po backfill: USDC ≥ 0 dla sesji `1f33b923…` **lub** jawny swap row |

**Done:** API zwraca saldo CHAIN zgodne z ręcznym sumowaniem close−open+collect dla jednej rotacji.

### Faza 3 — Metryki i UI

**Cel:** panel operatora zgodny z §4 normy.

| Task | Pliki |
| ---- | ----- |
| `ChainPortfolioMetrics`: start, current, nav_lp, tx_fees, lp_fees, vs_start | `wallet_session.rs`, API response |
| `ChainPortfolioPanel` na `PositionDetail` | `web/src/pages/PositionDetail.tsx` |
| i18n PL/EN | `web/src/lib/i18n.tsx` |
| Ostrzeżenie gdy ujemny mint w CHAIN | banner (jak `metrics_trusted`) |
| Link do Historia pozycji + lifecycle filtered | istniejące zakładki z filtrem `chain_session_id` |
| Reconcile CHAIN vs lineage net PnL (read-only diff) | endpoint lub sekcja w response |

**Done:** operator widzi start ~$9.95, portfel łańcucha, NAV LP, wynik vs start — **bez** mylenia z ostatnim `rebalance_session_id`.

### Faza 4 — Executor: open/close tylko z portfela łańcucha

**Cel:** koniec cichego dofinansowania z globalnego portfela (przy włączonej fladze).

| Task | Pliki |
| ---- | ----- |
| `resolve_chain_mint_caps(db, chain_session_id)` | nowy moduł w `clmm-lp-data` lub API crate z trait |
| `CLMM_REOPEN_USE_CHAIN_PORTFOLIO=1` | `rebalance.rs`, `StrategyExecutor` |
| Swap-mix: wymóg swap w lifecycle **przed** open debit w GL | executor + posting order |
| Pending-open: `chain_session_id` w item | `pending-open-recovery` |
| Test integracyjny | mock caps: open nie przekracza salda |

**Zależność:** Faza 2 musi być na produkcji / backfill dla aktywnych strategii.

**Done:** rebalance SOL-only close → USDC open nie produkuje ujemnego USDC w CHAIN GL.

### Faza 5 — Zamknięcie cyklu i pełna historia

| Task | Pliki |
| ---- | ----- |
| `chain_session_status`: `active` / `closed` | PG meta table lub pole w registry |
| Ręczny close ustawia `closed_at` | handler close + lifecycle |
| `GET /wallets/chain-portfolio/history` | timeline + export |
| Zamrożony raport końcowy vs start | PDF/CSV opcjonalnie później |

---

## 5. Macierz zależności

```text
F0 docs ──► F1 chain_session_id w lifecycle
                 │
                 ▼
            F2 CHAIN GL posting + API
                 │
         ┌───────┴───────┐
         ▼               ▼
    F3 UI/metrics   F4 executor caps
         │               │
         └───────┬───────┘
                 ▼
            F5 close cyklu + historia
```

**Równolegle (nie blokuje):** lineage net PnL / chain-economic — reconcile w F3.

---

## 6. Backfill istniejących łańcuchów

Dla już trwających strategii (np. 22 PDA od `At6TYQ…`):

1. `stream-lineage` / DB edges → lista PDA + anchor.
2. Wygeneruj **jeden** `chain_session_id` na łańcuch (deterministyczny: hash anchor + first_open_sig **lub** nowy UUID + mapa w tabeli `chain_session_registry`).
3. UPDATE PSLR `raw_json` / kolumna `chain_session_id` dla wszystkich wierszy z `position_pubkey` ∈ chain **lub** `rebalance_session_id` ∈ chain_sessions.
4. `POST /wallets/chain-portfolio/backfill?chain_session_id=…`
5. Weryfikacja: porównaj sumę tokenów CHAIN z oczekiwanym „portfelem między rotacjami” + ostatni NAV LP.

**Ryzyko:** stare wiersze bez `rebalance_session_id` — heurystyka po `position_pubkey` + czas; oznacz `metrics_trusted=false`.

---

## 7. Testy i regresja

| Test | Faza |
| ---- | ---- |
| `session_mint_deltas` → chain scope (unit) | F2 |
| Backfill `1f33b923…`: USDC nie ujemne **albo** documented swap row | F2 |
| API chain-portfolio vs ręczna suma lifecycle | F2 |
| UI snapshot (opcjonalnie) | F3 |
| `open_respects_chain_caps` (execution) | F4 |
| Lineage net PnL vs chain vs_start (tolerancja) | F3 |

Wpisy w [`doc/BUGS.md`](BUGS.md): nowy **BUG-20260526-01** (phantom USDC / open debit) — status `open`, link do tego planu — **przy GO implementacji F2/F4**.

---

## 8. Env / feature flags

| Env | Domyślnie | Opis |
| --- | --------- | ---- |
| `CLMM_WALLET_GL_CHAIN_POSTING` | on | Posting CHAIN z lifecycle |
| `CLMM_WALLET_GL_CHAIN_READ` | on | API read |
| `CLMM_REOPEN_USE_CHAIN_PORTFOLIO` | **off** | Executor używa caps CHAIN |
| `CLMM_CHAIN_SESSION_BACKFILL_ON_READ` | off | Opcjonalnie lazy backfill |

---

## 9. Szacunek PR (kolejność)

| PR | Faza | Opis |
| -- | ---- | ---- |
| PR-CSP-1 | F1 | `chain_session_id` lifecycle + backfill bin |
| PR-CSP-2 | F2 | migracja + posting CHAIN + GET API |
| PR-CSP-3 | F2 | backfill endpoint + reconcile |
| PR-CSP-4 | F3 | UI ChainPortfolioPanel + i18n |
| PR-CSP-5 | F4 | executor caps + fix swap/open ordering |
| PR-CSP-6 | F3/F5 | historia + zamknięcie cyklu |

---

## 10. Kryteria GO per faza

| Faza | GO gdy |
| ---- | ------ |
| F1 | Operator akceptuje pole `chain_session_id` i reguły przypisania (§7 normy) |
| F2 | Akceptacja konta `CHAIN:` i reguł Δ (§5 normy) |
| F3 | Akceptacja makiet metryk (start / portfel / LP / vs start) |
| F4 | Akceptacja zachowania przy braku salda (pending, nie phantom debit) |
| F5 | Akceptacja raportu końcowego |

---

## 11. Świadomie poza zakresem v1

- Osobny keypair on-chain per łańcuch (5b)
- UI salda **całego** portfela operatora z GL zamiast RPC
- Automatyczne dopasowanie lineage cashflow = CHAIN saldo (tylko reconcile + wyjaśnienie różnic)
- Meteora / multi-venue

---

**Następny krok po akceptacji normy:** operator **`GO F1`** → PR-CSP-1 (`chain_session_id` w lifecycle + backfill dla anchor `9vhKYHA…` / test).
