# Plan refaktoru — wynik ekonomiczny łańcucha (net PnL)

**Status:** plan (bez implementacji do czasu **GO** operatora).  
**Data:** 2026-05-21  
**keywords:** net_pnl_usd, chain economic result, stream-lineage, chain-history, stream-pnl, lineage totals, chain_headline_end_nav, refresh_lineage_totals, valuation_quality

**Powiązane:** [`POSITION_CHAIN_HISTORY_PLAN.md`](POSITION_CHAIN_HISTORY_PLAN.md), [`DATA_CATALOG.md`](DATA_CATALOG.md), [`BUGS.md`](BUGS.md) (BUG-20260521-05/06), [`DECISION_LAYER.md`](DECISION_LAYER.md) §1a, analiza w rozmowie 2026-05-21.

---

## 1. Cel i zakres

### Cel produktowy

Jeden, audytowalny **wynik ekonomiczny łańcucha** (`totals.net_pnl_*`), spójny między:

- `GET …/stream-lineage`
- `GET …/chain-history` (materializacja + odczyt)
- `GET …/stream-pnl` (ten sam rollup co lineage totals, gdy podany `lineage_chain`)
- `GET …/positions/{address}` (karta „Wyniki” — ten sam net co nagłówek lineage dla anchorów z rotacją)

**Definicja (normatywna):**

```
economic_net_usd =
  end_nav(last_pda_in_chain)
  + Σ realized_cashflow_usd(nodes)
  - baseline_usd(first_pda_in_chain)
  - Σ tx_fees_usd(nodes)
```

**Osobno (bez zmiany semantyki):** IL / HODL / `lp_vs_hodl_with_fees_*` — benchmark, nie zamiennik net PnL.

### Non-goals (ten refaktor)

- Płatne feedy cen / archival RPC.
- Pełna księgowość GL sesji (osobny plan `WALLET_SESSION_GL_*`).
- Zmiana algorytmu rebalance / executor.
- Tryb `settlement_v1` — tylko **wspólny rollup**; różnice strict/loose zostają w warstwie snapshotów, nie w duplikacji wzorów.

---

## 2. Stan obecny (dlaczego refaktor)

| Objaw | Przyczyna techniczna |
| ----- | -------------------- |
| Nagłówek **current $0**, net **≈ −baseline** | `totals_json` sprzed repair; `current_value_usd` bez `chain_headline_end_nav` |
| **chain-history** ≠ **stream-lineage** | Ten sam repair, ale różny moment enrich / stary binary / meta nie przeliczona przy zapisie |
| **GET position** `pnl.net = 0` przy OK `value_usd` | Osobna ścieżka `compute_single_position_detail_pnl` vs lineage totals |
| Trzy kopie logiki | `position_stream_pnl`, `position_stream_lineage` (`refresh_*`, `reconcile_*`, `maybe_compute_*`), `position_chain_history` (wywołuje lineage) |

Istniejące guardy (2026-05-21) są poprawne kierunkowo, ale **rozproszone** — łatwo o regresję przy kolejnej zmianie.

---

## 3. Docelowa architektura

```
┌─────────────────────────────────────────────────────────┐
│  chain_economic_totals (nowy moduł, pure + testy)      │
│  - end_nav_for_node / end_nav_for_chain                 │
│  - rollup_chain_economic_totals(nodes) → TotalsDraft    │
│  - apply_to PositionStreamPnLResponse + node marks     │
└───────────────────────────┬─────────────────────────────┘
                            │
     ┌──────────────────────┼──────────────────────┐
     ▼                      ▼                      ▼
stream-lineage      chain-history read/write   stream-pnl
(compute + read)    (materialize + load)       (gdy lineage_chain)
     │                      │
     └──────────► GET position detail (reuse rollup dla chain anchor)
```

### Nowy moduł (propozycja ścieżki)

`crates/api/src/services/chain_economic_totals.rs`

**Typy:**

- `ChainEndNavSource` — `LiveCurrent | MaterializedEnd | CloseAmountExact | CloseEstimate | Missing`
- `ChainTotalsDraft` — pola liczone + `end_nav_source`, `baseline_source`, `totals_quality` (`exact` | `mixed` | `estimated` | `degraded`)

**API modułu (Rust, pub(crate)):**

```rust
pub fn lineage_node_end_nav_usd(n: &PositionStreamLineageNode) -> (Decimal, ChainEndNavSource);
pub fn chain_headline_end_nav_usd(nodes: &[PositionStreamLineageNode]) -> (Decimal, ChainEndNavSource);
pub fn rollup_chain_economic_totals(
    entry: &str,
    nodes: &[PositionStreamLineageNode],
    existing: Option<&PositionStreamPnLResponse>,
) -> ChainTotalsDraft;
pub fn apply_chain_totals_draft(
    draft: &ChainTotalsDraft,
    totals: &mut PositionStreamPnLResponse,
    nodes: &mut [PositionStreamLineageNode],
);
```

**Zasada:** `position_stream_lineage.rs` i `position_chain_history.rs` **nie** liczą net PnL inline — tylko wołają `apply_chain_totals_draft` po zebraniu węzłów.

### Hierarchia end NAV (jedna tabela w kodzie + testy)

| Kolejność | Warunek | Źródło |
| --------- | ------- | ------ |
| 1 | `current_value_usd > 0` | live / ostatni mark |
| 2 | `chain_history_end_value_usd` > 0 | PG materialized |
| 3 | lifecycle `close_amount_*_raw` × cena evencie | **nowe** — przed estymatą |
| 4 | zamknięty, baseline > 0 | estymata: baseline + fees + cashflow − tx |
| 5 | inaczej | Missing → **nie** raportować net ≈ −baseline |

Krok 3 zamyka lukę z diagnozy BFdX9 (close bez `end_close` snapshot, ale jest lifecycle).

---

## 4. Fazy wdrożenia (małe PR)

### Faza A — Ekstrakcja bez zmiany zachowania (1 PR)

**Cel:** przenieść istniejące funkcje do `chain_economic_totals.rs`, zachować sygnatury publiczne przez re-export.

| Zadanie | Pliki |
| ------- | ----- |
| Przenieść `lineage_node_end_nav_usd`, `chain_headline_end_nav_usd`, `rollup` (= dziś `maybe_compute` + końcówka `refresh`) | nowy moduł + `position_stream_lineage.rs` cienki wrapper |
| Przenieść testy `refresh_lineage_totals_*`, `reconcile_*` | `chain_economic_totals.rs` `#[cfg(test)]` |
| `mod chain_economic_totals` w `services/mod.rs` | 1 linia |

**Done when:** `cargo test -p clmm-lp-api chain_economic` zielone; brak diff w JSON odpowiedzi (snapshot test opcjonalny — jeden fixture z BUG-20260521-06).

---

### Faza B — Jedna ścieżka apply na read i write (1–2 PR)

**Cel:** chain-history **zapisuje** już naprawione totals; read nie polega wyłącznie na repair.

| Zadanie | Pliki |
| ------- | ----- |
| Po zbudowaniu `nodes` w `materialize_chain_history_for_anchor` → `apply_chain_totals_draft` **przed** serializacją `totals_json` | `position_chain_history.rs` |
| `load_chain_history_from_db`: jedno wywołanie `apply_chain_totals_draft` zamiast rozproszonego `refresh` + `reconcile` | j.w. |
| `compute_position_stream_lineage`: to samo na końcu | `position_stream_lineage.rs` |
| Deprecate / usuń zduplikowane bloki w `refresh_lineage_totals_from_nodes` (zostaje cienki delegat) | lineage |

**Done when:** dla fixture BFdX9 (2 węzły, ostatni open ~$9.99) **GET chain-history** i **GET stream-lineage** zwracają ten sam `totals.net_pnl_usd` ±0.01; `totals.current_value_usd` > 0.

---

### Faza C — Close NAV z lifecycle (1 PR)

**Cel:** end NAV z `close_amount_*_raw` zanim estymata.

| Zadanie | Pliki |
| ------- | ----- |
| Helper `close_nav_from_lifecycle_db_or_jsonl(position, pool_mints)` | `chain_economic_totals.rs` lub reuse `enrich_*` |
| Podłączyć w `lineage_node_end_nav_usd` jako krok 3 | j.w. |
| Test: węzeł closed, current=0, lifecycle ma close amounts → end > 0, net nie −100% | test modułu |

**Done when:** BUG-20260521-06 zamknięty dla przypadku „close w ledgerze, brak end_close snapshot”.

---

### Faza D — Spójność stream-pnl i GET position (1 PR)

| Zadanie | Pliki |
| ------- | ----- |
| `compute_position_stream_pnl_for_stream_members`: po wyliczeniu baseline/current z DB, **nadpisać** net przez `rollup_chain_economic_totals` gdy `lineage_chain` Some | `position_stream_pnl.rs` |
| `compute_single_position_detail_pnl`: jeśli anchor ma chain w DB → ten sam rollup (nie tylko single-PDA baseline) | j.w. + `handlers/positions.rs` |
| UI: bez zmian kontraktu; opcjonalnie badge `totals_quality` | `PositionStreamPnLResponse` + OpenAPI |

**Done when:** karta „Wyniki” na `PositionDetail` = net z lineage dla PDA w rotacji; brak `pnl.net=0` przy `value_usd>0`.

---

### Faza E — Kontrakt API i UI (1 PR, opcjonalnie rozbić)

| Pole (opcjonalne, breaking-soft) | Znaczenie |
| -------------------------------- | --------- |
| `totals.economic_quality` | `exact` \| `mixed` \| `estimated` \| `degraded` |
| `totals.end_nav_source` | enum string z Fazy C |
| Rozszerzyć `interpretation.economic_net_pnl_caption_pl` o jakość | PL/EN w `i18n` |

**UI (`PositionLineageHistoryPanel`, `ClosedPositionDetail`):**

- Przy `degraded` / `estimated` — krótki banner (jak `SessionBalancesPanel`).
- Nie mieszać IL z net (już jest; utrzymać).

**Done when:** operator widzi, że net jest szacunkiem, zanim zaufze −100%.

---

### Faza F — Backfill operacyjny (ops, nie kod lub 1 skrypt)

| Krok | Komenda / akcja |
| ---- | ---------------- |
| Restart API na **8081** z jednym `DATABASE_URL` | `tools/Start-ClmmApi-8081.ps1` |
| Refresh anchorów z rotacją z ostatnich 30 dni | `POST …/chain-history/refresh` lub CLI |
| Wpis w `BUGS.md` / `ENGINEERING_NOTES` | po merge Faz B–C |

---

## 5. Testy i kryteria akceptacji

### Testy jednostkowe (obowiązkowe)

| ID | Scenariusz |
| -- | ---------- |
| T1 | Zamknięty węzeł, current=0, fees+tx znane → net ∈ (−5%, +5%) baseline, nie −100% |
| T2 | Stale meta: baseline=0 w totals, węzły OK → baseline = first node |
| T3 | Stale meta: current=0, last node live > 0 → current = end_nav |
| T4 | 2 węzły rotacji, cashflow=0 na granicy → chain net ≈ suma leg net (tolerancja 0.05 USD) |
| T5 | `close_amount_*_raw` w lifecycle → end_nav przed estymatą (Faza C) |

### Test integracyjny (jeśli jest DB w CI)

- `load_chain_history_from_db` po materializacji: `totals.current_value_usd` = stream-lineage dla tego samego anchoru.

### Smoke manualny

- PDA `BFdX9AzL…`: stream-lineage, chain-history, position detail — ten sam znak i rząd wielkości net (~+0.03…+0.10 USD przy ~$10 NAV).

---

## 6. Ryzyka i mitigacje

| Ryzyko | Mitigacja |
| ------ | --------- |
| Podwójne liczenie LP fees (w NAV i w polu fees) | Nie dodawać `fees_collected` do net jeśli już w close NAV; flaga źródła w `ChainEndNavSource` |
| Regresja długich łańcuchów (>8 PDA) | Nie zmieniać batch path węzłów; tylko rollup totals na końcu |
| `settlement_v1` vs `live` | Osobne `metrics_mode` w meta; ten sam rollup, inne wejściowe snapshoty |
| Breaking API | Nowe pola opcjonalne; stare pola bez zmiany nazw |

---

## 7. Kolejność merge (rekomendowana)

```
A (ekstrakcja) → B (read/write) → C (lifecycle close NAV) → D (stream-pnl + position) → E (UI quality) → F (backfill)
```

**Szacunek:** 4–6 PR-ów, ~3–5 dni dev + 1 dzień backfill/QA.

**Blokuje:** zaufanie do „Wynik ekonomiczny łańcucha” w UI przed eksperymentami multi-strategy (F2 w `IMPLEMENTATION_PLAN.md`).

---

## 8. Checklist przed „gotowe”

- [ ] Jeden moduł `chain_economic_totals` — jedyne miejsce wzoru net PnL łańcucha.
- [ ] Materializacja zapisuje naprawione `totals_json`.
- [ ] GET chain-history ≡ stream-lineage (totals) dla 3 anchorów z produkcji (lista w BUG lub runbook).
- [ ] Wpis `ENGINEERING_NOTES` + aktualizacja BUG-20260521-06 → `fixed` po QA.
- [ ] OpenAPI / `web/src/lib/api.ts` jeśli dodane pola quality.

---

## 9. Po GO — pierwszy krok implementacji

1. Utworzyć `chain_economic_totals.rs` + przenieść funkcje z Fazy A bez zmiany logiki.
2. Uruchomić `cargo test -p clmm-lp-api refresh_lineage_totals chain_headline`.
3. Dopiero potem Faza B (materialize przed zapisem).
