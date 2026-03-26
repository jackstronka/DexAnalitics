# TODO: on-chain fees pipeline — co zostało i kolejność prac

**Ostatnia aktualizacja:** 2026-03-26 (M1/M2/B4 domknięte w repo; kolejka = operacja A)  
**Powiązane:** `doc/ONCHAIN_FEES_TRUTH_PLAN.md`, `doc/ONCHAIN_FEES_PROGRESS.md`, `doc/ORCA_RUNBOOK.md`, `doc/BACKTEST_OPTIMIZE_STRATEGIES.md`

## Od czego zacząć (kolejność sensowna)

1. **RPC (bez tego reszta stoi)** — ustaw własny endpoint: `SOLANA_RPC_URL`, opcjonalnie `SOLANA_RPC_FALLBACK_URLS` (comma-separated). Publiczne RPC często timeoutują na `getTransaction`; limity równoległości w enrich są już w kodzie (**B4 / M2**), ale **jakość danych** nadal zależy od endpointu.
2. **Operacyjnie: odtwórz decode i zmierz jakość** — **[A1] + [A2]** (to jest teraz główny „następny krok”): `swaps-enrich-curated-all` z sensownym `--max-decode` / `--decode-concurrency` / opcjonalnie `CLMM_ENRICH_DECODE_INFLIGHT`, potem `swaps-decode-audit --save-report`. Komendy: `doc/ORCA_RUNBOOK.md` (Krok 2–3); to samo dla Raydium/Meteora, inny folder pod `data/swaps/<protocol>/`.
3. ~~**Kod: kolejka RPC w enrich [M2]**~~ — **zrobione:** `swaps-enrich-curated-all` ogranicza równoległość (`--decode-concurrency`, override env `CLMM_ENRICH_DECODE_INFLIGHT`), jitter w `decode_one_signature_with_retry` + `CLMM_ENRICH_DECODE_JITTER_MS`; szczegóły: `doc/ORCA_RUNBOOK.md`, `crates/cli/src/swap_sync.rs`.
4. ~~**Kod: Meteora TVL / mniej `--lp-share` [M1]**~~ — **zrobione:** snapshot zapisuje `vault_amount_a` / `vault_amount_b`; `build_from_meteora_snapshots` liczy `lp_share` z TVL jak Raydium, gdy w oknie są oba pola (stary JSONL bez vaultów → nadal `--lp-share` lub ponowny snapshot).
5. Dalsze cele research (P2/P3): fazy **D** / **E2** — dopiero gdy P1 (decode + snapshot cadence) jest stabilne.

### Semantyka backtestu (nie on-chain, ale ważne przy interpretacji wyników)

- Benchmark **HODL** i cel **`risk_adj`** (`PnL/(1+DD)`, nie Sharpe): `doc/BACKTEST_OPTIMIZE_STRATEGIES.md` (sekcje *Benchmark HODL* i *risk_adj vs Sharpe*).

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
- [ ] **A5** Stabilne uruchamianie 24/7 bez Task Scheduler: `ops-ingest-loop` jako proces długowieczny + Windows Service (np. NSSM) z auto-restart i logowaniem. Zalecany minimalny setup w NSSM:
  - rekomenduj uruchamiać gotowego bika (np. `target\\release\\clmm-lp-cli.exe` lub `target\\debug\\clmm-lp-cli.exe`), nie `cargo run` (żeby uniknąć problemów z working dir i parserem/PTY),
  - ustaw `Startup directory` = katalog repo (`F:\\CLMM-Liquidity-Provider\\CLMM-Liquidity-Provider`), żeby względne ścieżki do `data/` działały,
  - w `Application parameters` ustaw np. `ops-ingest-loop --interval-secs 900 --jitter-secs 60 --swaps-max-pages 2 --swaps-max-signatures 600 --enrich-max-decode 160 --health-max-age-minutes 30 --health-min-decode-ok-pct 65.0 --fail-on-alert true`,
  - ustaw env: `SOLANA_RPC_URL` oraz opcjonalnie `SOLANA_RPC_FALLBACK_URLS` (comma-separated), aby service używał tych samych endpointów co ręczne uruchomienia,
  - włącz auto-restart na exit code w NSSM (typowo zakładka „Details” / „Shutdown” + „Restart” zależnie od wersji),
  - loguj stdout/stderr do plików (np. `data\\logs\\ops-ingest-loop-stdout.log` i `data\\logs\\ops-ingest-loop-stderr.log`), bo to jest najczęściej brakujący element w „Task Scheduler zawiódł” przypadkach.

## Faza B — jakość P1.1 (kod + walidacja)

- [x] **B1** Audyt: globalny histogram `decode_status` + % w raporcie JSON (CLI).
- [x] **B3** Backtest: flaga `--fee-swap-decode-status` (`ok` = tylko czyste swapy, `loose` = poprzednie zachowanie).
- [x] **B2 (Orca)** Anchor event `Traded` z `Program data:` (Borsh): `lp_fee`, `post_sqrt_price`, `input/output_amount`, `a_to_b`; `decode_status` może być `ok_traded_event`.
- [x] **B2 (Raydium)** — Anchor `SwapEvent` z `Program data:` → `decode_status = ok_swap_event`; filtry `ok` / `local_swap_fees` jak Orca.
- [x] **B2 (Meteora)** — DLMM `MeteoraDlmmSwapEvent` (Borsh jak carbon decoder); dyskryminatory `event:SwapEvent` i `event:Swap`; `lb_pair` == pool; `decode_status = ok_swap_event`.
- [x] **B4** Rate limiting / kolejka RPC w enrich — **zamknięte razem z [M2]** (implementacja w `crates/cli/src/swap_sync.rs`: limit równoległych dekodów przez `buffer_unordered(decode_inflight)`; `decode_inflight` z `--decode-concurrency` lub `CLMM_ENRICH_DECODE_INFLIGHT`; jitter przed próbą + opcjonalnie `CLMM_ENRICH_DECODE_JITTER_MS`).
  - **Problem (symptomy), jeśli nadal występuje:** dużo `decode_status=partial` / timeout mimo limitów — wtedy to **nie** jest już „brak kolejki w kodzie”, tylko **jakość endpointu** lub historia transakcji.
  - **Najczęstsza przyczyna:** publiczne RPC rate-limitują lub nie utrzymują wystarczająco długiej historii dla `getTransaction`.
  - **Stan kodu (zrobione w repo):** `swap_sync` korzysta z `RpcProvider` (retry + rotacja endpointów), dekoduje najnowsze sygnatury jako pierwsze i filtruje do okna ~72h; env endpointów:
    - `SOLANA_RPC_URL`
    - `SOLANA_RPC_FALLBACK_URLS` (comma-separated)
  - **Co jeszcze dopiąć (operacyjnie / RPC):** potwierdzić na własnym RPC, że `getTransaction` odpowiada sensownie dla sygnatur z okna 24h/48h i rośnie `% decode_status=ok`; przy dominacji timeoutów: archival/dedykowany endpoint, wyższe `--decode-timeout-secs`, niższa równoległość jeśli endpoint jest ciasny.
  - **Kierunki z internetu (kontekst):**
    - `getTransaction` (parametry/encoding): https://solana.com/docs/rpc/http/gettransaction
    - czasem pomagają drobiazgi typu poprawny endpoint URL (bez portu `:8899` w adresie, jeśli używasz standardowych hostów)
    - strategie retry/backoff: https://solana.com/docs/advanced/retry
    - ograniczenia historycznych danych na standardowych RPC (archival): https://docs.solanalabs.com/implemented-proposals/rpc-transaction-history
    - archival/historical data provider (np. Helius): https://www.helius.dev/docs/rpc/historical-data

## Faza M — krótki sprint (kod; dopina P1.1)

Powiązanie z checklistą „od czego zacząć” u góry. Po ukończeniu wpisz krótką notatkę w **Log wykonania**.

- [x] **M1 (Meteora TVL / `lp_share`)** — snapshot zapisuje rezerwy vaultów (`vault_amount_a` / `vault_amount_b`); `build_from_meteora_snapshots` wylicza `lp_share` z `capital/TVL` w oknie, gdy oba pola są w JSONL (jak Raydium). Stare pliki bez vaultów: `--lp-share` lub ponowny snapshot.
- [x] **M2 (kolejka RPC w enrich)** — `swaps-enrich-curated-all` ogranicza równoległość (`decode_inflight` z CLI/env), jitter przed próbą decode; `RpcProvider` bez zmian wymaganych do samego limitu (retry/failover jak wcześniej).

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

## Faza F — IL semantics unification (backtest)

Cel: wyeliminować mieszanie różnych definicji IL pod jedną etykietą w raportach.

- [x] **F1** Raporty CLI: rozdzielić metryki na:
  - `IL vs HODL (ex-fees)` (metryka end-of-backtest),
  - `IL Segment (last)` (klasyczne CLMM IL dla ostatniego segmentu po rebalansach).
- [x] **F2** `TrackerSummary`: dodać jawne pola dla obu metryk i zachować kompatybilność (`final_il_pct` jako alias legacy do `IL vs HODL (ex-fees)`).
- [x] **F3** Ujednolicić źródła metryk w `backtest_engine` i `position_tracker`, żeby pola miały tę samą semantykę.
- [x] **F4** Test regresyjny: static vs threshold (wielokrotne rebalance) i asercje, że:
  - `IL Segment (last)` może różnić się od `IL vs HODL (ex-fees)`,
  - etykiety/metryki nie są już mylone.

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
| 2026-03-25 | Docs | Sekcja „Od czego zacząć”, faza **M** (M1/M2), powiązanie B4↔M2; data aktualizacji nagłówka. |
| 2026-03-26 | M1 + M2 + B4 | Meteora: TVL z vaultów → `lp_share` w snapshot path; enrich: limit równoległości + jitter (`swap_sync`); TODO: checklist B4/M1/M2 [x], start operacyjny = A1/A2. |
