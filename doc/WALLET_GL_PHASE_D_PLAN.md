# Wallet GL — Faza D: read model sald z GL (shadow → produkt)

**Status:** Faza D kod ✅ (D1–D5); D0 operator opcjonalny; Faza E następna  
**Data:** 2026-05-26  
**Norma nadrzędna:** [`WALLET_GL.md`](WALLET_GL.md) §3 Faza D, [`FUNCTIONAL_SPECIFICATION.md`](FUNCTIONAL_SPECIFICATION.md) §5 / §5.2  
**Po:** Faza C v1 [`WALLET_GL_PHASE_C_PLAN.md`](WALLET_GL_PHASE_C_PLAN.md) ✅  
**Powiązane:** [`WALLET_SESSION_GL_IMPLEMENTATION_PLAN.md`](WALLET_SESSION_GL_IMPLEMENTATION_PLAN.md) (fazy 2a–4 shadow SESSION), [`IMPLEMENTATION_PLAN_CHAIN_SESSION_PORTFOLIO.md`](IMPLEMENTATION_PLAN_CHAIN_SESSION_PORTFOLIO.md), [`CHAIN_SESSION_PORTFOLIO.md`](CHAIN_SESSION_PORTFOLIO.md), [`doc/BUGS.md`](BUGS.md) BUG-20260526-01

---

## 0. Weryfikacja stanu kodu (2026-05-26)

Plan D nakłada się na wcześniejsze fazy (**SESSION GL 2a–4**, **CHAIN CSP F2–F5**, **UI-1**). Poniżej: co **już jest**, co jest **częściowe**, co **naprawdę brakuje**.

### Już wdrożone (nie duplikować w PR)

| Obszar | Dowód w kodzie | Plan D (slice) |
| ------ | -------------- | -------------- |
| Read SESSION z PG + fallback PSLR | `read_session_balances_resolved`, `GET /wallets/session-balances` | shadow v0 ✅ |
| Read CHAIN z PG + fallback PSLR | `read_chain_balances_resolved`, `GET /wallets/chain-portfolio` | shadow v0 ✅ |
| Posting inkrementalny lifecycle → GL | `ingest_lifecycle_rows` → SESSION/CHAIN/TX_FEE | D cel „inkrementalna aktualizacja” ✅ (lifecycle, nie journal) |
| Backfill SESSION / CHAIN | `POST …/session-balances/backfill`, `…/chain-portfolio/backfill` | D0 narzędzia ✅ |
| Backfill `chain_session_id` | `POST …/chain-portfolio/backfill-chain-ids` + UI | D0 ✅ |
| Reconcile SESSION GL↔PSLR | `POST /reconcile-session-gl`, `gl_matches_pslr`, UI `ReconcilePanel` | **część D1** ✅ |
| `metrics_trusted` + banner UI | `WalletSessionMetrics`, `SessionBalancesPanel`, `ChainPortfolioPanel` | jakość metryk ✅ |
| Źródło odczytu (`source`) | `gl_session_shadow`, `gl_chain_shadow_pslr_corrected`, … + UI `parseSource` | **proto-D1** (string zamiast enum) |
| Hero KPI strategii | `ChainStrategyHero` + `chain-portfolio` metrics | F3 / UI-1 ✅ |
| Reconcile CHAIN vs **lineage** (nie GL↔PSLR) | `WalletChainLineageReconcile` w `chain-portfolio` | audyt PnL ✅, **≠ D1** |
| Executor caps CHAIN/SESSION | `CLMM_REOPEN_USE_CHAIN_PORTFOLIO`, `session_capital.rs` | F4 ✅ (flaga domyślnie off) |
| Preflight open vs sesja | `SessionCapitalPreflight` | P4 ✅ |
| Journal transfer/convert | `transfer_sol`, `convert_sol` w wallet journal | Faza B ✅ |
| TX_FEE posting | migracja 015, `apply_tx_fee_posting_*` | Faza C4 ✅ |
| Test integracyjny SESSION GL | `crates/data/tests/session_gl_integration.rs` | **część D5** ✅ |
| Saldo portfela UI | `GET /effective-balances` (RPC + cache) | norma §5 ✅ — **celowo nie GL** |

### Częściowe (D1 = doprecyzowanie, nie greenfield)

| Luka | Stan dziś | Co dodać w D1 |
| ---- | --------- | ------------- |
| Jakość read model | `source: String` (7+ wartości) | opcjonalnie `quality` enum + `needs_reconcile: bool` na **GET** session/chain |
| `gl_matches_pslr` | tylko na `POST reconcile-session-gl` | dodać na `GET session-balances` / opcjonalnie chain |
| Reconcile CHAIN GL↔PSLR | brak endpointu (jest tylko lineage reconcile) | `POST /reconcile-chain-gl` **lub** rozszerzenie chain-portfolio |
| D1 UI | banner źródła + reconcile SESSION | ujednolicić z nowymi polami; chain bez reconcile GL↔PSLR |

### Naprawdę brakuje (właściwa Faza D do zrobienia)

| Slice | Brak |
| ----- | ---- |
| **D0** | operator: backfill na żywym łańcuchu `9vhKYH…` (kod jest) |
| **D2** | konto `WALLET:{owner}`, opening import, posting journal→WALLET (migracja 016) |
| **D3** | porównanie **GL vs RPC** (nie GL vs PSLR); brak `compare_gl` / `reconcile-wallet-gl` |
| **D4** | `CLMM_WALLET_GL_EFFECTIVE_READ` — effective-balances z GL |
| **D5** | testy CHAIN GL integracyjne; `DATA_CATALOG` dla nowych endpointów D2–D3 |

### Mapowanie planów (żeby nie mylić faz)

| Dokument | Co pokrywa |
| -------- | ---------- |
| [`WALLET_SESSION_GL_IMPLEMENTATION_PLAN.md`](WALLET_SESSION_GL_IMPLEMENTATION_PLAN.md) fazy **2a–4** | SESSION shadow read + backfill + reconcile |
| [`IMPLEMENTATION_PLAN_CHAIN_SESSION_PORTFOLIO.md`](IMPLEMENTATION_PLAN_CHAIN_SESSION_PORTFOLIO.md) **F2–F5** | CHAIN read, UI, historia, executor |
| **Ten dokument (Faza D)** | ujednolicenie jakości, **WALLET global**, GL↔RPC, opcjonalny switch §5 |

**Wniosek:** następny sensowny PR to **D1 (cienkie)** albo **D0 (operator)** — nie budować od zera session/chain read API.

---

## 1. Cel Fazy D

1. **Read model sald z PostgreSQL GL** — materializacja w `wallet_gl_balance` (już częściowo istnieje dla SESSION/CHAIN), z **jawną jakością** odpowiedzi.  
2. **Opening balance** dla konta globalnego `WALLET:{owner}` — GL bez punktu startowego nie zastępuje RPC.  
3. **Inkrementalna aktualizacja** — kontynuacja posting z lifecycle (primary) + journal `confirmed` (transfer/convert, luki).  
4. **Flagi `needs_reconcile` / `quality`** — operator widzi, czy GL jest wiarygodne vs PSLR / RPC.  
5. **Opcjonalny produktowy switch** — `effective-balances` z GL **tylko** po świadomej decyzji (D4); domyślnie RPC (norma §5).

**Nie jest celem D v1:**

- Pełny reconcile GL ↔ on-chain job (→ **Faza E**)  
- Konta `LP:{pda}` w read model  
- Twarda rezerwacja mintów (policy 3A exit — osobna decyzja po D+E)  
- Zastąpienie lineage / stream-pnl źródłem GL

---

## 2. Stan wyjściowy (co już mamy — „pół Fazy D”)

| Asset | Stan | Uwaga |
| ----- | ---- | ----- |
| `GET /wallets/session-balances` | ✅ | `source=gl_session_shadow*` + fallback PSLR |
| `GET /wallets/chain-portfolio` | ✅ | CHAIN + `metrics` + `reconcile` vs lineage |
| Posting SESSION/CHAIN/TX_FEE | ✅ | ingest lifecycle + backfill |
| `POST /reconcile-session-gl` | ✅ | GL vs PSLR + gaps |
| `metrics_trusted` | ✅ | legacy rows / brak mintów puli |
| `GET /wallets/effective-balances` | ✅ **RPC** | norma §5 — **nie** GL |
| Konto `WALLET:{owner}` | ❌ | brak posting / read API |
| Opening balance import | ❌ | — |
| `needs_reconcile` na response | ❌ | tylko implicit przez `source` |
| Porównanie GL vs RPC w API | ❌ | — |

**Implikacja:** hero „Strategia vs start” (CHAIN + LP NAV) może być poprawny **po backfillu danych** (D0), ale **saldo portfela w UI** nadal z RPC do slice D4.

---

## 3. Plan kont — read model (docelowy v1)

| `account_type` | `account_code` | Read API (D) | Posting źródło |
| -------------- | -------------- | ------------ | -------------- |
| `session` | `SESSION:{uuid}` | ✅ `session-balances` | lifecycle (PSLR) |
| `chain` | `CHAIN:{uuid}` | ✅ `chain-portfolio` | lifecycle (PSLR) |
| `system` | `TX_FEE` | opcjonalnie D2b | lifecycle `tx_fee_lamports` |
| `wallet` | `WALLET:{owner}` | **D2** `wallet-balances` | opening snapshot + journal transfer/convert |
| `system` | `SPL:{mint}` | seed 009 (meta) | — |
| `lp_position` | `LP:{pda}` | poza D | — |

**Konwencja:** saldo read model = suma `wallet_gl_balance.amount_raw` per `(account_id, mint)`; posting append-only przez `wallet_gl_posting`.

---

## 4. Macierz źródeł read model

| Scope | Primary posting | Read fallback | Quality gate |
| ----- | --------------- | ------------- | ------------ |
| SESSION | lifecycle ingest | PSLR aggregate | `gl_pslr_match` |
| CHAIN | lifecycle ingest | PSLR aggregate (chain scope) | `gl_pslr_match` + cap open (C2) |
| WALLET | opening + journal | RPC (D3 shadow compare) | `needs_reconcile` vs RPC |
| TX_FEE | lifecycle | sum PSLR `tx_fee_lamports` | opcjonalnie D2b |

---

## 5. Fazy implementacji (małe PR)

### D0 — Operator: dane przed read model (P0, bez kodu)

**Cel:** GL w PG odzwierciedla cykl testowy; unikamy fałszywego `needs_reconcile` i phantom mintów.

| Krok | Akcja |
| ---- | ----- |
| 1 | Restart API (migracje do **015**) |
| 2 | `POST /wallets/chain-portfolio/backfill-chain-ids` — anchor `9vhKYH…` |
| 3 | `POST /wallets/chain-portfolio/backfill?chain_session_id=…` |
| 4 | Opcjonalnie session backfill + `POST /reconcile-session-gl` |
| 5 | UI: hero CHAIN; SESSION tylko „ostatni rebalance” |
| 6 | `CLMM_REOPEN_USE_CHAIN_PORTFOLIO=1` + jeden rebalance E2E |

**Done:** BUG-20260526-01 → `fixed`; CHAIN USDC ≥ 0 lub jawny swap w cyklu.

---

### D1 — Jakość read model SESSION/CHAIN (P0 kod) ✅

**Stan:** `quality`, `needs_reconcile`, `gl_matches_pslr` na GET session/chain; `POST reconcile-chain-gl`; UI banner + reconcile SESSION i CHAIN.

| Task | Stan | Pliki |
| ---- | ---- | ----- |
| `source` (7 wartości session/chain) | ✅ | `wallet_gl_posting.rs`, UI `parseSource` |
| `POST reconcile-session-gl` + `gl_matches_pslr` | ✅ | `wallet_gl_posting.rs`, `SessionBalancesPanel` |
| Pola `quality`, `needs_reconcile` na GET session/chain | ✅ | `models.rs`, handlers |
| `gl_matches_pslr` na GET (bez osobnego POST) | ✅ | handlers |
| `POST /reconcile-chain-gl` (GL vs PSLR) | ✅ | `wallet_gl_posting.rs`, `ChainPortfolioPanel` |
| i18n dla `needs_reconcile` | ✅ | `i18n.tsx` |

**Done:** operator nie myli `pslr_fallback` z „GL działa”; checklist przed D4.

**Nie:** zmiana `effective-balances`.

---

### D2 — Opening balance + `WALLET:{owner}` (P1) ✅

**Cel:** globalny portfel logiczny w GL (transfer/convert + snapshot startowy).

| Task | Stan | Pliki |
| ---- | ----- | ----- |
| Migracja `016_wallet_gl_wallet_account.sql` — wzorzec konta `wallet` | ✅ | `crates/data/migrations/` |
| `wallet_account_code(owner)`, `ensure_wallet_account`, posting z journal `transfer_sol` / `convert_sol` | ✅ | `wallet_session.rs`, `wallet_gl_posting.rs` |
| Jednorazowy posting `kind=opening_import` (idempotent event_id) | ✅ | migracja 016, `wallet_session.rs` |
| `POST /wallets/wallet-balances/opening-import` — snapshot z bieżącego RPC (operator) | ✅ | `handlers/wallets.rs` |
| `GET /wallets/wallet-balances?owner=` — shadow read | ✅ | `handlers/wallets.rs`, `routes.rs` |
| Hook journal confirmed → WALLET (gdy `deltas[]` + owner) | ✅ | `wallet_ledger.rs` → `wallet_gl_posting.rs` |
| Testy unit | ✅ | `wallet_session`, `wallet_gl_posting` |
| UI Wallet page (`WalletGlBalancesPanel`) | ✅ | `web/src/components/WalletGlBalancesPanel.tsx` |

**Done:** saldo WALLET rośnie na transfer/convert; opening import dokumentowany w ENGINEERING_NOTES.

**Ryzyko:** tx poza API (airdrop, zewnętrzny transfer) → GL < RPC do Fazy E.

---

### D3 — Shadow compare: GL vs RPC (P1) ✅

**Cel:** telemetria rozbieżności przed produktowym switch.

| Task | Stan | Pliki |
| ---- | ---- | ----- |
| `GET /wallets/reconcile-wallet-gl?owner=` — raport diff GL vs effective-balances | ✅ | `handlers/wallets.rs`, `wallet_gl_posting.rs` |
| UI panel diagnostyki (Wallet page) | ✅ | `WalletGlBalancesPanel.tsx` |
| Log/metric gdy \|GL−RPC\| > próg na WSOL/USDC | ✅ | `tracing::warn` w `reconcile_wallet_gl_vs_rpc` |
| Testy unit reconcile | ✅ | `wallet_gl_posting` tests |

**Done:** operator widzi lukę przed włączeniem D4.

---

### D4 — Produktowy switch `effective-balances` (P2, flaga) ✅

**Cel:** świadome zastąpienie RPC w UI gdy GL trustworthy.

| Task | Stan | Pliki |
| ---- | ---- | ----- |
| Env `CLMM_WALLET_GL_EFFECTIVE_READ=1` (default **off**) | ✅ | `wallet_gl_posting.rs` |
| Gdy on: preferuj GL gdy `needs_reconcile=false` i opening balance istnieje | ✅ | `apply_wallet_gl_effective_read_overlay`, handler overlay |
| Fallback RPC + `effective_balance_source=rpc_fallback` | ✅ | `models.rs`, handler |
| Banner UI „saldo z księgi” | ✅ | `WalletEffectiveSourceBanner`, Wallet, PositionCreate |
| Aktualizacja **§5** functional spec + runbook rollback | ✅ | `FUNCTIONAL_SPECIFICATION.md`, `WALLET_GL.md` §6 |

**GO produktowe:** operator potwierdza D0 + D1 + D2 opening + D3 diff akceptowalny.

---

### D5 — Docs + testy close-out (P1) ✅

| Task | Stan | Pliki |
| ---- | ---- | ----- |
| Checkboxy Faza D w `WALLET_GL.md` | ✅ | docs |
| Wpisy `ENGINEERING_NOTES` per slice D1–D5 | ✅ | docs |
| Rozszerzenie `session_gl_integration` o CHAIN + WALLET | ✅ | `crates/data/tests/session_gl_integration.rs` |
| `doc/DATA_CATALOG.md` — endpointy D1–D4 | ✅ | docs |

---

## 6. Kolejność PR (rekomendacja)

```text
D0 operator backfill (9vhKYH…)     ← blokuje sensowne testy
    │
    ▼
D1 quality + needs_reconcile       ← najmniejsze ryzyko, największa czytelność
    │
    ├── D2 WALLET + opening balance
    │        │
    │        ▼
    │   D3 GL vs RPC compare
    │        │
    │        ▼
    │   D4 effective-balances switch (flag)
    │
    └── D5 docs/tests (po każdym slice lub na końcu)
```

**Równolegle:** domknięcie BUG-20260526-01 po D0 (nie wymaga D1 kodu).

---

## 7. Kryteria GO per slice

| Slice | GO gdy |
| ----- | ------ |
| **D0** | CHAIN backfill OK; hero strategii ≈ intuicja; BUG → `fixed` |
| **D1** | UI pokazuje `quality` / `needs_reconcile`; brak regresji session/chain read |
| **D2** | opening import + transfer journal → WALLET balance; test DB |
| **D3** | raport diff GL↔RPC na curated mintach; bez zmiany domyślnego UI |
| **D4** | operator akceptuje flagę; rollback przetestowany |
| **D5** | checklist WALLET_GL + testy integracyjne green |

---

## 8. Env (propozycja)

| Zmienna | Domyślnie | Znaczenie |
| ------- | --------- | --------- |
| `CLMM_WALLET_GL_SESSION_READ` | on | istniejące |
| `CLMM_WALLET_GL_CHAIN_READ` | on | istniejące |
| `CLMM_WALLET_GL_WALLET_READ` | on (D2+) | read `WALLET:{owner}` |
| `CLMM_WALLET_GL_WALLET_POSTING` | on (D2+) | journal → WALLET |
| `CLMM_WALLET_GL_EFFECTIVE_READ` | **off** (D4) | effective-balances z GL |
| `CLMM_REOPEN_USE_CHAIN_PORTFOLIO` | off | executor caps (F4; nie Faza D) |

---

## 9. Testy (minimal)

```bash
cargo test -p clmm-lp-data wallet_session
cargo test -p clmm-lp-api wallet_gl_posting
LOGLEVEL=WARN cargo test -p clmm-lp-data --test session_gl_integration   # DATABASE_URL
# po D3:
cargo test -p clmm-lp-api effective_balances
# web:
cd web && npx tsc --noEmit
```

---

## 10. Świadomie poza Fazą D v1

- Automatyczny opening balance z archival RPC bez operatora  
- Korekty księgowe (`kind=gl_adjustment`) — Faza E  
- Saldo UI **domyślnie** z GL bez flagi D4  
- Pełne pokrycie tx off-API (deposits, CEX)  
- Read model LP locked w puli (`LP:*`)

---

## 11. Relacja z Fazą E

Faza D daje **read model i jakość**; Faza E daje **job/endpoint reconcile** GL ↔ on-chain z raportem i opcjonalną korektą. D3 jest **podglądem** diff; E jest **procesem** utrzymania.

---

**Następny krok po akceptacji planu:** operator **D0 (backfill)** lub **`GO D1`** (kod jakości read model).

**keywords:** wallet_gl, Faza D, read model, needs_reconcile, GlReadQuality, WALLET owner, opening balance, effective-balances, shadow, gl_session_shadow, chain-portfolio, FUNCTIONAL_SPEC §5
