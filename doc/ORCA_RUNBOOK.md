# Orca Only Runbook (snapshots -> swaps decode -> fee/backtest)

## Cel
Powtarzalnie przygotować dane dla Orca Whirlpool tak, aby:
- `fee_source=snapshots` miało wystarczające snapshoty (tier 2),
- `fee_swap_decode_status=ok` miało sensowną pokrywalność dekodowania,
- backtest/optimize na bazie lokalnych snapshots + dekodowanych swapów dawał stabilne wyniki.

## Założenia / wejścia
- Curated Orca puli są zdefiniowane w `STARTUP.md` (sekcja Orca).
- Interesują Cię jedynie wyniki w `data/pool-snapshots/orca/...` oraz `data/swaps/orca/...` (pozostałe protokoły ignorujemy interpretacyjnie, choć komendy potrafią je też przejść).
- Komendy odpalasz w PowerShell z katalogu repo: `F:\CLMM-Liquidity-Provider\CLMM-Liquidity-Provider`.

## Parametry “bezpieczne” dla Orca (polecane do stabilizacji RPC)
Używaj poniższych domyślnych wartości:
- `--max-decode 40`
- `--decode-timeout-secs 25`
- `--decode-retries 2`
- dla testu skróć: `--max-decode 10`, `--decode-timeout-secs 15`, `--decode-retries 0`

## Krok 0: wybór puli do weryfikacji (opcjonalne, ale zalecane)
1. Wybierz 1–2 adresy `orca` poola (Whirlpool address) do “smoke test”.
2. Dla każdego wybranego poola przygotuj nazwę/ID tak, by łatwo znaleźć folder: `data/pool-snapshots/orca/<pool>/snapshots.jsonl` oraz `data/swaps/orca/<pool>/decoded_swaps.jsonl`.

## Krok 1: Snapshoty Orca (Tier 2 przygotowanie)
1. Dodaj świeży snapshot dla Orca (np. pierwsze `N` puli):
   - `cargo run --bin clmm-lp-cli -- orca-snapshot-curated --limit <N>`
2. Potwierdź readiness dla wybranych pooli (dla weryfikacji tier 2):
   - `cargo run --bin clmm-lp-cli -- snapshot-readiness --protocol orca --pool-address <POOL_ADDRESS>`
3. Sukces: `READY` dla co najmniej jednego targetowego poola (albo rosnące pokrycie w kryteriach fee-growth/protocol-fee).

## Krok 2: Sync surowych swapów (P1 -> P1.1 baza do dekodowania)
1. Zaciągnij ostatnie transakcje dla curated puli:
   - `cargo run --bin clmm-lp-cli -- swaps-sync-curated-all --limit <N> --max-signatures <MAX_SIGNATURES>`
2. Sukces: pojawiają się/aktualizują nowe wiersze w:
   - `data/swaps/orca/<pool>/swaps.jsonl`

## Krok 3: Dekodowanie swapów (Orca decode quality)
1. Zdekoduj z re-wyliczeniem (zalecane po zmianach w dekoderze / fallbackach):
   - `cargo run --bin clmm-lp-cli -- swaps-enrich-curated-all --limit <N> --max-decode 40 --refresh-decoded --decode-timeout-secs 25 --decode-retries 2`
2. Audit jakości dekodowania:
   - `cargo run --bin clmm-lp-cli -- swaps-decode-audit --limit <N> --save-report`
3. Sukces (minimalny, operacyjny):
   - dla `data/swaps/orca/<pool>/decoded_swaps.jsonl` powinno być widoczne `decode_status` wśród:
     - `ok_traded_event` (preferowane dla Orca),
     - `ok` (akceptowalne, jeśli log też jest poprawny),
     - `partial` i `missing_meta` jako margines (nie większość).

## Krok 4: “Szybki lokalny” histogram decode_status (Orca tylko)
Jeśli chcesz szybciej niż patrzeć na raporty, uruchom w repo:
```powershell
node -e "
const fs=require('fs');
const base='data/swaps/orca';
for(const pool of fs.existsSync(base)?fs.readdirSync(base):[]){
  const f=`${base}/${pool}/decoded_swaps.jsonl`;
  if(!fs.existsSync(f)) continue;
  const txt=fs.readFileSync(f,'utf8').trim();
  if(!txt){console.log(pool,'(empty)'); continue;}
  const counts={};
  txt.split(/\\r?\\n/).filter(Boolean).forEach(l=>{
    const j=JSON.parse(l);
    const k=j.decode_status||'missing_decode_status';
    counts[k]=(counts[k]||0)+1;
  });
  console.log(pool,counts);
}
"
"
```
Sukces: wśród liczb dominują `ok_traded_event`/`ok`, a nie `partial`/`missing_*`.

## Krok 5: Opcjonalnie backtest/optimize pod Orca (fee_source=snapshots)
Uwaga: `backtest-optimize` wymaga mintów A/B (i zwykle Whirlpool adresu do snapshotów).
1. Uruchom optymalizację dla wybranej pary tokenów:
```powershell
cargo run --bin clmm-lp-cli -- backtest-optimize `
  --mint-a <MINT_A> --mint-b <MINT_B> `
  --days <DAYS> `
  --resolution-seconds 3600 `
  --capital 7000 `
  --tx-cost 0.1 `
  --use-realistic-rebalance-cost `
  --network-fee-usd 0.08 `
  --priority-fee-usd 0.12 `
  --jito-tip-usd 0.0 `
  --slippage-bps 5.0 `
  --fee-swap-decode-status ok `
  --price-path-source snapshots `
  --snapshot-protocol orca `
  --snapshot-pool-address <WHIRLPOOL_ADDRESS>
```
2. Sukces: wyniki są stabilne (nie “skaczą” drastycznie po ponownym runie) i nie ma systematycznych ostrych regresji vs-hodl/pnl.

## Koszty Rebalansów (gdzie to się mapuje w projekcie)
- W backtest/optimize koszt rebalansu jest liczone jako: `fixed(network+priority+jito+tx_cost) + slippage_bps * rebalanced_notional`.
- Dopóki nie mamy kalibracji parametrów z realnych obserwacji (priority fee, jito tip, efektywny slippage), traktuj `--network-fee-usd/--priority-fee-usd/--jito-tip-usd/--slippage-bps` jako “modelowe” priorytety i dostrajaj je testowo na Orca.
- W roadmap `STARTUP.md` plan zakłada “Hardening and risk controls”: limity na gas/priority fees oraz max rebalance frequency w docelowym b0cie.

### Ustawianie strategii rebalance (periodic vs threshold)
Dla komendy `backtest` (nie `backtest-optimize`) strategię rebalance wybierasz flagą `--strategy`:
- `--strategy periodic --rebalance_interval <HOURS>` (periodic co N godzin)
- `--strategy threshold --threshold_pct <PCT>` (threshold gdy cena przekroczy próg)
- `--strategy il_limit` (IL-aware rebalance/close logic w warstwie strategii)

W `backtest-optimize` domyślna siatka obejmuje 5 strategii: `static_range`, `periodic`, `threshold`, `il_limit`, `retouch_shift`.
Parametry IL-limit dla gridu:
- `--il-max-pct <PCT>` (domyślnie 5),
- `--il-close-pct <PCT>` (opcjonalny próg zamknięcia),
- `--il-grace-steps <N>` (domyślnie 0).

Różnica komend:
- `optimize` -> warstwa analityczna (szybkie rekomendacje parametrów),
- `backtest-optimize` -> warstwa historyczna (grid po świecach/swapach).

Okres historyczny (czas dla backtestu) jest podawany przez użytkownika i jest to:
- `--days <DAYS>` oraz opcjonalnie `--hours <HOURS>` (nadpisuje `--days`), albo
- `--start-date <YYYY-MM-DD>` / `--end-date <YYYY-MM-DD>` (nadpisuje `--days`).

## Krok 6: Monitor “czy pipeline nie umarł” (zalecane na koniec dnia)
1. Health check snapshotów i decode jakości:
   - `cargo run --bin clmm-lp-cli -- data-health-check --max-age-minutes 30 --min-decode-ok-pct 65`
2. Jeśli używasz schedulerów:
   - dodaj `--fail-on-alert`, żeby pipeline zwracał kod błędu.

## Procedura eskalacji (gdy Orca decode coverage spada)
1. Zwiększ stabilność dekodowania:
   - podnieś `--decode-timeout-secs` do 30–40,
   - podnieś `--decode-retries` do 3,
   - zwiększ `--max-decode` dopiero, gdy timeouty przestają dominować.
2. Upewnij się, że snapshoty spełniają tier 2 readiness:
   - run `snapshot-readiness --protocol orca --pool-address <POOL>` jeszcze raz po nowym snapshot-run.
3. Jeśli dalej dominują `partial`:
   - porównaj 5–10 konkretnych sygnatur `decoded_swaps.jsonl` (w raporcie z `swaps-decode-audit --save-report`).

## Sprint 1: Bot Dry-Run Session (Orca-first)

Cel:
- uruchomić pełny loop decyzji i execution bez ryzyka on-chain,
- potwierdzić, że tx lifecycle (simulate/send/confirm path) i lifecycle events działają spójnie.

Zakres:
- tylko Orca,
- jeden pool na start: `HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF`,
- `dry_run=true`.

### Checklist przed startem dry-run
- [ ] Aktualny build/test zielony dla `clmm-lp-execution`.
- [ ] Strategy config ustawiony dla targetowego poola.
- [ ] Tryb `dry_run=true` potwierdzony.
- [ ] Alert stream aktywny (log/API/ws).
- [ ] Lifecycle tracker zapisuje eventy.

### Co obserwować w trakcie sesji
- [ ] Decyzje są zgodne ze strategią (price-reactive, bez sztucznego tłumienia triggerów).
- [ ] Przy `Rebalance` powód (`reason`) jest poprawnie tagowany (`Periodic`, `RangeExit`, `RetouchShift`, itp.).
- [ ] Przy flow rebalance fee collection jest próbowane przed zmianą zakresu.
- [ ] Brak cichych sukcesów: nieudane operacje zwracają błąd i są widoczne w logach.
- [ ] Brak pętli powtarzalnych błędów bez eskalacji.

### Minimalne kryteria zaliczenia dry-run
- [ ] Co najmniej jedna pełna ścieżka decyzji `Hold` i jedna `Rebalance` przeszła przez loop.
- [ ] Brak krytycznych błędów nieobsłużonych przez retry/eskalację.
- [ ] Spójność: to co w logach == to co w lifecycle.
- [ ] Worklog z sesji został dopisany (co działało, co nie, co poprawiono).

## Go/No-Go do Limited Live (Stage 1)

**GO** jeśli wszystkie warunki:
- [ ] Dry-run zaliczony wg sekcji powyżej.
- [ ] Retry/simulation/confirmation path działa stabilnie.
- [ ] Operator ma gotowy wallet (`PRIVATE_KEY`) i procedurę stop/rollback.
- [ ] Uzgodniony mały kapitał testowy i okno nadzoru operatora.
- [ ] Brak otwartych krytycznych incydentów z ostatniej sesji.

**NO-GO** jeśli którykolwiek warunek:
- [ ] Niespójność między decyzją a wykonaniem (ghost success/failure).
- [ ] Powtarzalne błędy tx bez jasnego root cause.
- [ ] Brak potwierdzonej procedury emergency stop.

