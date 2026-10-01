# Wallet GL — Faza C: plan kont + spójne delty

**Status:** C1–C4 wdrożone (2026-05-26); C5 docs close-out  
**Data:** 2026-05-26  
**Norma nadrzędna:** [`WALLET_GL.md`](WALLET_GL.md) §3 Faza C  
**Po:** Faza B v1 [`WALLET_GL_PHASE_B_PLAN.md`](WALLET_GL_PHASE_B_PLAN.md) ✅  
**Powiązane:** [`WALLET_SESSION_GL_INTEGRATION_ANALYSIS.md`](WALLET_SESSION_GL_INTEGRATION_ANALYSIS.md), [`IMPLEMENTATION_PLAN_CHAIN_SESSION_PORTFOLIO.md`](IMPLEMENTATION_PLAN_CHAIN_SESSION_PORTFOLIO.md), [`doc/BUGS.md`](BUGS.md) BUG-20260526-01

---

## 1. Cel Fazy C

1. **Jawny plan kont** (co już częściowo jest w PG) + konwencja znaku.  
2. **Spójne delty** per `kind` — albo komplet w journal `confirmed`, albo **`decode_status`** (lifecycle-first, brak milczenia).  
3. **SESSION / CHAIN GL** pozostaje projekcją z **lifecycle (PSLR)**; journal uzupełnia audyt API i transfer/convert.

**Nie jest celem C v1:** saldo UI z GL zamiast RPC (Faza D), globalny reconcile (Faza E), konta `LP:{pda}` w posting.

---

## 2. Stan wyjściowy (co już mamy)

| Asset | Stan | Faza |
| ----- | ---- | ---- |
| `wallet_gl_token_account` / `wallet_gl_curated_pool` | migracja 009 | seed planu kont |
| `wallet_gl_account` SESSION | migracja 011 | ✅ posting z lifecycle |
| `wallet_gl_account` CHAIN | migracja 013 | ✅ posting z lifecycle |
| `session_mint_deltas_from_lifecycle_json` | `wallet_session.rs` | close/collect/open/swap |
| Journal `collect_fees` | delty z pre/post UI | ✅ |
| Journal `open` / `swap_before_open` | delty częściowe | częściowo |
| Journal `close` / `decrease` / `rebalance` / tx submit | **puste delty** | luka C |
| `journal_principal_deferred_to_lifecycle` | open/close skip SESSION z journal | świadoma decyzja |
| Phantom USDC (BUG-20260526-01) | open debit bez swap credit w GL | G5 w C2 |

---

## 3. Plan kont (docelowy v1 — bez LP posting)

| `account_type` | `account_code` | Posting v1 | Źródło Δ |
| -------------- | -------------- | ---------- | -------- |
| `system` | `SPL:{mint}` | seed 009 | — |
| `system` | `TX_FEE` | **C4** | lifecycle `tx_fee_lamports` (opcjonalnie journal native) |
| `session` | `SESSION:{uuid}` | ✅ | lifecycle → PSLR |
| `chain` | `CHAIN:{uuid}` | ✅ | lifecycle → PSLR |
| `wallet` | `WALLET:{owner}` | poza v1 | transfer/convert (journal only) |
| `lp_position` | `LP:{pda}` | poza v1 | — |

**Konwencja znaku (SESSION/CHAIN):** `+` = wpływ na portfel logiczny (close principal, collect, swap in), `−` = open deposit / swap out.

---

## 4. Macierz delt — journal vs lifecycle

| `kind` | Journal `confirmed` (C) | SESSION/CHAIN posting | Priorytet |
| ------ | ------------------------ | --------------------- | --------- |
| `swap_before_open` | ✅ już delty | lifecycle swap row | — |
| `open_position` | caps z request (B); **decode_status=lifecycle** gdy defer | lifecycle open | C1 |
| `close_position` | **mirror z op/lifecycle** na confirmed | lifecycle close | **C1** |
| `collect_fees` | ✅ | lifecycle collect | — |
| `decrease_liquidity` | delty z op RPC pre/post lub `decode_status=deferred` | brak v1 | C3 |
| `rebalance_position` | `decode_status=deferred` + link do lifecycle ids | lifecycle (wiele wierszy) | C3 |
| `increase_liquidity` | tx submit: `decode_status=pending_decode` v1 | brak v1 | C3 |
| `transfer_sol` / `convert_sol` | ✅ | nie SESSION | — |

---

## 5. Fazy implementacji (małe PR)

### C1 — Journal close + `decode_status` (P0)

**Cel:** `confirmed` close nie jest „pusty” bez wyjaśnienia; audyt w `/wallet/ledger`.

| Task | Pliki |
| ---- | ----- |
| Pole opcjonalne `decode_status` na `WalletLedgerEvent` (+ PG kolumna jeśli potrzeba) | `models.rs`, migracja `014_*`, `wallet_ledger.rs` |
| Wartości: `exact`, `lifecycle_mirror`, `deferred_lifecycle`, `empty` | dokumentacja |
| Po sukcesie `close_position`: wypełnij `deltas[]` z `OperationResult` / lifecycle close amounts (best-effort) **lub** `decode_status=deferred_lifecycle` + `cost_session_id` | `position_close_ops.rs` |
| Test: confirmed close ma delty **albo** `decode_status` ≠ brak | `wallet_gl_posting` / `position_close_ops` tests |
| **Nie** zmieniać `journal_principal_deferred_to_lifecycle` (unikamy double SESSION) | — |

**Done:** tail journal dla ręcznego close ma czytelny status; BUG audit łatwiejszy.

---

### C2 — G5: swap credit przed open debit (P0 produkt)

**Cel:** CHAIN/SESSION nie pokazuje phantom −USDC gdy open finansowany swap-mix.

| Task | Pliki |
| ---- | ----- |
| W `session_mint_deltas` / chain scope: przy open sprawdź saldo sesji; jeśli brak credit — **nie** debetuj ponad `min(open, saldo+swap_in_chain)` **lub** wymagaj poprzedniego wiersza swap w tym samym `chain_session_id` | `wallet_session.rs` |
| Alternatywa v1 (mniejsza): flag `metrics_trusted=false` + posting open tylko do wysokości pre-open CHAIN balance | `chain_portfolio.rs` |
| Test regresji: sekwencja swap(+USDC) → open(−USDC) daje net ≥ 0 | `wallet_session` tests |
| Backfill operatora po merge | runbook |

**Done:** po backfill CHAIN USDC ≥ 0 dla sesji `1f33b923…` **albo** jawny swap row; BUG-20260526-01 → `fixed`.

**Zależność:** backfill CHAIN ids + GL na łańcuchu testowym.

---

### C3 — decrease / rebalance / tx submit (P1) ✅

| Task | Pliki |
| ---- | ----- |
| `decrease_liquidity`: `decode_status=deferred_lifecycle` (brak op delt v1) | `handlers/positions.rs` |
| `rebalance_position`: `decode_status=deferred_lifecycle` + `correlation_id` | `handlers/positions.rs` |
| `tx/submit-signed`: `decode_status=pending_decode` domyślnie (bez symulacji decode v1) | `wallet_ledger_tx.rs` |

---

### C4 — Konto `TX_FEE` (P2) ✅

| Task | Pliki |
| ---- | ----- |
| Migracja: `account_type=system`, `TX_FEE` | `015_*` |
| Posting z lifecycle `tx_fee_lamports` per signature (WSOL raw) | `wallet_gl_posting.rs`, `wallet_session.rs` |
| Env off: `CLMM_WALLET_GL_TX_FEE_POSTING=0` | — |

---

### C5 — Walidacja + docs (P1) ✅

| Task | Pliki |
| ---- | ----- |
| Przy append `confirmed`: warn jeśli `deltas.is_empty()` && brak `decode_status` | `wallet_ledger.rs` |
| Checkboxy Faza C w `WALLET_GL.md` | docs |
| Wpis `ENGINEERING_NOTES` per PR | docs |

---

## 6. Kolejność PR (rekomendacja)

```text
C1 decode_status + close journal mirror
    │
    ▼
C2 G5 swap/open ordering (CHAIN phantom USDC)  ← największa wartość operatora
    │
    ├── C3 decrease/rebalance decode_status
    └── C4 TX_FEE (opcjonalnie)
         │
         ▼
    C5 walidacja + docs close-out
```

**Równolegle (operator, bez kodu):** backfill CHAIN na `9vhKYH…` przed C2 verify.

---

## 7. Kryteria GO per slice

| Slice | GO gdy |
| ----- | ------ |
| **C1** | Akceptacja: journal close = audyt; SESSION nadal tylko lifecycle |
| **C2** | Akceptacja reguły open ≤ saldo CHAIN (+ swap w cyklu); test na 1f33b923 / backfill |
| **C3** | Akceptacja `decode_status` zamiast pustych confirmed |
| **C4** | Akceptacja osobnego konta fee vs tylko lineage totals |

---

## 8. Testy (minimal)

```bash
cargo test -p clmm-lp-data wallet_session
cargo test -p clmm-lp-api wallet_gl_posting
cargo test -p clmm-lp-api wallet_ledger
# po C2:
cargo test -p clmm-lp-data chain_mint_deltas
```

---

## 9. Świadomie poza Fazą C v1

- Konta `LP:{position}` i pełny double-entry wallet↔LP  
- Decode każdej tx z `/tx/submit-signed` (symulacja → delty)  
- Bot rebalance → osobne delty w journal (lifecycle wystarcza)  
- UI sald z GL (Faza D)

---

**Następny krok po akceptacji planu:** operator **`GO C1`** lub **`GO C2`** (jeśli priorytet = phantom USDC przed audytem close w journal).

**keywords:** wallet_gl, Faza C, decode_status, chart of accounts, close_position deltas, G5 swap credit, TX_FEE, phantom USDC, lifecycle_mirror
