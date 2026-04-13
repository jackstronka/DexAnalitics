# BUGS Registry

Purpose: durable, searchable bug memory for AI/humans across sessions/models.

Rules for entries:
- Keep newest entries at the top.
- Use stable ids: `BUG-YYYYMMDD-XX`.
- Include `keywords:` for grep/AI recall.
- Record both: what was wrong and how it was fixed.
- Mark status: `open`, `fixed`, `regressed`, `wontfix`.

## Entry Template

### BUG-YYYYMMDD-XX — Short title

status: open|fixed|regressed|wontfix  
severity: low|medium|high|critical  
reported_by: user|ai|monitoring  
first_seen: YYYY-MM-DD  
fixed_in: <commit-sha-or-branch-or-empty>  
keywords: comma,separated,tokens,for,search

- **Symptom:** user-visible behavior.
- **Root cause:** technical reason.
- **Fix:** what changed.
- **Guards/tests:** checks to prevent recurrence.
- **Paths:** `file/a`, `file/b`

---

### BUG-20260413-06 — Stan portfela na „nowa pozycja” bywa nieaktualny / trzeba odświeżyć

status: open  
severity: medium  
reported_by: user  
first_seen: 2026-04-13  
fixed_in:   
keywords: position-create, wallet-balances, api-signer, stale-data, react-query, race, owner-pubkey

- **Symptom:** Przy wejściu na flow otwarcia nowej pozycji odczyt „Stan portfela” bywa niepełny lub wygląda na opóźniony względem tego, co widać na stronie Wallet; czasem dopiero pełne odświeżenie strony pokazuje oczekiwane salda.
- **Root cause (analiza kodu):** W `PositionCreate` `effectiveOwnerPk` jest liczone jako `apiSigner.pubkey ?? ownerPk` (`ownerPk` = wybrany portfel z listy keypairów w localStorage). Dopóki zapytanie `GET /wallets/api-signer` się nie zakończy, używane jest **tymczasowo** `ownerPk`, więc `useQuery(['wallet-balances', effectiveOwnerPk])` może najpierw pobrać salda **innego** klucza niż ten, którym realnie podpisuje open (API signer). Po dojściu odpowiedzi api-signer klucz query się zmienia i następuje drugi fetch — użytkownik może złapać „złą” chwilę UI lub porównywać z Wallet (tam **zawsze** `ownerPk`, bez api-signer). Dodatkowo `staleTime: 20_000` i globalne `refetchOnWindowFocus: false` sprzyjają pokazywaniu cache bez natychmiastowego ponownego odczytu przy nawigacji.
- **Fix:** Do wdrożenia: np. nie włączać `wallet-balances` na PositionCreate dopóki `api-signer` nie jest `isFetched` (gdy signer jest skonfigurowany), albo jedno zapytanie łączone; ewent. `refetchOnMount: 'always'` / krótszy `staleTime` tylko dla tej strony; jasny komunikat „ładowanie portfela podpisującego…” zamiast chwilowych sald z niewłaściwego ownera.
- **Guards/tests:** Test E2E lub jednostkowy: kolejność rozwiązań query (wallets vs api-signer) nie powinna wyświetlać sald dla złego pubkey; regresja: po swap/open invalidate trafia w ten sam `effectiveOwnerPk`.
- **Paths:** `web/src/pages/PositionCreate.tsx` (`effectiveOwnerPk`, `apiSignerQ`, `effectiveBalancesQ`), `web/src/pages/Wallet.tsx` (porównanie: samo `ownerPk`), `web/src/main.tsx` (domyślne opcje React Query), `crates/api/src/handlers/wallets.rs` (`GET /wallets/api-signer`)

### BUG-20260413-07 — Performance „Value” vs Position history (start/end): duża rozbieżność (~2×) na otwartej pozycji

status: open  
severity: medium  
reported_by: user  
first_seen: 2026-04-13  
fixed_in:   
keywords: position-detail, performance, stream-lineage, value-usd, snapshots, valuation-drift, UI-consistency

- **Symptom:** Na `/positions/<PDA>` (otwarta pozycja) karta **Performance → Value** pokazuje np. **~$3.88**, a w **Position history (rotations)** kolumny **start value** i **end value** obie np. **~$1.80** z etykietą `exact` — użytkownik widzi pozornie „dwukrotnie mniej” w tabeli niż w Performance.
- **Root cause (analiza kodu):** Ta metryka „wartość USD” jest liczona w **dwóch miejscach** (nie jest jednym cache): karta Performance vs wiersz lineage. Karta Performance bierze `value_usd` z `GET /positions/{address}`: świeże `compute_position_usd_valuation` na stanie z monitora/RPC (`handlers/positions.rs`). Tabela lineage w `GET /positions/{address}/stream-lineage` dla węzła w ścieżce DB w `node_metrics` ustawia **baseline** i **current** z tabeli `position_stream_valuation_snapshots` (pierwszy vs ostatni wiersz wg sortowania SQL), a niekoniecznie z tą samą chwilą ani tą samą logiką co bieżąca karta. Dla **świeżo otwartej** pozycji często jest **jeden** (lub kilka) snapshotów — **start i end mogą wskazywać ten sam zapis**, z wartością zapisaną wcześniej (inny moment/ceny/jedna noga w delcie) podczas gdy karta już pokazuje nowszą wycenę. Etykieta `exact` w UI pochodzi z `raw_json.valuation_quality` snapshotu lub z ścieżki lifecycle — opisuje **jakość wejścia cen**, nie gwarancję zgodności z kartą Performance.
- **Fix:** Do wdrożenia: dla węzłów **otwartych** (`closed_ts_utc` puste) ustawiać `current_value_usd` (ew. spójnie `baseline` przy braku sensownego open snapshot) tą samą ścieżką co `get_position` / `compute_position_usd_valuation`, albo w UI wyraźnie oznaczyć tabelę jako „wartości ze snapshotów ledgera” vs „bieżąca wartość” w Performance; opcjonalnie dopisywać nowy snapshot przy każdym `get_position` i używać **najnowszego** wiersza jako `current` dla aktywnej pozycji.
- **Guards/tests:** Test regresyjny: mock DB z jednym snapshotem o wartości X i mock on-chain valuation Y — lineage dla otwartej pozycji nie powinien pokazywać obu kolumn jako X gdy API detail zwraca Y (po fixie); albo snapshot current nadpisany live.
- **Paths:** `web/src/pages/PositionDetail.tsx` (Performance vs tabela lineage), `crates/api/src/handlers/positions.rs` (`get_position`, zapis snapshotów), `crates/api/src/services/position_stream_lineage.rs` (`node_metrics`, zapytania `position_stream_valuation_snapshots`), `crates/api/src/services/position_valuation.rs`

### BUG-20260413-05 — Stream lineage chained manual opens to unrelated rotation history

status: fixed  
severity: medium  
reported_by: user  
first_seen: 2026-04-13  
fixed_in: local  
keywords: position-stream-lineage, rotation, registry.jsonl, lifecycle, rebalance_session_id, false-parent, manual-open

- **Symptom:** A newly opened position in the same pool as prior activity appeared in **Position history (rotations)** as continuing an old PDA chain instead of a standalone node.
- **Root cause:** Registry fallback linked `registry_open` → `registry_close` by time/pool/owner even when `rebalance_session_id` did not match (or was empty). Lifecycle fallback linked opens to the latest close in a short window without requiring rotation evidence; forward close→open used the first qualifying open, not the true successor; lifecycle parent inference treated loose swap rows as rotation.
- **Root cause (runtime `debug-c45ac3.log`, H-lineage):** With DB enabled, `db_chain_from_edges_len` stayed **1** while JSONL fallback inflated `chain_len` to **3–4** — `build_linear_chain` walked from a single graph root and **omitted `entry` on forked `position_stream_edges`** (`A→B` and `A→C`), fell back to `[entry]`, then registry/lifecycle re-stitched a long pool history.
- **Fix:** Registry parent/chain only when both rows carry the same non-empty `rebalance_session_id`. Lifecycle uses `lifecycle_rotation_parent_before_open` (session match, `close_kind=rotation`, or bot activity tied to the closed PDA); forward links require that helper to return the closed PDA; `infer_parent_position_from_lifecycle_best_effort` delegates to the same helper. DB mode uses **`build_lineage_chain_from_db_edges`** (backward from `entry`, then forward); **JSONL/registry fallback is skipped when any persisted edge touches `entry`**. **JSONL/registry fallback also requires `lifecycle_open_has_prior_close_same_session`** (latest open’s `rebalance_session_id` must match a prior close in the same pool/payer within 60m) so UI `bot_open_position` + fresh `cost_session_id` does not inherit unrelated pool history; `position_open` (CLI) still suppresses stitching.
- **Guards/tests:** Unit tests `lifecycle_chain_*`, `db_edges_*`, `jsonl_stitch_*`.
- **Paths:** `crates/api/src/services/position_stream_lineage.rs`

### BUG-20260413-04 — Open target USD looked too low after success

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-13  
fixed_in: local  
keywords: open-position, target-usd, valuation, wsol, usdc, price-source, drift

- **Symptom:** User sets open target (e.g. `5 USD`), position opens successfully, but displayed post-open value appears noticeably lower.
- **Root cause:** Runtime logs showed quote/open were near target, but later UI value used a lower external SOL/USD feed than pool-implied SOL/USD for the same WSOL/USDC pool state. This created an avoidable valuation drift in display value.
- **Fix:** For WSOL/USDC valuation path, backend now prefers SOL/USD implied from the pool tick (same on-chain state used for token amounts) instead of external feed-only pricing.
- **Guards/tests:** Verified on user reproduction after fix; instrumentation removed post-confirmation.
- **Paths:** `crates/api/src/services/position_valuation.rs`, `crates/api/src/handlers/pools.rs`, `crates/api/src/services/position_service.rs`, `crates/api/src/handlers/positions.rs`

### BUG-20260413-03 — Close Position fails with Whirlpool custom 6018

status: open  
severity: high  
reported_by: user  
first_seen: 2026-04-13  
fixed_in:   
keywords: close-position, whirlpool, custom-6018, tokenminsubceeded, slippage, min-out

- **Symptom:** `Close Position failed: ... InstructionError(2, Custom(6018)) ... TokenMinSubceeded` on `whirLb...` even for manual close flow.
- **Root cause:** Unknown yet (hypotheses in progress): min-out/slippage too tight for close instruction vs price move at execution time, retry path may not be sufficient/visible, or wrong effective slippage passed to close.
- **Fix:** In progress (runtime instrumentation added to close path to log effective slippage, attempt count, and exact fail branch before next fix).
- **Guards/tests:** TODO after fix: regression test for close retry on 6018 and explicit user-facing hint with effective slippage/attempts.
- **Paths:** `crates/protocols/src/orca/executor.rs`, `crates/api/src/services/position_service.rs`

### BUG-20260413-02 — PositionCreate nie sugerował swapu na operacyjny SOL

status: partially fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-13  
fixed_in: local  
keywords: position-create, swap-suggestion, operational-sol, rent, fees, jupiter

- **Symptom:** UI nie proponował `swap to SOL` przed `Open Position`, a backend zwracał błąd `API wallet has insufficient SOL for rent/fees`.
- **Symptom:** API potrafiło zwrócić `Top up both native SOL balance and wrapped-SOL token amount` mimo że UI nie blokował formularza (WSOL noga wyglądała na pokrytą).
- **Symptom:** `Open position failed: SPL Token program error InsufficientFunds` z logiem `Transfer: insufficient lamports ..., need ...` (Instruction 2) mimo wcześniejszego `Swap potwierdzony`.
- **Symptom:** Kolejne przypadki `InsufficientFunds` (Instruction 0) mimo że UI pokazywał wystarczające „Stan portfela”.
- **Symptom:** Nadal pojawiał się błąd `Transfer: insufficient lamports 24922302, need 33006413` (Instruction 0) nawet po wcześniejszych poprawkach UI.
- **Symptom:** Preflight bywa blokowany mimo wystarczającego `current_wsol` (tokenowo), bo native guard dla WSOL open działał w trybie konserwatywnym `need + pad`.
- **Root cause:** `fundingCheck` w `PositionCreate` sprawdzał tylko deficyty tokenów A/B dla notionalu pozycji i dla WSOL traktował dostępne saldo jako `native SOL + WSOL token`. Brakowało rozdzielenia wymagań: (1) WSOL token na nogę pozycji oraz (2) native SOL >= `min_open_lamports` na rent/fee buffer.
- **Root cause:** Dodatkowo brakowało doliczenia kosztu utworzenia konta WSOL (ATA rent) przy pierwszym użyciu WSOL, więc projekcja native SOL mogła być zaniżona względem realnego `SystemProgram::Transfer`.
- **Root cause:** UI walidował salda na podstawie lokalnie wybranego walleta (`/wallets/balances?owner=...`), podczas gdy `open/swap` są wykonywane przez API signer wallet z backendu. Przy rozjechanych portfelach frontend przepuszczał formularz mimo realnego braku środków na signerze.
- **Root cause:** W `WhirlpoolExecutor::preflight_open_liquidity_balances` pad operacyjny dla open był zbyt niski (`2_500_000` lamportów), więc preflight mógł przepuścić przypadki, które później padały na `SystemProgram::Transfer` po stronie tx build/send.
- **Fix:** Dodano check `shortOperationalSol` oparty o projekcję native SOL po finansowaniu nogi WSOL oraz `GET /wallets/api-signer (min_open_lamports)`. Dodatkowo `getAvailableUiAmount` liczy teraz saldo tokenowe (bez native), więc deficyt nogi WSOL jest wykrywany zgodnie z walidacją backendu. Projekcja odejmuje też estymatę rent dla WSOL ATA, gdy konto WSOL jeszcze nie istnieje. `blocked` uwzględnia oba warunki, a UI pokazuje dedykowany przycisk `Jupiter: swap to SOL`.
- **Fix:** `PositionCreate` przełączył źródło sald na API signer pubkey (z `/wallets/api-signer`) jako główny owner dla `wallet-balances`, walidacji fundingu i odświeżania po swapie. Dodano też notkę UI, że walidacja używa portfela API signer.
- **Fix:** Na podstawie historycznych openów z `data/ledger/orca_position_lifecycle.jsonl` (13 próbek: p50 ~10.08M, p95 ~10.48M, max ~11.07M lamportów) ustawiono domyślny `CLMM_MIN_OPEN_SOL_LAMPORTS` na `12_000_000` (0.012 SOL) jako bufor operacyjny. Wyższe wymagania SOL wynikające z notionalu nogi WSOL są walidowane oddzielnie.
- **Fix:** `WhirlpoolExecutor` używa teraz wspólnego bufora `open_native_sol_pad_lamports()` (env `CLMM_MIN_OPEN_SOL_LAMPORTS`, default `12_000_000`) zamiast sztywnego `2_500_000` w preflight WSOL/open; ten sam bufor jest też używany przy clampowaniu swapu WSOL.
- **Fix:** Usunięto blokujący heurystyczny bail native-SOL na etapie `preflight_open_liquidity_balances` dla nóg WSOL. Zamiast tego przed wysyłką tx wykonywana jest symulacja finalnego planu instrukcji (`simulate_transaction`) i parsowany jest rzeczywisty log runtime `Transfer: insufficient lamports X, need Y`; guard używa teraz `need` z symulacji + margines 1% (`ceil(need*1.01)`).
- **Guards/tests:** `npx tsc --noEmit` w `web/` przechodzi. TODO: test UI regresyjny dla scenariusza „A/B OK, ale operacyjny SOL za niski”.
- **Guards/tests:** `cargo check -p clmm-lp-protocols` przechodzi po zmianie prechecka na exact-plan + 1% margin.
- **Paths:** `web/src/pages/PositionCreate.tsx`, `crates/protocols/src/orca/executor.rs`

### BUG-20260413-01 — Manual close could trigger unintended reopen

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-13  
fixed_in: local  
keywords: manual-close, stale-snapshot, strategy-executor, evaluate_position, reopen, race

- **Symptom:** Po ręcznym `Close Position` chwilę później powstawała nowa pozycja, mimo że użytkownik nie uruchamiał rebalance.
- **Root cause:** `StrategyExecutor::evaluate_position` po `refresh_position` fallbackował do starego cache. Gdy monitor usunął już zamkniętą pozycję, ewaluator nadal mógł wykonać decyzję na starym snapshotcie.
- **Fix:** W `evaluate_position` usunięto fallback do stale cache dla ścieżki po refresh. Teraz:
  - przy błędzie refresh: skip cyklu (bez akcji),
  - gdy po refresh pozycji nie ma już w monitorze: skip cyklu (bez akcji).
- **Guards/tests:** TODO dodać test regresyjny race-case: manual close w trakcie pętli `evaluate_all` nie może wywołać `Rebalance`/`open`.
- **Paths:** `crates/execution/src/strategy/executor.rs`

### BUG-20260410-06 — Rotacja zamyka pozycję bez otwarcia nowej (urwany rebalance flow)

status: open  
severity: high  
reported_by: user  
first_seen: 2026-04-10  
fixed_in:   
keywords: rebalance, close_without_open, swap_mix, recovery, strategy

- **Symptom:** Pozycja zamknięta jako `close_kind=rotation`, ale brak kolejnego `bot_open_position` dla tej sesji/łańcucha.
- **Root cause:** Rebalance flow urwał się po close + etapach swap (`bot_swap_mix_round` / `bot_swap_exact_in_attempt`) bez finalnego open; brak jawnego `rebalance_incomplete` wpisu dla tego przypadku.
- **Fix:** Do wdrożenia: twardy marker `rebalance_incomplete` + trwały `pending-open` recovery gdy close zakończony, a open nie doszedł do skutku; UI powinno pokazywać taki status zamiast "po prostu closed".
- **Guards/tests:** test scenariusza: close success + swap rounds + open failure/abort => wpis `rebalance_incomplete` + recovery artifact.
- **Paths:** `data/ledger/orca_position_lifecycle.jsonl`, `crates/execution/src/strategy/rebalance.rs`, `crates/api/src/handlers/positions.rs`

### BUG-20260410-05 — Collect Fees: brak executora mimo aktywnego środowiska

status: partially fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-10  
fixed_in: local (pending user verification)  
keywords: collect_fees, executor, wallet, KEYPAIR_PATH, SOLANA_KEYPAIR, WALLET_KEYPAIR_BASE58, fee_owed, position-reader

- **Symptom:** (1) `Collect Fees failed: Service unavailable: Fee collection requires executor and wallet configuration`; (2) po naprawie wallet: `Collect Fees failed: Internal error: collect_fees: read position for fee_owed`.
- **Symptom:** (1) `Collect Fees failed: Service unavailable: Fee collection requires executor and wallet configuration`; (2) po naprawie wallet: `Collect Fees failed: Internal error: collect_fees: read position for fee_owed`; (3) po sukcesie collect brak informacji o kwocie/tokenach zebranych fee.
- **Symptom:** (1) `Collect Fees failed: Service unavailable: Fee collection requires executor and wallet configuration`; (2) po naprawie wallet: `Collect Fees failed: Internal error: collect_fees: read position for fee_owed`; (3) po sukcesie collect brak informacji o kwocie/tokenach zebranych fee; (4) w lineage widoczne `collect 1x` oraz `A=0/B=0` bez jasnej informacji czy to błąd czy faktycznie zero owed.
- **Symptom:** (1) `Collect Fees failed: Service unavailable: Fee collection requires executor and wallet configuration`; (2) po naprawie wallet: `Collect Fees failed: Internal error: collect_fees: read position for fee_owed`; (3) po sukcesie collect brak informacji o kwocie/tokenach zebranych fee; (4) w lineage widoczne `collect 1x` oraz `A=0/B=0` bez jasnej informacji czy to błąd czy faktycznie zero owed; (5) `LP Zebrane`: jedna noga miała wartość, druga pokazywała `-`.
- **Symptom:** (1) `Collect Fees failed: Service unavailable: Fee collection requires executor and wallet configuration`; (2) po naprawie wallet: `Collect Fees failed: Internal error: collect_fees: read position for fee_owed`; (3) po sukcesie collect brak informacji o kwocie/tokenach zebranych fee; (4) w lineage widoczne `collect 1x` oraz `A=0/B=0` bez jasnej informacji czy to błąd czy faktycznie zero owed; (5) `LP Zebrane`: jedna noga miała wartość, druga pokazywała `-`; (6) brak szybkiego wyjaśnienia „dlaczego 0” w UI.
- **Symptom:** (1) `Collect Fees failed: Service unavailable: Fee collection requires executor and wallet configuration`; (2) po naprawie wallet: `Collect Fees failed: Internal error: collect_fees: read position for fee_owed`; (3) po sukcesie collect brak informacji o kwocie/tokenach zebranych fee; (4) w lineage widoczne `collect 1x` oraz `A=0/B=0` bez jasnej informacji czy to błąd czy faktycznie zero owed; (5) `LP Zebrane`: jedna noga miała wartość, druga pokazywała `-`; (6) brak szybkiego wyjaśnienia „dlaczego 0” w UI; (7) komunikat collect bez czytelnego formatu oraz brak widoku collect leg values w tabeli sesji.
- **Symptom:** (8) `Internal error: stream lineage: collect fee rows: error returned from database: kolumna "lp_collected_token_a_raw" nie istnieje` na środowiskach z nieodpaloną migracją `005_ledger_lp_collected_raw.sql`.
- **Root cause:** Dwa etapy: (a) resolver executora ładował wallet tylko z części źródeł env i proces API nie dziedziczył signer vars; (b) collect path twardo failował gdy pre-read pozycji do `fee_owed_*` nie powiedzie się, mimo że sama transakcja collect może być wykonalna.
- **Root cause:** (c) zapytanie lineage zakładało fizyczne kolumny `lp_collected_token_*_raw` w `position_stream_ledger_rows`; na starszym schemacie były tylko w `raw_json`.
- **Fix:** Rozszerzono `load_wallet_from_env()` o fallback na `SOLANA_KEYPAIR`/`WALLET_KEYPAIR_BASE58`, dodano diagnostykę env/path oraz jawne przekazywanie signer vars w `Start-ClmmApi-8081.ps1`. Collect nie przerywa się już na błędzie odczytu `fee_owed_*`; wykonuje harvest i zapisuje authoritative leg values tylko gdy pre-read się powiedzie. API `collect_fees` zwraca teraz komunikat z kwotami obu nóg (A/B) wyliczony jako `pre_uncollected - post_uncollected` oraz dołącza szczegóły pre/post w `data`.
- **Fix:** Rozszerzono `load_wallet_from_env()` o fallback na `SOLANA_KEYPAIR`/`WALLET_KEYPAIR_BASE58`, dodano diagnostykę env/path oraz jawne przekazywanie signer vars w `Start-ClmmApi-8081.ps1`. Collect nie przerywa się już na błędzie odczytu `fee_owed_*`; wykonuje harvest i zapisuje authoritative leg values tylko gdy pre-read się powiedzie. API `collect_fees` zwraca teraz komunikat z kwotami obu nóg (A/B) wyliczony jako `pre_uncollected - post_uncollected` oraz dołącza szczegóły pre/post w `data`. Dla lineage dodano notę jakości danych: przy `collect_events > 0` i `A/B == 0` API jawnie komunikuje, że collect został wykonany przy `fee_owed_a/b == 0`.
- **Fix:** Rozszerzono `load_wallet_from_env()` o fallback na `SOLANA_KEYPAIR`/`WALLET_KEYPAIR_BASE58`, dodano diagnostykę env/path oraz jawne przekazywanie signer vars w `Start-ClmmApi-8081.ps1`. Collect nie przerywa się już na błędzie odczytu `fee_owed_*`; wykonuje harvest i zapisuje authoritative leg values tylko gdy pre-read się powiedzie. API `collect_fees` zwraca teraz komunikat z kwotami obu nóg (A/B) wyliczony jako `pre_uncollected - post_uncollected` oraz dołącza szczegóły pre/post w `data`. Dla lineage dodano notę jakości danych: przy `collect_events > 0` i `A/B == 0` API jawnie komunikuje, że collect został wykonany przy `fee_owed_a/b == 0`. Jeśli collect ma tylko jedną nogę zmapowaną, brakująca noga jest normalizowana do `0` (z notą), aby UI nie pokazywał `-`.
- **Fix:** Rozszerzono `load_wallet_from_env()` o fallback na `SOLANA_KEYPAIR`/`WALLET_KEYPAIR_BASE58`, dodano diagnostykę env/path oraz jawne przekazywanie signer vars w `Start-ClmmApi-8081.ps1`. Collect nie przerywa się już na błędzie odczytu `fee_owed_*`; wykonuje harvest i zapisuje authoritative leg values tylko gdy pre-read się powiedzie. API `collect_fees` zwraca teraz komunikat z kwotami obu nóg (A/B) wyliczony jako `pre_uncollected - post_uncollected` oraz dołącza szczegóły pre/post w `data`. Dla lineage dodano notę jakości danych: przy `collect_events > 0` i `A/B == 0` API jawnie komunikuje, że collect został wykonany przy `fee_owed_a/b == 0`. Jeśli collect ma tylko jedną nogę zmapowaną, brakująca noga jest normalizowana do `0` (z notą), aby UI nie pokazywał `-`. Dodano per-node `collect_zero_diagnostics` (in-range share est., swap count est., position share est.) i render w tabeli `LP Zebrane`. Dodatkowo LP legs dla collect są teraz brane priorytetowo z Orca `harvest_position_instructions.fees_quote` (obie nogi), a nie tylko z pre-read `PositionReader`.
- **Fix:** Rozszerzono `load_wallet_from_env()` o fallback na `SOLANA_KEYPAIR`/`WALLET_KEYPAIR_BASE58`, dodano diagnostykę env/path oraz jawne przekazywanie signer vars w `Start-ClmmApi-8081.ps1`. Collect nie przerywa się już na błędzie odczytu `fee_owed_*`; wykonuje harvest i zapisuje authoritative leg values tylko gdy pre-read się powiedzie. API `collect_fees` zwraca teraz komunikat z kwotami obu nóg (A/B) wyliczony jako `pre_uncollected - post_uncollected` oraz dołącza szczegóły pre/post w `data` (kwoty w komunikacie zaokrąglone do 3 miejsc). Dla lineage dodano notę jakości danych: przy `collect_events > 0` i `A/B == 0` API jawnie komunikuje, że collect został wykonany przy `fee_owed_a/b == 0`. Jeśli collect ma tylko jedną nogę zmapowaną, brakująca noga jest normalizowana do `0` (z notą), aby UI nie pokazywał `-`. Dodano per-node `collect_zero_diagnostics` (in-range share est., swap count est., position share est.) i render w tabeli `LP Zebrane`. Dodatkowo LP legs dla collect są teraz brane priorytetowo z Orca `harvest_position_instructions.fees_quote` (obie nogi), a nie tylko z pre-read `PositionReader`. W tabeli sesji (`Logs / rebalances`) dodano kolumnę `Collect values` z `A raw/B raw` dla collect tx.
- **Fix:** (2026-04-13) Query w `stream-lineage` został uodporniony na drift schematu: wartości `lp_collected_token_*_raw` są czytane z `raw_json` (aliasowane do tych samych nazw), bez bezpośredniego odwołania do brakujących kolumn.
- **Guards/tests:** dodać unit testy resolvera źródeł wallet (ścieżka vs env key material) i test collect_fees dla scenariusza „position pre-read fails but tx still executes”.
- **Paths:** `crates/api/src/services/position_executor.rs`, `crates/api/src/services/position_service.rs`, `crates/api/src/handlers/wallets.rs`, `tools/Start-ClmmApi-8081.ps1`, `crates/execution/src/strategy/rebalance.rs`, `crates/api/src/services/position_stream_lineage.rs`

### BUG-20260410-04 — Brak regresyjnych testów UI dla feedbacku collect/swap

status: open  
severity: high  
reported_by: user  
first_seen: 2026-04-10  
fixed_in:   
keywords: ui-tests, collect_fees, swap-before-open, message-passthrough, regression

- **Symptom:** Zmiany w komunikatach UI dla `Collect Fees` i `Swap` mogą wracać do starego, mylącego zachowania bez szybkiego wykrycia.
- **Root cause:** Brak dedykowanych testów integracyjnych frontend dla scenariuszy „success bez signature” / „backend message passthrough”.
- **Fix:** Dodać testy UI/integration dla `PositionDetail` i `PositionCreate` weryfikujące prezentację `message` z API.
- **Guards/tests:** PR touching `web/src/pages/PositionDetail.tsx` lub `web/src/pages/PositionCreate.tsx` powinien zawierać test dla tych ścieżek.
- **Paths:** `web/src/pages/PositionDetail.tsx`, `web/src/pages/PositionCreate.tsx`

### BUG-20260410-01 — Lineage showed inconsistent start/end for same node

status: open  
severity: high  
reported_by: user  
first_seen: 2026-04-10  
fixed_in: local  
keywords: lineage, baseline_value_usd, current_value_usd, rebalance_session_id, rotation

- **Symptom:** same PDA showed different `start/end` across views; sometimes `start` wildly above `end`.
- **Symptom:** `Logs / rebalances` can show `—` in `start value` / `end value` for older rows (or intermittently for the same row) because API emits `0` for baseline/current in some runs.
- **Root cause:** mixed-source lineage valuation with fallback drift.
- **Root cause:** per-node valuation in `stream-lineage` still depends on short-timeout best-effort price fetch (`fetch_mint_prices_usd` with 2s timeout) and incomplete open/close leg deltas in lifecycle rows; when either is missing, baseline/current may remain zero and UI intentionally renders `—` for zero.
- **Fix:** session-first continuity (`close(old)->open(new)` by same `rebalance_session_id`) plus guardrails on cap/continuity fallbacks.
- **Fix:** Added stable mint-price merge in lineage (`live + last-good cache`, TTL 15m) so temporary free-price timeouts do not zero `start/end` for rows that had recent quotes; backfill snapshots now use the same stable price path.
- **Fix:** `stream-lineage` now persists and prefers event snapshots (`baseline_open` / `end_close`) for chain PDAs in DB mode, with token amounts + USD + `price_source` + `valuation_quality` in snapshot `raw_json`; baseline/current selection prioritizes these kinds over generic earliest/latest snapshots.
- **Guards/tests:** Added unit invariants in `position_stream_lineage.rs` (`continuity_from_session_carries_prev_end_to_next_baseline`, `baseline_fallback_guardrail_blocks_implausible_prev_end`).
- **Paths:** `crates/api/src/services/position_stream_lineage.rs`

### BUG-20260410-02 — Collect fees UI said requested, but no real effect

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-10  
fixed_in: local  
keywords: collect_fees, ui-message, dry_run, position_detail

- **Symptom:** UI displayed `Collect requested.` even when backend returned dry-run/no-op style response.
- **Root cause:** frontend success toast ignored backend message payload.
- **Fix:** `PositionDetail` now shows API `message` for collect, not hardcoded text.
- **Guards/tests:** still missing dedicated UI integration test for collect message passthrough (open action item).
- **Paths:** `web/src/pages/PositionDetail.tsx`

### BUG-20260410-03 — Swap step appeared to do nothing

status: fixed  
severity: high  
reported_by: user  
first_seen: 2026-04-10  
fixed_in: local  
keywords: swap-before-open, position-create, dry-run, ui-feedback

- **Symptom:** clicking `Swap` often looked like no action.
- **Root cause:** backend dry-run/info response was not surfaced when signature was missing.
- **Fix:** `PositionCreate` now renders `swapStepInfo` from API message regardless of signature presence.
- **Guards/tests:** still missing dedicated UI integration test for swap message without `swap_signature` (open action item).
- **Paths:** `web/src/pages/PositionCreate.tsx`

