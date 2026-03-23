# TODO: on-chain fees pipeline — co zostało i kolejność prac

**Ostatnia aktualizacja:** 2026-03-22 (T1/T2/C3)  
**Powiązane:** `doc/ONCHAIN_FEES_TRUTH_PLAN.md`, `doc/ONCHAIN_FEES_PROGRESS.md`

## Czy musisz być obecny?

| Rodzaj pracy | Obecność |
|--------------|----------|
| Zmiany w kodzie, testy, dokumentacja w repo | **Nie** — można robić asynchronicznie. |
| Długie `swaps-enrich-curated-all` / masowe `getTransaction` na Twoim PC | **Tak** (albo włączony scheduler/serwer) — to zużywa RPC i czas. |
| Klucze API, Task Scheduler, ścieżki „Start in”, hasła UAC | **Tak** — tylko Ty. |
| Weryfikacja wyników biznesowych (np. sens fee vs rynkek) | **Tak** — okresowo. |

---

## Faza A — dane lokalne (operacyjne, u Ciebie)

- [ ] **A1** Pełna przebudowa decode po fixie:  
  `swaps-enrich-curated-all --refresh-decoded --max-decode <wystarczająco dużo> --decode-timeout-secs 30 --decode-retries 3`
- [ ] **A2** `swaps-decode-audit --save-report` — sprawdzić `% ok` per pool.
- [ ] **A3** Harmonogram: snapshot + swaps sync + enrich (częstotliwość pod RPC).
- [ ] **A4** `data-health-check` w harmonogramie; ewentualnie `--fail-on-alert`.

## Faza B — jakość P1.1 (kod + walidacja)

- [x] **B1** Audyt: globalny histogram `decode_status` + % w raporcie JSON (CLI).
- [x] **B3** Backtest: flaga `--fee-swap-decode-status` (`ok` = tylko czyste swapy, `loose` = poprzednie zachowanie).
- [x] **B2 (Orca)** Anchor event `Traded` z `Program data:` (Borsh): `lp_fee`, `post_sqrt_price`, `input/output_amount`, `a_to_b`; `decode_status` może być `ok_traded_event`.
- [x] **B2 (Raydium)** — Anchor `SwapEvent` z `Program data:` → `decode_status = ok_swap_event`; filtry `ok` / `local_swap_fees` jak Orca.
- [x] **B2 (Meteora)** — DLMM `MeteoraDlmmSwapEvent` (Borsh jak carbon decoder); dyskryminatory `event:SwapEvent` i `event:Swap`; `lb_pair` == pool; `decode_status = ok_swap_event`.
- [ ] **B4** Rate limiting / kolejka RPC w enrich (opcjonalnie).

## Faza C — spójność `backtest` vs `backtest-optimize`

- [x] **C1** (rdzeń) Gdy **brak Dune** lub **pusta** lista swapów po filtrze: `backtest-optimize` buduje opłaty z **`data/swaps`** (decode → timing proxy), jak `backtest`. Wymaga `--snapshot-protocol` + `--snapshot-pool-address` lub `--whirlpool-address` jako adres poola. Flaga **`--fee-swap-decode-status`** (ok/loose) jak w `backtest`.
- [x] **C1b** `backtest-optimize --price-path-source snapshots` (+ opcjonalnie `--start-date` / `--end-date` jak `backtest`) — ta sama ścieżka cenowa co `backtest` (Orca JSONL, cross-pair, bez Birdeye).
- [x] **C2** `run_grid` / `run_single` przyjmują opcjonalny **`local_pool_fees_usd`** (wspólny silnik).
- [x] **C2b** Hybrydowe wypełnianie brakujących bucketów: decoded (gdzie jest) + tx-count proxy (gdzie brak).
- [x] **C3** Test regresyjny: `local_swap_fees::build_local_pool_fees_usd` + fixture Orca `data/swaps/orca/Czfq3.../decoded_swaps.jsonl` (ścieżka `repo_data_dir()` w `#[cfg(test)]`); plus istniejące `snapshot_readiness_regression_test` (tier 2).

## Faza D — P2 / P3 (aktywna płynność)

- [ ] **D1** Snapshoty: prawdziwe konta tick/bin w sąsiedztwie (nie tylko indeksy).
- [ ] **D2** Join swap ↔ snapshot; model `fee * L_pos / L_active`.
- [ ] **D3** Porównanie z baseline w `backtest_fee_compare_*.json`.

## Faza E — P4 (Orca)

- [ ] **E1** `fee_growth_inside` / tick arrays — wg `ONCHAIN_FEES_TRUTH_PLAN.md`.

## Faza E2 — Fee v2 (range / position-aware)

Żeby `threshold` realnie zmieniał `Fees Earned`, backtest musi naliczać fee z perspektywy **pozycji i jej zakresu** (a nie “stałego share na krok”).

- [ ] **E2.1** Backtest fee model: usunąć/ograniczyć uproszczony “step_volume * fee_rate * share” w trybie snapshot fee (lub dodać tryb “position fee truth”).
- [ ] **E2.2 (Orca/CLMM)** Implementacja pozycji (checkpointy) z logiką:
  - `feeGrowthInsideNow` z tick neighborhood + `fee_growth_global_*`,
  - `dFee = feeGrowthInsideNow - feeGrowthInsideLast`,
  - `feesOwed = liquidity_position * dFee / SCALE` (dla A/B),
  - aktualizacja `feeGrowthInsideLast` po każdym “collect” (u Ciebie: po każdym rebalance / otwarciu nowej segmentowej pozycji).
- [ ] **E2.3** Strategia/Rebalance: po `RebalanceAction::Rebalance` resetować checkpointy pozycji (nowy `feeGrowthInsideLast` w kontekście nowego `tickLower/tickUpper`).
- [ ] **E2.4** Test regresyjny: przy tym samym oknie i tym samym range (`lower/upper`) uruchomić sweep `threshold` (np. 2%, 1%, 0.5%, 0.3%) i sprawdzić, że:
  - `Rebalances` rośnie wraz z mniejszym progiem,
  - `Fees Earned` zaczyna się różnić między progami.

- [x] **E2.5** Per-step active liquidity: w trybie snapshot-only podać `pool_liquidity_active` per timestamp i dzielić przez `L_active(t)` (range-aware share) zamiast stałej wartości z `fetch_pool_state`.
- [ ] **E2.6** Anty-przeszacowanie: dodać kalibrację skalującą model share/fee (np. `share_calibrated = clamp(k * share_est, 0, 1)`) wyznaczaną na bazowym „referencyjnym” zakresie (najlepiej z `TIR ~ 100%`) tak, żeby model nie generował nierealnie wysokich `Fees Earned` dla wąskich range’ów.
- [ ] **E2.7** Guardrail sanity-check: porównywać “modeled pool fees vs snapshot pool fees” oraz monitorować “share mass balance” po rebalansie (jeśli przekracza X%, log + clamp/fallback do legacy share).

## Tech debt

- [x] **T1** Usunąć martwy kod po `return` w handlerach snapshot w `main.rs` (ostrzeżenia `unreachable_code`).
- [x] **T2** `quote_usd_map` unused assignment w backtest — posprzątać.

---

## Log wykonania (uzupełniaj przy commitach)

| Data | Krok | Notatka |
|------|------|---------|
| 2026-03-19 | B1 + B3 | Histogram w `swaps-decode-audit`; `--fee-swap-decode-status` w `backtest`. |
| 2026-03-19 | C1 (częściowo) | Zapisany gap: optimize ≠ local swaps decode. |
| 2026-03-19 | C1 + C2 | Moduł `local_swap_fees.rs`; `run_grid` + `backtest-optimize` z lokalnymi opłatami. |
| 2026-03-20 | C2b | Hybrydowe merge: decoded + timing proxy (P1.2) — brakujące kroki nie dawały już $0. |
| 2026-03-20 | B2 (Orca) | Parser Anchor `Traded` (`Program data:`); status `ok_traded_event`; audit + `ok` filter uwzględniają. |
| 2026-03-22 | T1+T2+C3 | Snapshot handlery tylko `snapshots::collector::*?.await?`; `quote_usd_map` w scope Birdeye; test regresyjny `build_local_pool_fees_uses_decoded_swaps_when_strict_ok` + `repo_data_dir()` pod testy. |
| 2026-03-22 | C1b | `backtest-optimize`: `--price-path-source snapshots`, `--start-date`/`--end-date`; grid na `snapshot_price_path::build_from_orca_snapshots`. |
| 2026-03-22 | B2 (Raydium) | `parse_raydium_swap_event_for_pool` + `ok_swap_event` w enrich / audit / `--fee-swap-decode-status ok`. |
| 2026-03-22 | B2 (Meteora) | `parse_meteora_swap_event_for_pool` (`events/meteora_swap_event.rs`); swap_sync + ten sam `ok_swap_event`. |
