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

## RPC przed dekodowaniem (wszystkie protokoły w curated)

- Ustaw **`SOLANA_RPC_URL`** (własny / płatny / archival jeśli potrzeba historii). Opcjonalnie **`SOLANA_RPC_FALLBACK_URLS`** (lista po przecinku).
- **Mainnet:** rozważ **`CLMM_EXPECTED_CLUSTER=mainnet-beta`** — przy starcie `RpcProvider` odrzuci oczywiste mieszanie klastrów (np. devnet URL przy intencji mainnet). Szczegóły: [`doc/MAINNET_OPERATIONAL_CHECKLIST.md`](MAINNET_OPERATIONAL_CHECKLIST.md).
- **Mainnet (pre-live, sekcja danych):** zanim otworzysz jakąkolwiek pozycję na mainnecie, zmierz i zapisz **minimum position sizing** (to nie jest stała kwota; zależy od `current_tick`, `tick_spacing` i range). Procedura + szablon tabeli: [`doc/MAINNET_MIN_POSITION_SIZING.md`](MAINNET_MIN_POSITION_SIZING.md).
- **Ledger kosztów cyklu życia (CLI + bot + swap):** ten sam plik `data/ledger/orca_position_lifecycle.jsonl` (override: `CLMM_POSITION_LIFECYCLE_LEDGER_PATH`; alias: `CLMM_POSITION_OPEN_LEDGER_PATH`). Po udanym `orca-position-open` / `orca-position-close` wiersze mają `source=cli`; po udanym **`orca-swap`** — `event=cli_swap`; po udanych operacjach Orca w **`RebalanceExecutor`** (collect / decrease / close / open) wiersze mają `source=orca_bot` i `event=bot_*`. Pola kosztów jak wyżej: `tx_fee_lamports` (`meta.fee`) oraz **`fee_payer_net_lamports_delta`** z `postBalances−preBalances` dla płatnika — przy zamknięciu część SOL wraca (rent/liquidity), więc delta bywa dodatnia; **łączny efekt** licz jako sumę wierszy open+close (nogi SPL osobno). **Sekwencja swap + rebalans + open:** ustaw **`CLMM_REBALANCE_SESSION_ID`** (ta sama wartość dla wszystkich kroków w tej samej sesji powłoki); wtedy każdy wiersz może mieć **`rebalance_session_id`** — sumuj po tym polu, żeby zsumować **swap + wszystkie tx rebalansu i otwarcia pozycji**.
- Bez sensownego `getTransaction` enrich nadal zwróci głównie `partial` / timeout — wtedy najpierw endpoint; równoległość jest ograniczona przez `--decode-concurrency` / `CLMM_ENRICH_DECODE_INFLIGHT` (patrz krok 3).

## Smoke mainnet: open → close (mały kapitał) dla curated Orca

- **Curated Orca w `STARTUP.md`:** `SOL/USDC` (`Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE`), `whETH/SOL` (`HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF`), `cbBTC/USDC` (`HxA6SKW5qA4o12fjVgTpXdq2YnZ5Zv1s7SB4FFomsyLM`). Pełny przejazd **otwarcie → krótki sleep → zamknięcie** dla wszystkich trzech pooli:  
  `powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\orca_position_smoke_curated_pools.ps1 -Verify`  
  (najpierw ustaw `SOLANA_RPC_URL` / `mainnet_rpc_env.ps1`; portfel musi mieć **oba tokeny** danej pary — np. cbBTC/USDC wymaga **cbBTC i USDC** w surowych jednostkach co najmniej `-AmountA`/`-AmountB`).
- **Preflight przed open (zautomatyzowany zestaw):** `tools/orca_position_open_preflight.ps1` — `orca-pool-read` + `solana_account_state.ps1 -Json`; sprawdza `amount_a`/`amount_b` (raw) vs salda (dla wSOL: native + ATA), oraz rezerwę native SOL na fee (`-ReserveSolLamports`, domyślnie 15_000_000). **Domyślnie** wywołuje go `orca_position_open_then_close_quick.ps1` / `orca_position_open_then_close_fast.ps1` przed `orca-position-open`; pominąć tylko świadomie: `-SkipPreflight`.
- **Auto-fund przed open (ta sama pula):** logika w `tools/orca_position_preflight_core.ps1` — pętla **exact-out** `orca-swap` na **tym samym** poolu, aż preflight przejdzie (`Invoke-OrcaPositionAutoFundFromPool`). Włącz: **`-AutoFund`** na `orca_position_open_then_close_quick.ps1` / `orca_position_open_then_close_fast.ps1` / `orca_position_smoke_curated_pools.ps1` (oraz parametry `-AutoFundMaxRounds`, `-FundSwapSlippageBps`, `-FundDeficitBufferBps`). Tylko **nogi tokenowe** (A/B); jeśli brakuje **native SOL** względem `-ReserveSolLamports`, skrypt zatrzymuje się (swap tego nie naprawia). Samo przygotowanie sald bez open: `tools/orca_position_auto_fund_for_open.ps1`.
  - **Planowanie swapów (żeby uwzględnić ograniczenia auto-fund):** skrypt ma **ustaloną kolejność**: dopóki jest deficyt **tokenu A** (wg `token_mint_a` z puli), robi **exact-out na A** (płacisz **B**); gdy A jest OK, dopiero ewentualnie **exact-out na B** (płacisz **A**). Nie wybiera „inteligentnie” który deficyt jest pilniejszy. Praktyka: przed `-AutoFund` policz deficyty z preflightu i **upewnij się, że na starcie masz wystarczający zapas nogi „rozliczeniowej”** do pierwszego kroku (zwykle duży zapas **B**, jeśli brakuje obu — bo najpierw dolewane jest A). Jeśli **obie** nogi są za krótkie i **żadna** nie może zapłacić pierwszego swapu, najpierw **doładuj portfel ręcznie / CEX / inną pulą**, albo **zmniejsz** `-AmountA`/`-AmountB`, albo wykonaj **własną** sekwencję swapów (kolejność i źródło płynności pod Twoją kontrolę), a potem odpal open z preflightem. **Myśl osobno** o rezerwie native SOL na fee — auto-fund jej nie buduje.
- **Jedna pula / dry-run logiki:** `tools/orca_position_open_then_close_quick.ps1` (preferuje `target\release\clmm-lp-cli.exe` jeśli zbudowany; `-CargoOnly` wymusza `cargo run`). **Szybsze iteracje / aktualny exe:** `.\tools\build_clmm_lp_cli.ps1` (albo cienki wrapper `.\tools\build_clmm_lp_cli_release.ps1`).
- **Spójność z `quick_verify_data`:** ten sam pool **cbBTC/USDC** (`HxA6…`) jest w smoke curated i w `tools/quick_verify_data.ps1` — jeden adres, jedna para.
- **Swapy na 3 curated parach (wszystkie sensowne kierunki w obrębie pary):**  
  - Lista pul + przykłady 4 wariantów na parę (exact-in / exact-out, obie nogi):  
    `powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\orca_swap_curated.ps1 -ListPairs`  
  - Wykonanie: `-Pair SOL_USDC | WHETH_SOL | CBBTC_USDC`, `-From` / `-To` (symbole **SOL**, **USDC**, **WHETH**, **CBBTC**; aliasy **WSOL→SOL**, **ETH/WETH→WHETH**, **BTC/WBTC→CBBTC**), `-SwapType exact-in|exact-out`, `-AmountRaw` (**jednostki From** przy `exact-in`, **jednostki To** przy `exact-out`), opcjonalnie `-Execute`, `-SlippageBps`, `-CargoOnly`. Skrypt woła `tools/orca_swap.ps1` (preferuje `target\*\clmm-lp-cli.exe` jak w innych narzędziach).  
  - **Między parami** (np. z **SOL** zrobić **USDC** na `SOL_USDC`, potem **USDC→cbBTC** na `CBBTC_USDC`): **dwa** osobne wywołania `orca_swap_curated.ps1` — jedna pula nie przyjmuje SOL obok własnych dwóch mintów.  
  - **Źródło prawdy adresów:** `tools/orca_curated_mainnet_pools.ps1` (importowane też przez `orca_position_smoke_curated_pools.ps1`).
  - **Jedna bramka (rebalance / przygotowanie):** `tools/orca_curated_rebalance.ps1 -Action Help` — `ListPairs`, `Preflight`, `Open` (live, `-OpenOnly` wewnętrznie), `Close` (`-Position`), `Swap` (deleguje do `orca_swap_curated.ps1`), `FundCbBtc`, `Smoke`.
- **Szacunek + swapy pod open na cbBTC/USDC:** `tools/orca_fund_cbbtc_usdc_open.ps1` — preflight na `HxA6…`, brakujące **cbBTC** szacuje z **dry-run** `orca-swap` (exact-out cbBTC), brak **USDC** z sumy (noga pozycji + USDC na ten swap); niedobór USDC uzupełnia planem **SOL→USDC** na puli `SOL_USDC` (exact-out USDC, też z dry-run). Domyślnie tylko **plan**; **`-Execute`** wysyła transakcje (`orca_swap.ps1`) w kolejności SOL/USDC → cbBTC/USDC, potem ponowny preflight. Jeśli portfel ma za mało **SOL** względem szacunku, skrypt kończy się błędem (zmniejsz `-AmountA`/`-AmountB` lub doładuj SOL).
- **RPC:** skrypty w `tools/` mogą ustawić domyślne `CLMM_RPC_DENYLIST=ankr,projectserum` na mainnecie (patrz `tools/clmm_rpc_tools_helpers.ps1`), żeby unikać złych fallbacków.
- **Pierwsze uruchomienie wielu botów na tej samej parze (np. 3× whETH/SOL, różne strategie rebalansu + szablon dziennika):** [`doc/WHETH_SOL_THREE_BOTS_FIRST_RUN.md`](WHETH_SOL_THREE_BOTS_FIRST_RUN.md).

## Rebalance Whirlpool: zmiana zakresu (ticki) i swap

### Czy trzeba zamknąć pozycję i otworzyć nową, żeby mieć inne ticki?

**W typowym flow — tak.** W Orca Whirlpool (jak w modelu Uniswap v3) **jedna pozycja (NFT)** jest powiązana z **stałą parą** `tick_lower` / `tick_upper` na danej puli. Nie ma operacji „zmień zakres tej samej pozycji na nowe ticki”: żeby grać w **innym** przedziale, standardowo robisz:

1. **Collect fees** (opcjonalnie, zależnie od strategii),
2. **Decrease liquidity** (często do zera na starej pozycji),
3. **Close position** (zwolnienie kont, NFT),
4. **Open position** z nowymi tickami → **nowy adres pozycji (PDA)**.

W tym repozytorium **`RebalanceExecutor`** realizuje właśnie ten wzorzec (zamknięcie starej pozycji + otwarcie nowej z nowym zakresem). **Alternatywa bez zamykania:** możesz **otworzyć drugą** pozycję w innym zakresie i utrzymywać dwie pozycje równolegle (dwa NFT, podział kapitału); to nie zastępuje „przesunięcia” pierwszej pozycji.

### Swap przed rebalansem i `CLMM_REBALANCE_SESSION_ID` (stan vs docelowo)

- **`RebalanceExecutor` (Rust) nie wykonuje swapu** Whirlpool w tej samej ścieżce co rebalans — jeśli musisz **najpierw** wymienić tokeny (np. wyrównać SOL/USDC przed `close`/`open`), użyj osobno **`orca-swap`** albo skryptu; udany swap trafia do ledgeru jako `event=cli_swap` (ten sam plik JSONL co pozycja).
- Żeby **policzyć całość** (swap + tx rebalansu + open): ustaw **`CLMM_REBALANCE_SESSION_ID`** na **jedną** wartość **przed** każdym krokiem w tej samej sesji powłoki / w tym samym procesie, który uruchamia kolejne komendy — wiersze z tym samym `rebalance_session_id` możesz sumować (`tx_fee_lamports` albo delty płatnika, zgodnie z tym, co mierzysz).
- **Proces bota:** jeśli startujesz bota z już ustawionym `CLMM_REBALANCE_SESSION_ID` w środowisku, wiersze `source=orca_bot` też dostaną to pole (o ile env jest widoczny dla procesu).
- **Docelowo (integracja w kodzie):** gdy swap zostanie dodany przed rebalansem w Rust, sensowne jest **jedno** id sesji generowane w kodzie na początku sekwencji (np. z konfiguracji lub UUID) i przekazywane do zapisów ledgeru — bez polegania na ręcznym `export` między osobnymi procesami. Do tego momentu **wspólne env w jednej sesji** jest prostym sposobem na spójne sumowanie kosztów.

### Historia rebalansów (liczenie zdarzeń) i kosztów tx

- **Zdarzenie „rebalance” w pliku (opcjonalnie):** uruchamiając bota z **`--il-ledger-path <plik.jsonl>`** (`orca-bot-run` / `orca-bot-open-and-run`), po **udanym** `RebalanceExecutor::execute` (zamknięcie starej pozycji + otwarcie nowej) `LifecycleTracker::record_rebalance` dopisuje wiersz JSON z `"event": "rebalance"` (m.in. stare/nowe ticki, `reason`, `optimization_run_id`). **Bez** tej flagi historia rebalansów jest tylko w logach procesu / pamięci trackera — **nie** w osobnym pliku.
- **Koszty każdej tx (fee + delta SOL płatnika):** nadal **`data/ledger/orca_position_lifecycle.jsonl`** — wiersze `source=orca_bot` (`bot_collect_fees`, operacje rebalance, itd.) oraz ewentualnie `cli_swap` / `position_open` w tej samej sesji; sumuj po **`rebalance_session_id`** gdy używasz **`CLMM_REBALANCE_SESSION_ID`** (patrz akapit wyżej).
- **Pełny łańcuch poza repo:** eksplorator (Solscan) po **podpisie** lub po adresie portfela — repo nie zastępuje bloku; dziennik operacyjny (np. `data/experiments/.../OPERATIONS_JOURNAL.md`) może trzymać **tabelę ręczną** (data, stary PDA → nowy PDA, id sesji kosztów, link do tx).
- **Agregacja lokalna:** `clmm-lp-cli ledger-rebalance-summary` — czyta IL JSONL (`--il-ledger` / `CLMM_IL_LEDGER_PATH`) oraz lifecycle JSONL (domyślnie jak wyżej); sumuje `tx_cost_lamports` dla wierszy `rebalance` i grupuje koszty on-chain po `rebalance_session_id`.
- **Dashboard (`clmm-lp-api`):** ustaw **`CLMM_IL_LEDGER_PATH`** na ten sam plik co `--il-ledger-path` bota — endpoint `GET /api/v1/bot-activity/il-ledger` i strony **Bot activity** / szczegół pozycji (**IL ledger**) czytają ten plik.

### Rejestr pozycji (otwarte / zamknięte) dla kolektorów

- Plik **`data/positions/registry.jsonl`** (override: **`CLMM_POSITION_REGISTRY_PATH`**): zdarzenia **`registry_open`** / **`registry_close`** po udanym open/close z CLI lub bota. Kolektory mogą z pliku wyliczyć, które `position_pubkey` są nadal otwarte (ostatni event wygrywa) i **przestać** zbierać dane per pozycja po `registry_close`. Pełny opis: [`doc/POSITION_REGISTRY.md`](POSITION_REGISTRY.md).

## Parametry “bezpieczne” dla Orca (polecane do stabilizacji RPC)
Używaj poniższych domyślnych wartości:
- `--max-decode 40`
- `--decode-timeout-secs 25`
- `--decode-retries 2`
- dla testu skróć: `--max-decode 10`, `--decode-timeout-secs 15`, `--decode-retries 0`

## Krok 0: wybór puli do weryfikacji (opcjonalne, ale zalecane)
1. Wybierz 1–2 adresy `orca` poola (Whirlpool address) do “smoke test”.
2. Dla każdego wybranego poola przygotuj nazwę/ID tak, by łatwo znaleźć folder: `data/pool-snapshots/orca/<pool>/snapshots.jsonl` oraz `data/swaps/orca/<pool>/decoded_swaps.jsonl`.

## Devnet E2E: lifecycle + unsigned tx flow (bot simulation)

Cel: uruchomić testy `#[ignore]` pokrywające:
- lifecycle keypair: `open -> decrease -> collect -> close`,
- flow client-signing: `build unsigned -> sign -> submit`.

Wymagane ENV:
- `SOLANA_RPC_URL` (np. `https://api.devnet.solana.com`)
- `KEYPAIR_PATH` (lub `SOLANA_KEYPAIR_PATH`) z funded devnet wallet
- `DEVNET_POOL_ADDRESS` (domyślnie: `3KBZiL2g8C7tiJ32hTv5v3KM7aK9htpqTw4cTXz1HvPt`)
- opcjonalnie: `DEVNET_TICK_LOWER`, `DEVNET_TICK_UPPER`, `DEVNET_OPEN_AMOUNT_A`, `DEVNET_OPEN_AMOUNT_B`

Uruchomienie:
```powershell
cargo test -p clmm-lp-api devnet_ -- --ignored
```

Minimalna checklista GO/NO-GO:
- GO, gdy:
  - `devnet_pool_state_smoke` przechodzi,
  - `devnet_unsigned_tx_sign_submit_smoke` nie zwraca błędów walidacji wejścia i przechodzi preflight path,
  - `devnet_bot_lifecycle_keypair_smoke` wykonuje co najmniej open + cleanup (`close`) bez panic.
- NO-GO, gdy:
  - brak środków na wallet,
  - symulacja tx consistently zwraca błąd policy gate / simulate,
  - open działa, ale close stale nie domyka pozycji.

### Ważne: `position_address` z API open

- Nie wyliczaj pozycji z `(pool, tick_lower, tick_upper)` po stronie skryptów/bota.
- Dla automatyzacji używaj `position_address` zwróconego przez `/tx/open/build` (to adres pozycji powiązany z realnym `position_mint` wygenerowanym przez SDK).
- Skrypty `tools/bot_run_devnet.ps1` i `tools/bot_session_devnet.ps1` obsługują teraz `-OpenBuildResponseJson`, żeby przekazać ten adres bez ręcznego przepisywania.

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
   - `cargo run --bin clmm-lp-cli -- swaps-enrich-curated-all --limit <N> --max-decode 40 --refresh-decoded --decode-timeout-secs 25 --decode-retries 2 --decode-concurrency 4 --decode-jitter-ms 0`
   - Równoległość: domyślnie `--decode-concurrency 4` (max 32); `CLMM_ENRICH_DECODE_INFLIGHT` nadpisuje, gdy ustawione. Opcjonalnie `--decode-jitter-ms` (losowe 0..N ms przed próbą) lub `CLMM_ENRICH_DECODE_JITTER_MS`.
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
- **Pełna siatka** (cała tabela, bez „top 10”): `--all-rows` (to samo co `--full-ranking`). `top-n` wtedy nie obcina głównej tabeli; sekcje `Max Fees` / `Conservative` / … nadal po kilka wierszy.
- **Skrypt** `scripts/export_optimize_merged_24_48_72_full.ps1` — **domyślnie** te same flagi co powyżej (snapshot path + `fee-source snapshots`) i **`--objective vs-hodl`**. Parametr **`-Hours 24,48`** (lub skrypt **`scripts/export_optimize_merged_24_48_snapshots.ps1`**) — pełna siatka tylko dla **24h i 48h** w **jednym** pliku `optimize_tables_merged_24_48_h.txt`. Parametr **`-Objective fees`** → ranking po **`total_fees`** (opłaty nadal ze snapshotów); wyjście: `optimize_tables_merged_24_48_72_fees.txt`. Skrót: **`scripts/export_optimize_merged_24_48_72_fees_snapshots.ps1`**. Bez pliku `data/pool-snapshots/orca/<POOL>/snapshots.jsonl` skrypt zakończy się błędem; jawny fallback: **`-UseBirdeye`**.

2. Sukces: wyniki są stabilne (nie “skaczą” drastycznie po ponownym runie) i nie ma systematycznych ostrych regresji vs-hodl/pnl.

## Real-world validation log (Orca, whETH/SOL)

Cel: porownac wynik realnych pozycji LP z backtestem dla identycznego okna czasu, kapitalu i zakresu.

### Sesja testowa
- Data: 2026-03-24 (local)
- Godzina startu: 11:00 (local)
- Para: `whETH/SOL`
- Pool: `HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF`
- Kapital testowy: 3 czesci po 100 USD (oddzielne pozycje)
- Uwagi: zakresy sa dostosowane do tickow Orca.

Zakresy otwarte:
- `23.279` do `23.749`
- `23.391` do `23.635`
- `23.447` do `23.560`
- `23.038` do `23.978` (wariant 4% wokol ceny ~23.515; zakres finalny po dostosowaniu do tickow Orca)

Co zapisac po zamknieciu testu (dla kazdej czesci):
- final value (USD), zebrane fees (USD), koszt tx/rebalance (USD),
- timestamp wyjscia (UTC/local),
- backtest uruchomiony na tym samym oknie i tym samym zakresie.

### Porównanie 5m vs 10m dla snapshotów Orca (ostatnia pełna godzina)

W repo jest gotowy skrypt PowerShell, który:
- bierze okno **ostatniej pełnej godziny UTC**,
- wyznacza **entry price** jako pierwszą cenę w oknie (parsuje `Entry Price` z outputu CLI),
- odpala backtesty dla zakresów **±1% / ±2% / ±3%**,
- robi to dla `snapshots_5m.jsonl` (flaga `--snapshot-jsonl-suffix 5m`) i dla domyślnego `snapshots.jsonl`,
- zapisuje CSV i pokazuje szybkie różnice.

Uruchomienie:

```powershell
cargo build --release --bin clmm-lp-cli
pwsh -File .\tools\compare_orca_snapshots_5m_vs_10m_last_full_hour.ps1
```

Opcjonalnie (szybciej): automatycznie wygeneruj cache i użyj `--prepared-snapshot-window`:

```powershell
pwsh -File .\tools\compare_orca_snapshots_5m_vs_10m_last_full_hour.ps1 -RunSnapshotBacktestPrep -PreparedSnapshotWindow h24
```

### Ważne: kapital realny vs kapital planowany

Do porownania z backtestem uzywamy **kapitalu realnie zdeponowanego on-chain**, a nie nominalnego "planowanego" (np. 100 USD).

Definicja robocza (per pozycja):
- `effective_entry_capital` = wartosc tokenow faktycznie wniesionych do LP (A+B),
- uwzglednij net effect rent (`rent fee` wyslane - rent zwrocony),
- `network fee` licz osobno jako koszt transakcyjny (nie jako kapital pozycji).

Praktyka operacyjna:
- Dla kazdej pozycji zapisz z historii tx/contract calls:
  - ile realnie zeszlo `SOL`,
  - ile realnie zeszlo `whETH`,
  - rent out / rent refund,
  - network fee.
- Przelicz `SOL` i `whETH` do USD na timestamp wejscia i policz `effective_entry_capital`.
- Do backtestu podstawiaj ten `effective_entry_capital`, a nie "100 USD".

Checklist dla wszystkich aktywnych zakresow:
- [ ] `23.279` - `23.749`: policzony `effective_entry_capital`
- [ ] `23.391` - `23.635`: policzony `effective_entry_capital`
- [ ] `23.447` - `23.560`: policzony `effective_entry_capital`
- [ ] `23.038` - `23.978`: policzony `effective_entry_capital`

## Snapshot-only porównanie Orca / Raydium / Meteora (SOL/USD)
Cel: porównać wyniki backtest-optimize pomiędzy protokołami bez `Dune` i bez `Birdeye` (oraz bez Dexscreener na USD price). Opieramy się wyłącznie o informacje z lokalnych `snapshots.jsonl` (oraz ewentualnie ich dekodowanie z `data_b64`).

### Co jest teraz gotowe
- `backtest-optimize --price-path-source snapshots` i `--fee-source snapshots` działa jako “snapshot-only” dla **Orca** oraz **Raydium**.
- `backtest-optimize --price-path-source snapshots` i `--fee-source snapshots` działa również dla **Meteory** jako snapshot-only proxy:
  - cena: z formuły DLMM dla `active_id` i `bin_step` (proxy “spot”),
  - opłaty: z delty `protocol_fee_amount_{x,y}` między snapshotami (proxy fee-accrual na poziomie pary).
  - `lp_share`: jeśli snapshot ma **`vault_amount_a` / `vault_amount_b`** (standardowy zapis z `meteora-snapshot-curated`), wyliczamy udział jak Raydium z TVL (`capital / tvl_usd`). **`--lp-share`** jest nadal opcjonalnym override; wymagane tylko gdy w pliku brak sald rezerw (stare snapshoty).

### Czego brakuje (do implementacji)
1. Ceny z snapshotów dla pozostałych protokołów
   - Raydium: zrobione — liczymy `price_ab` z `sqrt_price_x64/tick_current` i per-step fees z delta `fee_growth_global*_x64` + `liquidity_active`.
   - Meteora: zrobione częściowo — mamy proxy price z `active_id/bin_step`, ale nadal brakuje pełnego “bin-by-bin” neighbourhood modelu (bin arrays / fee-growth w binach), więc to jest przybliżenie.

2. Fee-model “snapshot-only” dla Raydium i Meteory
   - Raydium: zrobione — per-step fees są liczone z delta `fee_growth_global*_x64` (fallback na `protocol_fees_token*`).
   - Meteora: zrobione jako proxy — per-step fees z delty `protocol_fee_amount_{x,y}` (poziom pary), bez bin-level fee-growth.

3. Eliminacja zewnętrznych serwisów do USD
   - Aktualny kod Orca snapshot-to-USD używa Dexscreener do wyznaczenia `quote_usd`.
   - Dla porównania `SOL/USDC` i `SOL/USDT` można całkowicie usunąć Dexscreener przez regułę: `USDC=1.0 USD`, `USDT=1.0 USD`.
   - Zrobione dla `USDC/USDT`: snapshot-only nie wykonuje wywołań do Dexscreenera, bo `quote_usd` jest mapowane do `1.0` z samego mintu.

### Plan działania (snapshot-only)
1. Raydium
   - Zrobione: `backtest-optimize` wspiera `--snapshot-protocol raydium --price-path-source snapshots --fee-source snapshots`.

2. Meteora
   - Ustalić, jakie dane posiadamy w snapshotach:
     - jeśli nie mamy “bin prices” i “bin fee-growth”, trzeba rozszerzyć snapshot pipeline o bin arrays (snapshot neighbourhood wokół `active_id`) — to nadal jest “docelowo dokładne” zamiast proxy.
   - Zaimplementować `build_from_meteora_snapshots(...)`:
     - dekodować `lb_pair` z `data_b64` (zrobione),
     - proxy price i fees są już zaimplementowane — ale bin-level precision jest jeszcze do dopięcia.
   - Wpiąć w `backtest-optimize` dla `--snapshot-protocol meteora`.

3. USDC/USDT bez zewnętrznych USD oracle
   - Dodać mapowanie `quote_usd`:
     - `USDC (EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v)` -> `1.0`
     - `USDT (Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB)` -> `1.0`
   - Jeśli token B nie jest USDC/USDT: tryb “snapshot-only” ma z failować (żeby nie produkować senseless wyników).

### Kryteria akceptacji porównania “Orca vs Raydium vs Meteora”
- Uruchomienie `backtest-optimize` dla wszystkich 3 protokołów z:
  - `--price-path-source snapshots`
  - `--fee-source snapshots`
  - bez `--dune-swaps`
  - bez `BIRDEYE_API_KEY`
- Powtarzalność: wyniki dla tego samego okna czasowego i zakresu nie powinny “skakać” od różnic w mapowaniu USD.

## Koszty Rebalansów (gdzie to się mapuje w projekcie)
- W backtest/optimize koszt rebalansu jest liczone jako: `fixed(network+priority+jito+tx_cost) + slippage_bps * rebalanced_notional`.
- Dopóki nie mamy kalibracji parametrów z realnych obserwacji (priority fee, jito tip, efektywny slippage), traktuj `--network-fee-usd/--priority-fee-usd/--jito-tip-usd/--slippage-bps` jako “modelowe” priorytety i dostrajaj je testowo na Orca.
- W roadmap `STARTUP.md` plan zakłada “Hardening and risk controls”: limity na gas/priority fees oraz max rebalance frequency w docelowym b0cie.

### Ustawianie strategii rebalance (periodic vs threshold)
Dla komendy `backtest` (nie `backtest-optimize`) strategię rebalance wybierasz flagą `--strategy`:
- `--strategy periodic --rebalance_interval <HOURS>` (periodic co N godzin)
- `--strategy threshold --threshold_pct <PCT>` (threshold gdy cena przekroczy próg)
- `--strategy il_limit` (IL-aware rebalance/close logic w warstwie strategii)

**Pełny opis strategii siatki `backtest-optimize`** (w tym `oor_recenter`, różnice `threshold` vs OOR-only, `retouch_shift`, planowane rozszerzenia):  
→ **`doc/BACKTEST_OPTIMIZE_STRATEGIES.md`**

Skrót: domyślna siatka obejmuje m.in. `static`, **`oor_recenter`**, `threshold` (wiele progów %), `periodic` (12/24/48/72 h), `il_limit`, `retouch_shift`.

Parametry IL-limit dla gridu:
- `--il-max-pct <PCT>` (domyślnie 5),
- `--il-close-pct <PCT>` (opcjonalny próg zamknięcia),
- `--il-grace-steps <N>` (domyślnie 0).

**RetouchShift (hybryda czas + %)** — gdy nadal jesteś poza zakresem po pierwszym retouchu, kolejne retouchy są dozwolone po min. `--retouch-repeat-cooldown-secs`, jeśli minęło `--retouch-repeat-rearm-secs` od ostatniego retouchu *albo* cena oddaliła się o co najmniej `--retouch-repeat-extra-move-pct` w złym kierunku względem ceny z ostatniego retouchu. **Domyślne parametry startowe:** próg ruchu **0,3%** (`--retouch-repeat-extra-move-pct 0.003`) oraz **1 h** (`--retouch-repeat-rearm-secs 3600`); cooldown między retouchami domyślnie **300 s** (żeby próg % mógł zadziałać przed upływem godziny). Wyłączenie hybrydy (stare: jeden retouch na epizod OOR aż do powrotu w range): `--retouch-repeat-off`.

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

## Skrót: jeden komendowy “ops cycle” (A1→A4)

Jeśli chcesz wykonać **snapshoty → sync swapów → enrich (decode) → audit → health-check** jednym uruchomieniem,
użyj w repo gotowego skryptu PowerShell:

```powershell
.\scripts\run_ops_ingest_cycle.ps1 -Limit 2 -RefreshDecoded -SolanaRpcUrl "https://api.mainnet-beta.solana.com"
```

Zalecane:
- ustaw `SOLANA_RPC_FALLBACK_URLS` (comma-separated), jeśli masz więcej endpointów,
- zacznij od małego `-Limit` i dopiero potem zwiększaj.

## Szybki verifier GO/NO-GO (snapshot + decode + health)

Jedna komenda do szybkiego sprawdzenia:
- readiness tier 1/2 dla curated pooli,
- health alerts (`data-health-check`),
- globalny `% ok` dekodowania (`swaps-decode-audit`).

```powershell
.\tools\quick_verify_data.ps1 -LimitPerProtocol 1 -MinDecodeOkPct 65 -HealthMaxAgeMinutes 180
```

Wynik:
- zapisuje raport JSON: `data/reports/quick_verify_*.json`,
- zwraca `OVERALL GO: True/False`,
- kończy kodem `2` jeśli GO=FALSE (pod scheduler/CI).

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

## Devnet MVP Functional Tests (obowiazkowe przed mainnet)

To jest twarde minimum akceptacyjne: przed jakimkolwiek wdrozeniem na mainnet trzeba zaliczyc funkcjonalne testy E2E na devnecie dla sciezek Orca.

Dlaczego:
- implementacja `WhirlpoolExecutor` moze miec niepelne account metas dla niektorych instrukcji,
- `decrease_liquidity` jest aktualnie wysylane z `token_min_a=0` i `token_min_b=0` (wysoka tolerancja slippage),
- tylko realny przebieg tx na devnecie potwierdza komplet kont i poprawna kolejnosc instrukcji.

### Minimalny scope MVP (devnet)
- [ ] `orca-position-open` dry-run i execute dla poprawnego zakresu tickow.
- [ ] `orca-position-open` odrzuca niepoprawny zakres (`tick_lower >= tick_upper`, zly tick spacing).
- [ ] `orca-position-decrease --liquidity-pct` i `--liquidity` dziala end-to-end (tx + odczyt nowej liquidity).
- [ ] Sciezka API (jesli uzywana operacyjnie): endpoint decrease/open przechodzi walidacje i zwraca czytelne bledy.
- [ ] Co najmniej jeden test negatywny tx (celowo bledne parametry) zwraca blad, a nie cichy sukces.

### Zasada GO/NO-GO
- **GO do mainnet tylko gdy** wszystkie testy MVP na devnecie sa zielone i zarchiwizowane w worklogu.
- **NO-GO** gdy jakikolwiek test E2E na devnecie jest niestabilny, flaky lub nieprzechodzacy.

## CLI: `orca-bot-run` (monitor + strategia bez API)

Ta sama pętla co `clmm-lp-api` (`PositionMonitor` + `StrategyExecutor`), uruchamiana z linii poleceń.

- **Domyślnie dry-run:** tylko logi decyzji (bez podpisywania). Użyj `--execute`, żeby wysyłać transakcje — wtedy potrzebny jest **`--keypair <plik.json>`** albo zmienna **`SOLANA_KEYPAIR`**.
- **RPC:** `SOLANA_RPC_URL` / `SOLANA_RPC_FALLBACK_URLS` (jak w reszcie repo).
- **`--position`:** adres **pozycji** Whirlpool (NFT), nie adres poola.
- **Opcjonalnie `--optimize-result-json`:** plik wyniku `backtest-optimize` → `DecisionConfig` (jak `POST /apply-optimize-result` w API).
- **Historia pod backtest:** `--il-ledger-path` (JSONL: open / rebalance / close) oraz `--position-fee-ledger-path` (JSONL: checkpointy fee). CLI tworzy katalog nadrzędny pliku, jeśli nie istnieje. Konwencja i join z symulacją: `doc/BOT_OPERATIONS_MODEL_2026-03-23.md`.
- **Ciągłość operacyjna (restart po crashu, systemd, Docker, logi):** [`doc/OPERATIONAL_CONTINUITY.md`](OPERATIONAL_CONTINUITY.md); Windows: [`tools/orca_bot_run_supervised.ps1`](../tools/orca_bot_run_supervised.ps1).

Przykład (tylko obserwacja):

```powershell
cargo run --bin clmm-lp-cli -- orca-bot-run --position <POSITION_PUBKEY> --eval-interval-secs 300 --poll-interval-secs 30
```

Przykład z ledgerami (np. pod późniejsze porównanie z backtestem):

```powershell
cargo run --bin clmm-lp-cli -- orca-bot-run `
  --position <POSITION_PUBKEY> `
  --eval-interval-secs 300 `
  --poll-interval-secs 30 `
  --il-ledger-path data/bot-runs/devnet/run-il.jsonl `
  --position-fee-ledger-path data/bot-runs/devnet/run-fee.jsonl
```

Skrypt `tools/bot_run_devnet.ps1` domyślnie zakłada katalog `data/bot-runs/devnet/<timestamp>/` i dopisuje oba pliki (`il_ledger.jsonl`, `position_fee_ledger.jsonl`), chyba że podasz `-BotRunDir`, `-IlLedgerPath`, `-PositionFeeLedgerPath` albo wyłączysz zapis: `-SkipLedger`.

## CLI: `orca-bot-open-and-run` (open + monitor + strategia)

Jedna komenda do flow devnet: otwórz pozycję on-chain i od razu uruchom pętlę bota na nowym `position_address` (bez ręcznego przepisywania adresu).

Przykład (otwarcie po szerokości + bot dry-run):

```powershell
cargo run --bin clmm-lp-cli -- orca-bot-open-and-run `
  --pool <POOL_PUBKEY> `
  --range-width-pct 10 `
  --keypair "C:\secure\devnet-bot\wallet.json" `
  --eval-interval-secs 300 `
  --poll-interval-secs 30
```

Przykład (open + bot limited-live):

```powershell
cargo run --bin clmm-lp-cli -- orca-bot-open-and-run `
  --pool <POOL_PUBKEY> `
  --tick-lower <TICK_LOWER> `
  --tick-upper <TICK_UPPER> `
  --keypair "C:\secure\devnet-bot\wallet.json" `
  --execute
```

## CLI: Zamykanie pozycji / pobieranie fee

Collect fees:

```powershell
cargo run --bin clmm-lp-cli -- orca-position-collect-fees `
  --position <POSITION_PUBKEY> `
  --keypair "C:\secure\devnet-bot\wallet.json"
```

Harvest (alias, Orca naming):

```powershell
cargo run --bin clmm-lp-cli -- orca-position-harvest `
  --position <POSITION_PUBKEY> `
  --keypair "C:\secure\devnet-bot\wallet.json"
```

Close position (full close):

```powershell
cargo run --bin clmm-lp-cli -- orca-position-close `
  --position <POSITION_PUBKEY> `
  --keypair "C:\secure\devnet-bot\wallet.json"
```

### Devnet convenience: open -> wait -> close

```powershell
cargo run --bin clmm-lp-cli -- orca-position-open-and-close `
  --pool 3KBZiL2g8C7tiJ32hTv5v3KM7aK9htpqTw4cTXz1HvPt `
  --range-width-pct 10 `
  --sleep-secs 10 `
  --keypair "C:\secure\devnet-bot\wallet.json"
```

## Devnet: swap SOL -> (dev)USDC w CLI (do zasilenia tokenów pod open/rebalance)

Jeśli masz SOL, ale brakuje Ci (dev)USDC do otwarcia pozycji SOL/USDC, możesz zrobić swap na tej samej puli Whirlpool.

Przykład (ExactIn): zamień 0.01 SOL (\(0.01 * 1e9 = 10_000_000\) lamports) na devUSDC, ze slippage 1%.

```powershell
cargo run --bin clmm-lp-cli -- orca-swap `
  --pool 3KBZiL2g8C7tiJ32hTv5v3KM7aK9htpqTw4cTXz1HvPt `
  --specified-mint So11111111111111111111111111111111111111112 `
  --swap-type exact-in `
  --amount 10000000 `
  --slippage-bps 100 `
  --keypair "C:\secure\devnet-bot\wallet.json"
```

## Devnet: automatyczne wyrównanie portfela do ~50/50 (SOL vs devUSDC)

Skrypt policzy ile SOL doswapować do devUSDC na podstawie quote z puli i aktualnych sald, a potem wykona swap jednym strzałem.

```powershell
.\tools\devnet_rebalance_wallet_half.ps1 -KeypairWinPath "C:\secure\devnet-bot\wallet.json"
```

Dry-run (bez wysyłania transakcji):

```powershell
.\tools\devnet_rebalance_wallet_half.ps1 -KeypairWinPath "C:\secure\devnet-bot\wallet.json" -DryRun
```

### Tryb “pozycja”: close -> 50/50 -> open

Jeśli chcesz zrebalansować **tylko środki z konkretnej pozycji**, skrypt może:
1) zamknąć pozycję,
2) wyrównać portfel do ~50/50,
3) otworzyć nową pozycję na wskazanym poolu (minimalne capy pod testy).

```powershell
.\tools\devnet_rebalance_wallet_half.ps1 `
  -KeypairWinPath "C:\secure\devnet-bot\wallet.json" `
  -Position <POSITION_PUBKEY> `
  -ReopenPool 3KBZiL2g8C7tiJ32hTv5v3KM7aK9htpqTw4cTXz1HvPt `
  -ReopenRangeWidthPct 10 `
  -OpenAmountA 1000000 `
  -OpenAmountB 1000
```

Semantyka strategii i benchmarku HODL: `doc/BACKTEST_OPTIMIZE_STRATEGIES.md`.

