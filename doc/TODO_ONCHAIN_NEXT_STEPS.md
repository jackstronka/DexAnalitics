# TODO: on-chain fees pipeline — co zostało i kolejność prac

**Ostatnia aktualizacja:** 2026-03-22  
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
- [ ] **B2 (Raydium / Meteora)** — jeszcze do zrobienia.
- [ ] **B4** Rate limiting / kolejka RPC w enrich (opcjonalnie).

## Faza C — spójność `backtest` vs `backtest-optimize`

- [x] **C1** (rdzeń) Gdy **brak Dune** lub **pusta** lista swapów po filtrze: `backtest-optimize` buduje opłaty z **`data/swaps`** (decode → timing proxy), jak `backtest`. Wymaga `--snapshot-protocol` + `--snapshot-pool-address` lub `--whirlpool-address` jako adres poola. Flaga **`--fee-swap-decode-status`** (ok/loose) jak w `backtest`.
- [ ] **C1b** Tryb **snapshot-only price path** w optimize (jeśli ma być 1:1 z `backtest`) — nadal do decyzji.
- [x] **C2** `run_grid` / `run_single` przyjmują opcjonalny **`local_pool_fees_usd`** (wspólny silnik).
- [x] **C2b** Hybrydowe wypełnianie brakujących bucketów: decoded (gdzie jest) + tx-count proxy (gdzie brak).
- [ ] **C3** Test regresyjny (jedna konfiguracja → spodziewane źródło danych).

## Faza D — P2 / P3 (aktywna płynność)

- [ ] **D1** Snapshoty: prawdziwe konta tick/bin w sąsiedztwie (nie tylko indeksy).
- [ ] **D2** Join swap ↔ snapshot; model `fee * L_pos / L_active`.
- [ ] **D3** Porównanie z baseline w `backtest_fee_compare_*.json`.

## Faza E — P4 (Orca)

- [ ] **E1** `fee_growth_inside` / tick arrays — wg `ONCHAIN_FEES_TRUTH_PLAN.md`.

## Tech debt

- [ ] **T1** Usunąć martwy kod po `return` w handlerach snapshot w `main.rs` (ostrzeżenia `unreachable_code`).
- [ ] **T2** `quote_usd_map` unused assignment w backtest — posprzątać.

---

## Log wykonania (uzupełniaj przy commitach)

| Data | Krok | Notatka |
|------|------|---------|
| 2026-03-19 | B1 + B3 | Histogram w `swaps-decode-audit`; `--fee-swap-decode-status` w `backtest`. |
| 2026-03-19 | C1 (częściowo) | Zapisany gap: optimize ≠ local swaps decode. |
| 2026-03-19 | C1 + C2 | Moduł `local_swap_fees.rs`; `run_grid` + `backtest-optimize` z lokalnymi opłatami. |
| 2026-03-20 | C2b | Hybrydowe merge: decoded + timing proxy (P1.2) — brakujące kroki nie dawały już $0. |
| 2026-03-20 | B2 (Orca) | Parser Anchor `Traded` (`Program data:`); status `ok_traded_event`; audit + `ok` filter uwzględniają. |
