# Engineering notes (code changes)

**Purpose:** short, **append-only** entries whenever someone (or AI) makes a **non-trivial** code change. Optimized for **grep and semantic search**: each entry has a **`keywords:`** line with comma-separated tokens (crates, domains, CLI flags, protocols).

**When to add an entry**

- New or removed public CLI subcommand / important flag.
- Behavioral change in backtest, optimization, execution, or protocol adapters.
- New dependency, breaking RPC/data format assumption, or migration of on-disk layout under `data/`.
- Anything you would explain to a teammate in standup — if it touches multiple files or user-visible behavior, log it here.

**Skip** for: typo fixes, pure refactors with no behavior change, one-line test-only edits.

**Order:** **newest first** (add new `##` sections at the **top**, right under this preamble).

---
## 2026-04-04 — `/positions`: zakres kolor + `in_range` w `GET /orca/positions-by-owner`

**keywords:** Positions.tsx, OrcaOwnerPositionEntry, range_usdc_and_in_range_for_pool_ticks, in_range
**paths:** `web/src/pages/Positions.tsx`, `crates/api/src/handlers/orca_onchain.rs`, `crates/api/src/models.rs`, `crates/api/src/services/position_valuation.rs`

- Tabela monitora i tabela RPC: kolumna zakresu **zielona / czerwona** + etykieta In / Out of range.
- `OrcaOwnerPositionEntry.in_range`: jeden odczyt puli przez `range_usdc_and_in_range_for_pool_ticks` (wcześniej tylko zakres USDC).

---
## 2026-04-04 — Open Position: budżet USD ≈ wartość w pozycji (`quote-open-budget`)

**keywords:** deposit_quote, quote_open_budget, PositionCreate, token_max, Whirlpool, POST /pools, budget mode
**paths:** `crates/protocols/src/orca/deposit_quote.rs`, `crates/api/src/handlers/pools.rs`, `crates/api/src/models.rs`, `web/src/pages/PositionCreate.tsx`, `web/src/lib/api.ts`

- Problem: tryb „wspólna kwota USD” dzielił wartość 50/50 między tokeny → **caps** na Orca często w **złym stosunku** do krzywej przy wąskim zakresie → **~połowa** budżetu zostawała poza faktyczną płynnością.
- **Rozwiązanie:** `quote_deposit_budget_in_range` (binary search po `L`) + `POST /api/v1/pools/{address}/quote-open-budget` (ceny mintów jak `price_fetch`, ticki + `sqrt_price` z RPC). UI w trybie budżetowym **ustawia Amount** i wysyła **`token_max_*` z odpowiedzi** (bez straty float).
- Warunek: sensowny quote gdy **cena puli jest w [tick_lower, tick_upper)**; inaczej 400 z komunikatem lub ostrzeżenie `in_range: false`.

---
## 2026-04-04 — Wycena USD pozycji SOL/USDC: brak ceny WSOL (~½ wartości) + fallback

**keywords:** position_valuation, price_fetch, WSOL, Jupiter, CoinGecko, compute_position_usd_valuation, Value dashboard
**paths:** `crates/api/src/services/position_valuation.rs`, `crates/api/src/services/price_fetch.rs`

- Gdy mapa `mint → USD` nie zawiera ceny dla **WSOL** (`So111…`), kod używał `unwrap_or(0.0)` dla nogi SOL — **wartość pozycji** na dashboardzie była wtedy ok. **o połowę za niska** (została tylko noga USDC).
- **Walidacja on-chain:** jeśli brak ceny z feedu a mint to WSOL w parze **USDC + WSOL**, uzupełnienie **USD/SOL** z **bieżącego ticka puli** (`b_per_a_ui_decimal` — ta sama konwencja co zakres USDC).
- **price_fetch:** jeśli WSOL nadal nie ma ceny po Gecko/Jupiter/v4/DexPaprika/Dexscreener — ostatni fallback **CoinGecko** `simple/price` dla `solana` (tag źródła `coingecko_solana`).
- **Poprawka (wartość spadała ~\$1.16):** feed potrafi zwrócić **małą dodatnią** cenę WSOL (błędna jednostka / zły id) — wtedy stary warunek tylko `<= 0` **nie** podmieniał na tick puli. Teraz: **sanity band** (~\$10–\$2500) → tick; oraz **USDC mint → \$1** jeśli brak wpisu w mapie (żeby nie gubić nogi USDC).

---
## 2026-04-04 — `PositionResponse.uncollected_fees`: opłaty per token jak w Orca

**keywords:** UncollectedFeesInfo, fee_owed, PositionDetail, GET /positions, position_valuation, uncollected_fees_info_for_position
**paths:** `crates/api/src/models.rs`, `crates/api/src/services/position_valuation.rs`, `crates/api/src/handlers/positions.rs`, `web/src/pages/PositionDetail.tsx`

- Z waluacji USD (`compute_position_usd_valuation`) wystawiane są etykiety mintów (SOL/USDC/skrót) oraz kwoty `fee_owed` w jednostkach UI; pole opcjonalne gdy waluacja się nie uda.
- **Poprawka:** gdy pełna waluacja zwraca błąd (RPC), a zakres USDC nadal pochodzi z `tick_range_usdc_for_position` (osobny fetch z `.ok()`), `uncollected_fees` było puste — dodano `uncollected_fees_info_for_position` (tylko pool + decymale + `fee_owed`, bez math liquidity / cen).
- `GET /positions/:address`: przed waluacją USD wywołanie `refresh_position_fees_from_chain` (świeże `fee_owed` z RPC; monitor mógł pokazywać `$0.000` względem Orca). Web: `formatUsdUncollectedFees` — 6 miejsc dla kwot &lt; $0.01.
- `fees_usd` w `compute_position_usd_valuation`: mnożenie **Decimal** (fee UI × cena), nie `f64`; log `warn` gdy `fee_owed` &gt; 0 a USD = 0 (brak ceny mintu). UI: surowe `fees_earned_a/b` pod linią USD.
- Dashboard: „Uncollected fees (USD)” na szczegółach pozycji (format sub-cent).

## 2026-04-04 — Link pozycji ↔ strategia: `monitor.add_position` best-effort (RPC)

**keywords:** link_position_strategy, ensure_strategy_running_after_position_link, start_strategy_executor_core, PositionMonitor, add_position, Failed to get account
**paths:** `crates/api/src/handlers/strategies.rs`, `crates/api/src/handlers/positions.rs`

- Gdy publiczny RPC nie zwraca konta pozycji (`Failed to get account`), twarde **400** po zapisie `position_addresses` blokowało UI; teraz powiązanie zostaje zapisane, start executora kontynuowany, a komunikat HTTP **200** może zawierać ostrzeżenie zamiast błędu.
- Dotyczy: `POST /positions/{address}/strategy`, `POST /strategies/{id}/start`, oraz open z `strategy_id` (ścieżka z notatką zamiast wyłącznie „automation started”).

## 2026-04-04 — Vite: scalanie root `.env` dla `API_PORT` / proxy (mniej HTTP 502)

**keywords:** vite, API_UPSTREAM, API_PORT, loadEnv, dev proxy, 502
**paths:** `web/vite.config.ts`, `web/.env.example`

- `defineConfig` ładuje `loadEnv` z **repo root** i z `web/` (`web` nadpisuje root) — samo `npm run dev` widzi ten sam `API_PORT` co `cargo run --bin clmm-lp-api` z root `.env`, bez ręcznego `API_UPSTREAM`.
- Domyślny port proxy bez env: **8080** (zgodnie z `.env.example` i `start-dev-stack.mjs`), zamiast samego `web/` z hardcoded `8081`.

## 2026-04-04 — `POST /positions/{address}/strategy`: przypisz / zmień / odłącz strategię od PDA

**keywords:** link_position_strategy, LinkPositionStrategyRequest, position_addresses, executor_disabled_position_addresses, PositionDetail, append_position_address_to_strategy
**paths:** `crates/api/src/handlers/positions.rs`, `crates/api/src/services/strategy_service.rs`, `crates/api/src/routes.rs`, `web/src/lib/api.ts`, `web/src/pages/PositionDetail.tsx`

- Body: `{ "strategy_id": "<uuid>" | null }`. Ustawienie ID: usuwa PDA ze wszystkich strategii (także z list wyłączeń executora), dopisuje do wybranej, persystencja, `ensure_strategy_running_after_position_link`. `null`: tylko odłączenie (unlink).
- UI na szczegółach pozycji: lista powiązań lub „None linked”, select wszystkich strategii, **Apply** / **Remove link**; po sukcesie invalidacja `['strategies']`.

## 2026-04-04 — `GET /strategies`: zawsze scalaj `position_addresses` z surowego JSON

**keywords:** list_strategies, StrategyParameters, position_addresses, Open Position, strategy_id, PositionDetail
**paths:** `crates/api/src/handlers/strategies.rs`, `web/src/pages/PositionDetail.tsx`

- Lista i szczegóły strategii parsowały `parameters` przez `serde_json::from_value::<StrategyParameters>`; przy **nieudanym** deserializowaniu (np. starsze / nietypowe wartości liczbowe) używane było `unwrap_or_default()` — **tracone** `parameters.position_addresses`, więc UI pokazywało „None linked” mimo że po `POST /positions` z `strategy_id` adres był dopisany w pamięci/pliku.
- Dodano `strategy_parameters_from_stored_config`: najpierw best-effort `StrategyParameters`, potem **nadpisanie** `position_addresses` i `executor_disabled_position_addresses` z **surowego** obiektu `parameters` w `config`.
- Frontend: dopasowanie PDA z listą — `trim` + obsługa wartości nie-string; tooltip przy „None linked”.

## 2026-04-03 — `PositionResponse`: zakres w USDC (pary z jednym USDC)

**keywords:** PositionResponse, range_lower_usdc, range_upper_usdc, tick_range_usdc, PositionDetail, Dashboard
**paths:** `crates/api/src/services/position_valuation.rs`, `crates/api/src/handlers/positions.rs`, `crates/api/src/models.rs`, `web/src/lib/utils.ts`, `web/src/pages/PositionDetail.tsx`

- Dla puli **USDC + jeden inny token** API uzupełnia `range_lower_usdc` / `range_upper_usdc` (min/max granicy zakresu) oraz `range_usdc_quote` (np. `per 1 SOL`) — liczone z ticków + decymali mintów (mainnet/devnet USDC mint).
- Gdy para nie ma USDC, pola są `null` — UI pokazuje jak wcześniej same ticki.
- **Poprawka:** cena UI z ticka jest liczona w **log-domenie** (`exp(tick·ln(1.0001) + Δdec·ln(10))`), żeby uniknąć **underflow `f64`** na samym `1.0001^tick` przy głębokich ujemnych tickach (np. -25276) — wcześniej zakres USDC znikał mimo pary SOL/USDC.
- **`GET /orca/positions-by-owner`:** te same pola co w `PositionResponse` (po jednym fetchu stanu puli na wiersz), tabela RPC w UI pokazuje USDC jak monitor.

## 2026-04-03 — `GET /positions/:address`: fallback RPC gdy brak wpisu w monitorze

**keywords:** get_position, list_positions, monitored_position_from_chain, registry.jsonl, lifecycle_ledger_aggregates
**paths:** `crates/api/src/handlers/positions.rs`, `crates/api/src/services/position_valuation.rs`, `crates/api/src/position_registry_seed.rs`, `crates/api/src/services/lifecycle_ledger_aggregates.rs`, `crates/api/src/handlers/analytics.rs`

- Gdy adresu nie ma w `PositionMonitor`, API buduje stan z RPC (`PositionReader` + `WhirlpoolReader`) zamiast zwracać 404; w tle `tokio::spawn` próbuje `monitor.add_position` (healing listy).
- `GET /positions` (lista): dla każdego `registry_open` w `data/positions/registry.jsonl`, którego nie ma w monitorze, dokładany jest ten sam fallback RPC — **górna tabela** nie jest już pusta tylko przez „tylko RAM”.
- Przywrócono moduł `lifecycle_ledger_aggregates` + pole `fees_collected_from_ledger` w `GET /analytics/portfolio`.

## 2026-04-03 — `POST /positions`: `monitor.add_position` po open (szczegóły od razu)

**keywords:** open_position, PositionMonitor, add_position, GET /positions/:address, PositionCreate
**paths:** `crates/api/src/handlers/positions.rs`, `web/src/pages/PositionCreate.tsx`

- Po udanym open API woła `monitor.add_position(position_pda)`, żeby `GET /positions/:addr` działał od razu (wcześniej tylko rejestr / restart).
- Gałąź z samym `message` bez PDA bez zmian; idempotent replay (`message` + `position_pda`) nie jest już ucinany na początku.
- Web: po sukcesie nawigacja do `/positions/{position_pda}` gdy jest w odpowiedzi.

## 2026-04-03 — Whirlpool close: domyślnie niski slippage (100 bps) + opcjonalnie `WHIRLPOOL_CLOSE_SLIPPAGE_BPS`

**keywords:** close_position, WhirlpoolExecutor, TokenMinSubceeded, 6018, WHIRLPOOL_CLOSE_SLIPPAGE_BPS, RebalanceExecutor
**paths:** `crates/protocols/src/orca/executor.rs`, `crates/execution/src/strategy/rebalance.rs`, `.env.example`

- Domyślnie z powrotem **100 bps** (niski min-out); opcjonalnie env **`WHIRLPOOL_CLOSE_SLIPPAGE_BPS`** (`0..=10000`) na hoście API/CLI gdy trzeba obejść 6018 bez zmiany zasady „jak najniżej”.
- Hint przy `6018` doprecyzowany: podnieś slippage tylko przy retry / wyższy tymczasowy env.

## 2026-04-03 — API: ceny USD — GeckoTerminal przed Jupiterem (więcej źródeł bez klucza)

**keywords:** clmm-lp-api, price_fetch, GeckoTerminal, Jupiter, Dexscreener, DexPaprika
**paths:** `crates/api/src/services/price_fetch.rs`

- Do łańcucha cen dopisano **GeckoTerminal** `GET .../networks/solana/token_price/{mints}` (batch w chunkach), **przed** Jupiter v2 — darmowe, bez klucza, typowo wystarcza na dashboard bez `JUPITER_API_KEY`.
- Kolejność: stable → GeckoTerminal → Jupiter v2 → legacy v4 → DexPaprika → Dexscreener.

## 2026-04-03 — Rebalance API + UI: `strategy_range` / `price_band` / `ticks` (bez samego promptu na ticki)

**keywords:** RebalanceRequest, RebalanceInput, calculate_tick_range, PositionDetail, POST /positions/rebalance
**paths:** `crates/api/src/models.rs`, `crates/api/src/services/position_service.rs`, `web/src/lib/api.ts`, `web/src/pages/PositionDetail.tsx`

- `POST /positions/{address}/rebalance`: pole `input`: `ticks` (domyślnie), `strategy_range` (szerokość z `strategy_id` i/lub `range_width_pct`, środek = `tick_current`), `price_band` (`center_price` + `range_width_pct`).
- Dry-run z samymi tickami nadal bez odczytu puli z RPC (zgodność z testami offline).
- Web: panel wyboru trybu zamiast `prompt` na ticki.

## 2026-04-03 — Lifecycle ledger: kwoty tokenów przy `collect_fees` + sumy na szczegółach pozycji

**keywords:** bot_collect_fees, orca_position_lifecycle.jsonl, tx_lifecycle, PositionDetail, fee_payer_token_a_delta_ui, clmm-lp-protocols, clmm-lp-execution
**paths:** `crates/protocols/src/ledger/tx_lifecycle.rs`, `crates/execution/src/strategy/rebalance.rs`, `web/src/pages/PositionDetail.tsx`

- Wiersze `event: bot_collect_fees` dopisują opcjonalnie `pool_mint_*` oraz `fee_payer_token_*_delta_*` (post−pre z `meta.pre/postTokenBalances` dla fee payer + mintów puli), żeby UI mógł sumować realnie skredytowane fee w portfelu API po wielu collectach.
- `try_append_rebalance_executor_tx_cost` przyjmuje `Arc<RpcProvider>` (jedno pobranie tx na wiersz).
- Web: karta „Fees collected (cumulative, from ledger)” + kolumny ΔA/ΔB w tabeli ledgera; etykieta „Unclaimed fees (USD est.)” wyjaśnia różnicę względem niewypłaconych fee na pozycji.

## 2026-04-03 — API: USD dla pozycji / portfela — Jupiter v2 + fallback (koniec zer z powodu martwego `price.jup.ag/v4`)

**keywords:** clmm-lp-api, price_fetch, position_valuation, Jupiter, Dexscreener, DexPaprika, wallet, analytics
**paths:** `crates/api/src/services/price_fetch.rs`, `crates/api/src/services/position_valuation.rs`, `crates/api/src/handlers/prices.rs`, `web/src/lib/api.ts`, `.env.example`

- Publiczny endpoint `https://price.jup.ag/v4/price` bywa niedostępny; `https://api.jup.ag/price/v2` często wymaga `JUPITER_API_KEY` (`x-api-key`).
- Wspólne `fetch_mint_prices_usd`: stable USDC/USDT → Jupiter v2 → (best-effort) legacy v4 → DexPaprika SSE → Dexscreener `token-pairs` (jak w CLI snapshot).
- `GET /prices/jupiter` i wycena pozycji (`value_usd`, fees USD, dashboard) korzystają z tej samej ścieżki. Frontend: usunięto bezużyteczny fallback do v4 w przeglądarce.

## 2026-04-03 — Web: dłuższy timeout dla operacji on-chain (`/positions/*`)

**keywords:** web, timeout, fetch abort, open position, swap-before-open, api.ts
**paths:** `web/src/lib/api.ts`

- Domyślne `fetchJson` miało timeout 15s, co przy wolnym RPC powodowało abort requestu w UI mimo że transakcja mogła zostać wykonana po stronie backendu.
- Dodano `fetchJsonLong` (90s) i podpięto pod cięższe endpointy `POST/DELETE /positions/*` (swap/open/close/collect/rebalance/decrease).

## 2026-04-03 — Swap API: ograniczony czas post-confirmation (mniej abortów requestu)

**keywords:** clmm-lp-execution, swap-before-open, wait_for_confirmation, timeout, abort signal
**paths:** `crates/execution/src/strategy/rebalance.rs`

- Po potwierdzeniu tx swap endpoint potrafił długo wisieć na dodatkowym `wait_for_confirmation` (niestabilne `getTransaction`), co kończyło się abortem requestu po stronie UI.
- `ensure_execution_success` ma teraz timeout 15s dla post-confirmation check; przy timeout/err loguje ostrzeżenie i kontynuuje bez blokowania odpowiedzi API.

## 2026-04-03 — Runner restart: przed startem ubijamy stare instancje

**keywords:** script runner, Start-ClmmScriptRunner, Stop-ClmmScriptRunner, port reuse
**paths:** `tools/Start-ClmmScriptRunner.ps1`

- Wrapper runnera wywołuje `tools/Stop-ClmmScriptRunner.ps1` przed startem, żeby zwolnić “nasz” port przy restarcie zamiast mnożyć konflikty listenerów.

## 2026-04-03 — API: większy stack Tokio workerów (fix crash `swap-before-open`)

**keywords:** clmm-lp-api, tokio, stack overflow, swap-before-open, 502
**paths:** `crates/api/src/main.rs`

- `POST /positions/swap-before-open` potrafił crashować proces API (`thread 'tokio-rt-worker' has overflowed its stack`), a frontend widział `HTTP 502 (empty body)`.
- API uruchamia teraz runtime Tokio z konfigurowalnym większym stackiem workerów (`API_TOKIO_STACK_SIZE_BYTES`, domyślnie 8 MiB), co stabilizuje ścieżkę swapu.

## 2026-04-03 — Web dev proxy: domyślny API upstream na `:8081` (nie `:8080`)

**keywords:** web, vite, API_UPSTREAM, API_PORT, 8081, dev proxy
**paths:** `web/vite.config.ts`

- Domyślny port backendu dla Vite proxy zmieniony z `8080` na `8081`, bo lokalny workflow dashboardu/API używa `:8081`, a `:8080` bywa zajęte przez inne usługi.
- Ogranicza to przypadki, gdzie UI pokazuje “puste” dane po restarcie, mimo że właściwy API (`:8081`) ma już zapisane strategie.

## 2026-04-03 — Runner: wrapper `Start-ClmmScriptRunner.ps1` używa `/health` zamiast `netstat`

**keywords:** script runner, Start-ClmmScriptRunner, CLMM_SCRIPT_RUNNER_PORT, HTTP.sys, /health
**paths:** `tools/Start-ClmmScriptRunner.ps1`

- Poprawione wykrywanie zajętego portu: wrapper nie przełącza już na `9857`, jeśli `9847` jest zajęte przez HTTP.sys, ale runner odpowiada na `GET /health`.
- Dzięki temu API i UI mniej tracą spójność portów przy wielokrotnych restartach.

## 2026-04-03 — API: strategie persistują do `data/strategies.json` (do DELETE)

**keywords:** clmm-lp-api, strategies, persistence, strategy_store, JSON store, restart
**paths:** `crates/api/src/strategy_store.rs`, `crates/api/src/state.rs`, `crates/api/src/handlers/strategies.rs`, `crates/api/src/services/strategy_service.rs`

- Dodano persistencję strategii na dysk, aby strategie nie znikały po restarcie API — dopóki ich nie usuniesz przez `DELETE /strategies/{id}`.
- Zapis wykonuje się po `POST /strategies`, `PUT /strategies/{id}`, `DELETE /strategies/{id}`, po zmianie `executor_disabled_position_addresses` oraz po linkowaniu pozycji do strategii (`parameters.position_addresses`).

## 2026-04-03 — API: DRY_RUN env steruje trybem live/symulacji (swap/open) + runner port

**keywords:** clmm-lp-api, DRY_RUN, dry_run, swap-before-open, Start-Dashboard-Safe, script_runner
**paths:** `crates/api/src/state.rs`, `tools/Start-Dashboard-Safe.ps1`, `tools/script_runner/Start-ClmmScriptRunner.ps1`

- API przestaje mieć „zabetonowane” `dry_run=true` — teraz czyta `DRY_RUN` z `.env` (domyślnie nadal `true` dla bezpieczeństwa).
- Skrypty startowe przestają mylnie przełączać runnera na `:9857` gdy `:9847` wygląda na zajęte przez HTTP.sys; decyzja jest oparta o `GET /health`, a runner ma fallback portu przy niepowodzeniu bind.

## 2026-04-03 — API: obsługa WALLET_KEYPAIR_PATH dla operacji position (swap/open)

**keywords:** clmm-lp-api, KEYPAIR_PATH, SOLANA_KEYPAIR_PATH, WALLET_KEYPAIR_PATH, wallet loading, position_executor
**paths:** `crates/api/src/services/position_executor.rs`

- `swap-before-open`/`open position` po przełączeniu na live wymaga `StrategyExecutor` z podpisującym walletem.
- U Ciebie `.env` używa `WALLET_KEYPAIR_PATH`, a kod wcześniej czytał tylko `KEYPAIR_PATH`/`SOLANA_KEYPAIR_PATH` — dlatego executor nie był tworzony.
- Dodano alias `WALLET_KEYPAIR_PATH` oraz rozwijanie `~/...` na Windows.

## 2026-04-03 — UI: dwa kroki SWAP -> OPEN (endpoint `/positions/swap-before-open`)

**keywords:** PositionCreate, swap-before-open, two-step flow, swapBeforeOpen endpoint, cost_session_id
**paths:** `crates/api/src/models.rs`, `crates/api/src/handlers/positions.rs`, `crates/api/src/routes.rs`, `crates/api/src/openapi.rs`, `web/src/lib/api.ts`, `web/src/pages/PositionCreate.tsx`, `crates/api/src/services/position_service.rs`

- Dodano endpoint **`POST /positions/swap-before-open`**: wykonuje tylko Orca swap `ExactIn` (bez open).
- UI w `PositionCreate` teraz:
  - najpierw wykonuje SWAP i pokazuje `swap_signature`,
  - dopiero po powodzeniu umożliwia OPEN Position (bez `swap_before_open` w payload).
- `cost_session_id` (księgowanie) jest generowane na kroku SWAP i przekazywane na krok OPEN, żeby sumować koszty per otwarta pozycja.

## 2026-04-03 — UI: „Zakres według ceny” pokazuje złe wartości (raw vs UI decimals)

**keywords:** PositionCreate, uiPriceFromRawPriceRatio, rawPriceRatioFromUiPrice, decimals, tickToPriceRatio
**paths:** `web/src/lib/whirlpoolTicks.ts`, `web/src/pages/PositionCreate.tsx`

- Poprawione mapowanie `tick <-> price` w sekcji „Zakres według ceny”: `tick_to_price` daje raw ratio, więc do UI (np. USDC za 1 SOL) trzeba uwzględnić różnicę `decimals`.
- UI:
  - synchronizacja cen z tickami jest przeliczana raw -> UI,
  - przeliczenie „Ustaw ticki z tych cen” jest UI -> raw (przed wyrównaniem do `tick_spacing`).

## 2026-04-02 — Swap + Open / position ops: portfel z `KEYPAIR_PATH` (nie tylko `auto_execute`)

**keywords:** position_executor, resolve_executor_for_position_ops, set_wallet, KEYPAIR_PATH, start_strategy_executor_core, StrategyExecutor, swap_before_open
**paths:** `crates/api/src/services/position_executor.rs`, `crates/api/src/handlers/positions.rs`, `crates/api/src/handlers/strategies.rs`, `crates/api/src/services/strategy_service.rs`

- **Problem:** `set_wallet` było wywoływane tylko przy `auto_execute && !dry_run`, więc przy `auto_execute=false` executor nie miał klucza — **swap i open** kończyły się „Wallet not set”.
- **Fix:** przy `!dry_run` ładuj portfel z `KEYPAIR_PATH` / `SOLANA_KEYPAIR_PATH` zawsze, gdy plik jest ustawiony; `auto_execute=true` bez env nadal zwraca 400.
- **`resolve_executor_for_position_ops`:** jeśli nie ma uruchomionej strategii, tworzy leniwy executor pod `__api_position_ops__` (ten sam keypair), żeby **POST /positions** działało bez **POST /strategies/…/start**.

## 2026-04-02 — Open Position: zakres przez cenę (B/A) → ticki

**keywords:** PositionCreate, whirlpoolTicks, alignPriceRatioToTicks, price ratio, tick spacing
**paths:** `web/src/lib/whirlpoolTicks.ts`, `web/src/pages/PositionCreate.tsx`

- Obok pól **Tick Lower / Upper**: sekcja **ceny graniczne** (ten sam stosunek co `price` puli: mint B za 1 mint A). Przycisk **„Ustaw ticki z tych cen”** wylicza ticki wyrównane do `tick_spacing` (`floor` dolnej ceny, `ceil` górnej; zamiana jeśli użytkownik poda odwrotnie).
- Pola cenowe synchronizują się z tickami, gdy edytujesz ticki lub auto-sync strategii; ręczna edycja cen wyłącza sync do czasu „Ustaw ticki…”.

## 2026-04-02 — Koszt swapu: szacunek (ledger) + `cost_session_id` (księgowanie per pozycja)

**keywords:** swap cost, cost_session_id, rebalance_session_id, orca_position_lifecycle.jsonl, swap_cost_estimate, bot_swap_exact_in, PositionOpenResponse, GET estimate-swap-cost, position_registry, tx_lifecycle
**paths:** `crates/protocols/src/ledger/tx_lifecycle.rs`, `crates/protocols/src/ledger/swap_cost_estimate.rs`, `crates/protocols/src/ledger/position_registry.rs`, `crates/execution/src/strategy/rebalance.rs`, `crates/api/src/models.rs`, `crates/api/src/handlers/pools.rs`, `crates/api/src/handlers/positions.rs`, `web/src/pages/PositionCreate.tsx`

- **`GET /pools/{address}/estimate-swap-cost`**: mediana `tx_fee_lamports` z lokalnego JSONL dla `swap_exact_in` w tej puli (jeśli brak — `DEFAULT_ESTIMATED_SWAP_NETWORK_FEE_LAMPORTS` = 10_000); pokazuje **opłatę sieciową** (`meta.fee`), nie pełny delta SPL/SOL portfela.
- **`OpenPositionRequest.cost_session_id`**: opcjonalnie; **PositionCreate** generuje UUID przy „Swap + Open”, żeby wiersze swap + open miały ten sam `rebalance_session_id` w ledgerze (suma kosztów sesji → przypisanie do nowej pozycji po `position_pda` w tym samym id).
- **`try_append_rebalance_executor_tx_cost` / CLI swap / registry**: opcjonalny override sesji zamiast wyłącznie `CLMM_REBALANCE_SESSION_ID`; zdarzenie `swap_exact_in` → `event: bot_swap_exact_in`.
- **`POST /positions`**: odpowiedź **`PositionOpenResponse`** (`message`, `position_pda`, `swap_signature`, `cost_session_id`).

## 2026-04-02 — Open Position: swap w puli Orca przed `open` (`swap_before_open`)

**keywords:** open position, swap_before_open, SwapInPoolBeforeOpen, swap_exact_in, Orca ExactIn, PositionService, PositionCreate, RebalanceExecutor
**paths:** `crates/protocols/src/orca/executor.rs`, `crates/execution/src/strategy/rebalance.rs`, `crates/execution/src/strategy/executor.rs`, `crates/api/src/models.rs`, `crates/api/src/services/position_service.rs`, `web/src/lib/api.ts`, `web/src/pages/PositionCreate.tsx`

- **`OpenPositionRequest.swap_before_open`**: opcjonalnie `{ specified_mint, amount_in }` — walidacja mintu A/B puli; najpierw swap ExactIn w tej puli, potem open (portfel API / executor).
- **UI**: przy jednostronnym niedoborze — checkbox + szacunek `amount_in`; przycisk „Swap + Open Position”.
- **Executor**: `WhirlpoolExecutor::swap_exact_in` + `RebalanceExecutor::execute_swap_exact_in` — do ponownego użycia przy rebalansowaniu.

## 2026-04-02 — Branding: nazwa produktu „Bociarz LP” (UI, OpenAPI, logi, docs)

**keywords:** branding, Bociarz LP, web, openapi, phantom_auth, README
**paths:** `web/src/components/Layout.tsx`, `crates/api/src/openapi.rs`, `crates/api/src/main.rs`, `README.md`, `web/index.html`

- User-facing teksty **„CLMM LP”** / **„Bociarz LP Strategy Lab”** ujednolicono do **„Bociarz LP”** (nagłówek dashboardu, tytuł strony, OpenAPI, komunikat Phantom sign-in, start API/CLI, monitoring Docker). Nazwy crate’ów (`clmm-lp-*`) i zmienne env (`CLMM_*`) bez zmian.

## 2026-04-02 — Strategia: Range Width % wymagany; Open Position — ticki z puli + strategii

**keywords:** web, StrategyCreate, StrategyEdit, PositionCreate, range_width_pct, calculate_tick_range, whirlpoolTicks, getPoolState
**paths:** `web/src/lib/strategyFormShared.tsx`, `web/src/lib/whirlpoolTicks.ts`, `web/src/pages/StrategyCreate.tsx`, `web/src/pages/StrategyEdit.tsx`, `web/src/pages/PositionCreate.tsx`

- **`FIELD_ENABLED.static_range.rangeWidth`**: włączone — **Range Width %** jest wymagany dla wszystkich typów, które go używają (`isRangeWidthSatisfied` przy create/save).
- **`calculateTickRangeFromWidthPct`**: TS odpowiednik `orca::pool_reader::calculate_tick_range` (szerokość % całego pasma ceny, tick spacing).
- **`tickToPriceRatio` / `formatPriceRatio`**: pod polami ticków — **ceny mint B za 1 mint A** przy dolnym/górnym ticku + spot z puli (zgodnie z `1.0001^tick`).
- **Open Position**: przy wybranej strategii z `parameters.range_width_pct` — tick lower/upper wyliczane z **`getPoolState`** (refetch ~10 s) + checkbox **auto-sync**; ręczna edycja ticków wyłącza sync.

## 2026-04-02 — Open Position: walidacja sald vs kwoty + linki Jupiter (prefill)

**keywords:** web, PositionCreate, WalletBalancesResponse, Jupiter, WSOL, swap UX
**paths:** `web/src/pages/PositionCreate.tsx`

- Porównanie **wymaganych kwot** (token A/B) z **saldem** (dla WSOL: native SOL + konto WSOL); przy niedoborze — **blokada** „Open Position”, komunikat PL i CTA.
- **Jupiter** `https://jup.ag/swap?inputMint=&outputMint=` + opcjonalnie `amount` (szacunek ExactIn z cen USD API, +5% bufor). Portfel w przeglądarce zwykle łączy się z Jupiterem po otwarciu karty.
- Link **Orca** (strona główna) jako alternatywa bez prefilla URL.
- **Na później (nie wdrożone):** składanie swap tx w API / jedna transakcja swap+open; semi-auto z podpisem w aplikacji.

## 2026-04-02 — `CreateStrategyRequest`: opcjonalne `pool_address` (deserialize + PUT)

**keywords:** CreateStrategyRequest, pool_address, serde, PUT /strategies, StrategyEdit, clmm-lp-api
**paths:** `crates/api/src/models.rs`, `crates/api/src/handlers/strategies.rs`, `web/src/lib/api.ts`, `web/src/pages/StrategyEdit.tsx`

- Pole **`pool_address`** w body POST/PUT jest **opcjonalne** (`Option` + `serde(default)`), żeby klient bez tego pola nie dostawał błędu „missing field `pool_address`”.
- **POST**: jeśli podano niepusty string — trafia do `config.pool_address`.
- **PUT**: niepusty string ustawia pool; pusty string czyści legacy pool w configu; **brak pola / `null`** — merge: zostaje poprzedni `pool_address` z bazy.
- **Web (edycja)**: jeśli strategia ma legacy `pool_address`, pole jest wysyłane przy zapisie.

## 2026-04-02 — Open Position + strategia: auto-start executora; pauza per pozycja

**keywords:** open_position, ensure_strategy_running_after_position_link, executor_disabled_position_addresses, StrategyExecutor, position-executor, PositionDetail
**paths:** `crates/api/src/handlers/positions.rs`, `crates/api/src/handlers/strategies.rs`, `crates/execution/src/strategy/executor.rs`, `web/src/pages/PositionDetail.tsx`, `web/src/lib/api.ts`

- Po udanym `open` z `strategy_id`: dopisek PDA do strategii, potem **automatyczny start** executora (`ensure_strategy_running_after_position_link`); jeśli strategia już działała — tylko `monitor.add_position` + sync listy wyłączeń.
- **`parameters.executor_disabled_position_addresses`**: PDA pomijane w `evaluate_all` (automatyzacja wyłączona dla tej pozycji, bez zatrzymywania całej strategii).
- **`POST /strategies/{id}/position-executor`** body `{ position_address, enabled }` — usuwa/dodaje PDA na liście wyłączeń; UI na szczegółach pozycji.

## 2026-04-02 — Strategie: edycja, usuwanie, `auto_execute` w odpowiedzi API

**keywords:** web, StrategyEdit, StrategyDetail, delete_strategy, StrategyResponse, auto_execute, clmm-lp-api
**paths:** `crates/api/src/handlers/strategies.rs`, `web/src/pages/StrategyEdit.tsx`, `web/src/pages/StrategyDetail.tsx`, `web/src/App.tsx`, `web/src/lib/api.ts`

- **`GET/POST/PUT /strategies`**: `StrategyResponse` zawiera **`auto_execute`** (spójnie z `dry_run` i configiem).
- **`DELETE /strategies/{id}`**: przed usunięciem — zatrzymanie executora (`stop`), `running = false`, cleanup `executors` / `optimization_busy` (jak przy stop), potem usunięcie wpisu.
- **Web**: trasa **`/strategies/:id/edit`**, formularz edycji (PUT), przyciski **Edit / Delete** na szczegółach; na liście skrót **Edit**.

## 2026-04-02 — Strategia bez poolu: parametry zapis; pool przy Open Position

**keywords:** web, StrategyCreate, CreateStrategyRequest, StrategyParameters, position_addresses, OpenPositionRequest
**paths:** `crates/api/src/models.rs`, `crates/api/src/handlers/strategies.rs`, `crates/api/src/services/strategy_service.rs`, `crates/api/src/handlers/positions.rs`, `web/src/pages/StrategyCreate.tsx`, `web/src/pages/PositionCreate.tsx`, `web/src/lib/api.ts`

- **`CreateStrategyRequest`**: głównie parametry (`strategy_type`, `parameters`, dry_run, `auto_execute`); opcjonalne legacy **`pool_address`** (patrz wpis wyżej). Pool przy nowych flow wybierany przy **otwarciu pozycji**.
- **`StrategyResponse.pool_address`**: opcjonalne (legacy / stare wpisy); nowe strategie zwracają brak pola lub pusto.
- **`PUT /strategies/{id}`**: przy aktualizacji **zachowywane** jest `parameters.position_addresses` ze starej konfiguracji (merge), żeby nie gubić powiązań z pozycjami.
- **`append_position_address_to_strategy`**: bez walidacji poolu strategii vs pozycji — dopisek PDA po udanym `open`.
- **`StrategyParameters`** (Rust): pole `position_addresses`; frontend: lista powiązanych adresów na **Strategy detail**.

## 2026-04-02 — Open Position: opcjonalne przypisanie strategii (`strategy_id`)

**keywords:** web, PositionCreate.tsx, OpenPositionRequest, position_addresses, strategies, clmm-lp-api
**paths:** `crates/api/src/models.rs`, `crates/api/src/handlers/positions.rs`, `crates/api/src/services/strategy_service.rs`, `web/src/pages/PositionCreate.tsx`, `web/src/lib/api.ts`

- `POST /positions` przyjmuje opcjonalne **`strategy_id`**. Przed otwarciem: walidacja, że strategia istnieje i **`pool_address` strategii = pool pozycji**.
- Po sukcesie on-chain (pole `position_pda` w odpowiedzi serwisu) PDA jest dopisywane do **`parameters.position_addresses`** danej strategii (bez duplikatów).
- **Dry-run** nie zwraca PDA — linkowanie nie następuje (brak adresu).
- UI *Open Position*: lista strategii filtrowana po wybranym poolu.

## 2026-04-02 — Web Strategy Create: pola zależne od typu, opisy, tooltips

**keywords:** web, StrategyCreate.tsx, tooltip, StrategyType, StrategyParameters, execution, getPools
**paths:** `web/src/pages/StrategyCreate.tsx`, `web/src/components/ui/tooltip.tsx`

- Formularz tworzenia strategii **wyłącza** (disabled) parametry liczbowe nieużywane przez wybrany `strategy_type`; przy zmianie typu **czyści** wartości pól, które przestają obowiązywać.
- Pod wyborem typu: **krótki opis** zachowania trybu; przy każdej etykiecie — **tooltip** (ikona) z wyjaśnieniem pola i wpływu wartości na executor.
- Payload wysyła tylko **dozwolone** dla typu klucze w `parameters` (spójnie z backendem).
- **Pool:** zamiast ręcznego wpisywania adresu — `<select>` z `GET /pools` (sort wg TVL), przycisk odświeżenia, komunikaty przy błędzie / pustej liście; **etykiety par** z `GET /orca/tokens/{mint}` (symbole jak SOL/USDC, fallback skrót mintu).

## 2026-04-02 — Web Wallet: SPL jako token (Orca), nie sam skrót mintu

**keywords:** web, Wallet.tsx, getOrcaToken, orca/tokens, SPL
**paths:** `web/src/pages/Wallet.tsx`

- Tabela „Saldo on-chain” pokazuje **symbol/nazwę** z `GET /api/v1/orca/tokens/{mint}` (pierwsza linia), **pełny mint** pod spodem; kolumna „Mint” → „Token”.

## 2026-04-02 — Web: typy `Pool` / `PoolState` zgodne z API (`PoolResponse`)

**keywords:** web, api.ts, Pools.tsx, PoolDetail.tsx, PositionCreate.tsx, PoolResponse, PoolStateResponse, orca/tokens
**paths:** `web/src/lib/api.ts`, `web/src/pages/Pools.tsx`, `web/src/pages/PoolDetail.tsx`, `web/src/pages/PositionCreate.tsx`

- Frontend używał legacy pól (`token_a`/`token_b`, `fee_tier`, rezerwy w stanie), podczas gdy backend zwraca `token_mint_a`/`token_mint_b`, `fee_rate_bps`, `apy_estimate` oraz stan z `sqrt_price` i `fee_growth_*` — przez to `/pools/:address` mogło rzucać przy renderze (biały ekran).
- Dodano `getOrcaToken` (`GET /api/v1/orca/tokens/{mint}`) do symboli/decimals w szczegółach puli i w formularzu „Open Position”.

## 2026-04-02 — Runner skryptów: wygodniejszy start + stabilne HTTP/1.1

**keywords:** scripts, runner, web, api, http1, dotenv, tools
**paths:** `tools/script_runner/Start-ClmmScriptRunner.ps1`, `tools/Start-ClmmScriptRunner.ps1`, `web/src/pages/Scripts.tsx`, `crates/api/src/handlers/scripts.rs`

- Runner skryptów przy starcie importuje root `.env` (tylko brakujące zmienne), więc nie trzeba ręcznie ustawiać `CLMM_SCRIPT_RUNNER_TOKEN` ani `SOLANA_RPC_URL` w oknie PowerShell.
- API wymusza HTTP/1.1 do komunikacji z runnerem (HttpListener jest wrażliwy na nowsze tryby), co usuwa losowe 502/500 przy `POST /scripts/{id}/run`.
- UI `/scripts` pokazuje stan uruchamiania i blokuje wieloklik dla tego samego skryptu do czasu zakończenia.

## 2026-04-02 — Scripts: poprawione `tools/scripts-manifest.json` opisy

**keywords:** web, Scripts.tsx, tools, scripts-manifest, summary, when_to_use, runner
**paths:** `tools/scripts-manifest.json`, `doc/SCRIPTS_CATALOG.md`, `web/src/pages/Scripts.tsx`

- Uzupełniono/naprawiono `summary` w manifeście dla wszystkich `tools/*.ps1`, tak aby UI “Catalog” pokazywało opis faktycznej roli skryptu (nie generyczne “Operator script …”).
- Dodano brakujące wpisy dla `Start-Dashboard.ps1` i `Stop-ClmmApi.ps1` (wcześniej były tylko `auto_discovered`).

## 2026-04-02 — clmm-lp-cli: `pool_meta.json` cache dla snapshot backtestów (decimals + tick_spacing)

**keywords:** clmm-lp-cli, snapshot-backtest-prep, backtest-optimize, pool_meta, decimals, tick_spacing, rpc, snapshots
**paths:** `crates/cli/src/commands/snapshot_backtest_prep.rs`, `crates/cli/src/main.rs`, `crates/cli/src/commands/snapshot_price_path.rs`

- `snapshot-backtest-prep` zapisuje pod `data/backtest-snapshot-cache/` plik `pool_meta.json` (na pool) z `token_mint_a/b` + `*_decimals` oraz `tick_spacing` pobranymi z Whirlpool state.
- `backtest-optimize` w trybie `--price-path-source snapshots` próbuje czytać `pool_meta.json` i przekazuje `*_decimals` jako override do `snapshot_price_path::build_from_orca_snapshots`, dzięki czemu (przy gotowym cache) wyniki są powtarzalne nawet gdy RPC jest blokowane.
- `tick_spacing` jest cache’owane dla spójności metadanych i przyszłych kroków; obecnie snapshot-fees backtests nie wymagają go do wyliczeń ceny/fee z samego `snapshots.jsonl`.

## 2026-04-02 — clmm-lp-cli: `backtest-optimize` nie robi RPC `fetch_pool_state` przy snapshotach opłat

**keywords:** clmm-lp-cli, backtest-optimize, snapshots, fee-source snapshots, fetch_pool_state, rpc
**paths:** `crates/cli/src/main.rs`

- W trybie `--price-path-source snapshots` + `--fee-source snapshots` (snapshot-fees jako źródło prawdy) pominięto `crate::commands::backtest_optimize::fetch_pool_state()` — to usuwa zależność od on-chain Orca pool state w backtestach i pozwala działać nawet przy blokowanych/publicznych endpointach RPC.
- Fallback do on-chain RPC zostaje, gdy nie używamy snapshot fees (np. gdy `fee_source != snapshots` lub snapshot fee model jest wyłączony).

## 2026-04-02 — API: `dotenv()` + env dla runnera skryptów

**keywords:** clmm-lp-api, dotenv, scripts runner, SCRIPT_RUNNER_URL, SCRIPT_RUNNER_TOKEN
**paths:** `crates/api/src/main.rs`, `crates/api/Cargo.toml`, `.env.example`

- `clmm-lp-api` nie ładował wcześniej root `.env` — dodano `dotenv().ok()` i zależność `dotenv`, żeby `SCRIPT_RUNNER_URL` / `SCRIPT_RUNNER_TOKEN` działały zgodnie z dokumentacją.
- Uzupełniono `/.env.example` o zmienne dla lokalnego `tools/script_runner`.

## 2026-04-01 — web: `@vitejs/plugin-react` ^6 (Vite 8 peer)

**keywords:** web, vite, @vitejs/plugin-react, npm, ERESOLVE
**paths:** `web/package.json`

- `vite@^8` kolidował z `@vitejs/plugin-react@4.x` (peer tylko do Vite 4–7). Podniesiono plugin do **^6**, zgodnie z peer `vite@^8`.

## 2026-04-01 — UI §1 skrypty: audyt w `UI_REQUIREMENTS_PHASE1.md`, modal „Logi” (excerpt runu)

**keywords:** UI_REQUIREMENTS_PHASE1, Scripts.tsx, SCRIPTS_CATALOG, script_runs.jsonl
**paths:** `doc/UI_REQUIREMENTS_PHASE1.md`, `web/src/pages/Scripts.tsx`

- Tabela **mapowanie wymagań → implementacja** dla §1; strona Scripts: link do `doc/SCRIPTS_CATALOG.md`, przycisk **Logi** (stdout/stderr/error z `last_run`).

## 2026-04-01 — GET /scripts: `resolve_repo_root` — spacer w górę od cwd / exe (pusta lista skryptów)

**keywords:** clmm-lp-api, CLMM_REPO_ROOT, scripts-manifest, GET /scripts
**paths:** `crates/api/src/handlers/scripts.rs`, `web/src/pages/Scripts.tsx`

- Gdy API startuje z podkatalogu (np. `web/`), wcześniej `current_dir` nie zawierał `tools/` → **0 skryptów**. Teraz: szukanie root po `tools/scripts-manifest.json` albo `Cargo.toml` + `tools/` + `crates/`, potem spacer od `current_exe` (`target/debug/…`).
- UI: komunikat gdy katalog pusty + pole `repo_root` (diagnostyka).

## 2026-04-01 — Scripts: API + runner — pełna lista `tools/*.ps1` (merge z manifestem), pole `auto_discovered`

**keywords:** clmm-lp-api, GET /scripts, ScriptCatalogItem, scripts-manifest, Start-ClmmScriptRunner, UI_REQUIREMENTS_PHASE1
**paths:** `crates/api/src/handlers/scripts.rs`, `crates/api/src/models.rs`, `tools/script_runner/Start-ClmmScriptRunner.ps1`, `web/src/pages/Scripts.tsx`, `doc/UI_REQUIREMENTS_PHASE1.md`

- `list_scripts`: skan top-level `tools/*.ps1`, merge z manifestem (ścieżka już w manifeście → bez duplikatu); bez manifestu — tylko skan. `POST /scripts/{id}/run`: manifest lub `tools/{id}.ps1`.
- Runner PS: `Resolve-ScriptEntry` — ten sam fallback; brak twardego wymogu manifestu przy starcie.

## 2026-04-01 — doc: Docker Desktop (Windows) troubleshooting, `UI_REQUIREMENTS_PHASE1` + status implementacji, confirm przy zamknięciu pozycji

**keywords:** docker, Docker Desktop, UI_REQUIREMENTS_PHASE1, PositionDetail, DOCKER.md, STARTUP.md
**paths:** `doc/DOCKER.md`, `doc/UI_REQUIREMENTS_PHASE1.md`, `STARTUP.md`, `web/src/pages/PositionDetail.tsx`

- Błąd pipe `dockerDesktopLinuxEngine` → uruchomić Docker Desktop; rozszerzony `UI_REQUIREMENTS_PHASE1.md` (środowisko + tabela statusu fazy 1).
- **Close position:** `window.confirm` przed `closePosition` (§5 destruktywne akcje).

## 2026-04-01 — Docker Compose: `api` + `web`, `API_UPSTREAM`, `doc/DOCKER.md`

**keywords:** docker, docker-compose, vite, API_UPSTREAM, VITE_DOCKER, CHOKIDAR_USEPOLLING, Makefile docker-up
**paths:** `docker-compose.yml`, `docker/api/Dockerfile`, `docker/web/Dockerfile`, `.dockerignore`, `web/vite.config.ts`, `doc/DOCKER.md`, `Makefile`, `STARTUP.md`

- Dev: bind mount `./web`, wolumen na `node_modules`; proxy Vite do `http://api:8080` zamiast localhost w kontenerze.
- Prod: osobno (build statyczny + nginx/CDN) — opisane krótko w `doc/DOCKER.md`.

## 2026-04-01 — web: `npm run dev:stack` — jeden terminal (concurrently + kill-port), uproszczony Start-Dashboard

**keywords:** web, npm, dev:stack, concurrently, kill-port, Start-Dashboard, vite, clmm-lp-api
**paths:** `web/scripts/start-dev-stack.mjs`, `web/package.json`, `tools/Start-Dashboard.ps1`, `Start-Dashboard.bat`, `STARTUP.md`

- Domyślny start: **`npm start`** (= `dev:stack`) — równoległe zwolnienie portów + `taskkill`/`pkill` API, baner (picocolors), concurrently z **`prefix: name`**, **`padPrefix`**, **`timings`**, kolory hex; opcjonalnie **`CLMM_OPEN_BROWSER=true`** → `vite --open`.
- `Start-Dashboard.bat` / `tools/Start-Dashboard.ps1`: `npm install` (jeśli brak `node_modules`) → **`npm start`**.

## 2026-04-01 — Windows: `Stop-ClmmApi.ps1`, zatrzymanie starego API przed `Start-Dashboard` (blokada `clmm-lp-api.exe`)

**keywords:** windows, cargo, clmm-lp-api, Start-Dashboard, Stop-ClmmApi, orca_read_service
**paths:** `tools/Stop-ClmmApi.ps1`, `tools/Start-Dashboard.ps1`, `crates/api/src/services/orca_read_service.rs`, `STARTUP.md`

- `cargo run` nie może nadpisać `.exe`, gdy proces nadal działa — skrypt stop + auto-stop w launcherze.
- `OrcaReadService`: `#[allow(dead_code)]` na polach zarezerwowanych pod REST (usuwa warning przy buildzie API).

## 2026-04-01 — Windows: `Start-Dashboard.bat` + baner „brak API” w Layout

**keywords:** web, vite, Start-Dashboard, CLMM_REPO_ROOT, ApiBackendBanner, proxy
**paths:** `Start-Dashboard.bat`, `tools/Start-Dashboard.ps1`, `web/src/components/ApiBackendBanner.tsx`, `web/src/components/Layout.tsx`, `STARTUP.md`

- Jedno kliknięcie uruchamia API (:8080) i Vite (:3000); `CLMM_REPO_ROOT` ustawione dla Scripts.
- Gdy backend nie odpowiada, czerwony baner + status w sidebarze zamiast „pustych” zakładek bez wyjaśnienia.

## 2026-04-01 — API + web: `GET /orca/positions-by-owner` (skan NFT jak `orca-positions-list`)

**keywords:** clmm-lp-api, orca_whirlpools, fetch_positions_for_owner, OrcaOwnerPositionsResponse, web, Positions, Wallet, RpcProvider
**paths:** `crates/api/src/handlers/orca_onchain.rs`, `crates/api/src/models.rs`, `crates/api/src/routes.rs`, `crates/api/src/openapi.rs`, `web/src/lib/api.ts`, `web/src/pages/Positions.tsx`, `web/src/pages/Wallet.tsx`, `web/src/components/ApiDataHint.tsx`

- Endpoint RPC dla portfela (query `owner` base58); osobna karta na **Positions**; **Wallet** pokazuje licznik on-chain gdy ustawiony `VITE_DEV_WALLET_PUBKEY`.

## 2026-04-01 — web: wyjaśnienie pustych list (monitor vs Orca), proxy Vite→8080, IL w analytics

**keywords:** web, vite, PositionMonitor, ApiDataHint, PortfolioAnalytics, total_il_pct, React Query
**paths:** `web/src/components/ApiDataHint.tsx`, `web/vite.config.ts`, `web/src/main.tsx`, `web/src/lib/api.ts`, `STARTUP.md`

- Vite proxy domyślnie na **port 8080** (jak API). `PortfolioAnalytics` w UI: **`total_il_pct`** (zgodnie z API), nie `total_il_usd`.
- Cache React Query: **stale 5 min**, **gc 30 min**. Karta **ApiDataHint** na Dashboard / Wallet / Positions — dlaczego brak pozycji przy aktywnych NFT na Orca (monitor ≠ skan portfela).

## 2026-04-01 — web: pinned dev wallet (`VITE_DEV_WALLET_PUBKEY`)

**keywords:** web, vite, VITE_DEV_WALLET_PUBKEY, devWallet, Wallet, Layout
**paths:** `web/src/lib/devWallet.ts`, `web/src/components/DevWalletBar.tsx`, `web/.env.example`, `web/.env.development`, `STARTUP.md`

- UI pokazuje stały pubkey z env; domyślnie `web/.env.development` (Vite dev), nadpisanie przez `web/.env.local`.

## 2026-04-01 — flows: IL ledger + API + web + whETH script (`CLMM_IL_LEDGER_PATH`)

**keywords:** bot-activity, il-ledger, CLMM_IL_LEDGER_PATH, il_ledger_path_from_env, get_bot_il_ledger, BotActivity, PositionDetail, wheth_sol_three_bots
**paths:** `crates/protocols/src/ledger/tx_lifecycle.rs`, `crates/api/src/handlers/bot_activity.rs`, `crates/api/src/routes.rs`, `web/src/pages/BotActivity.tsx`, `web/src/pages/PositionDetail.tsx`, `tools/wheth_sol_three_bots_manual_range_25_25p5.ps1`, `STARTUP.md`, `doc/ORCA_RUNBOOK.md`

- **`il_ledger_path_from_env()`** w protocols; **GET `/bot-activity/il-ledger`** — tail+filter jak lifecycle; brak env → odpowiedź „unset”.
- Web: karta **IL / rebalance ledger** na Bot activity; blok na szczegółach pozycji; `getBotIlLedger` w `api.ts`.
- Skrypt whETH 3× bot: `--il-ledger-path` per strategia (`il_ledger_bot_*.jsonl`).

## 2026-04-01 — IL ledger + CLI: wpisy `rebalance` (stary PDA, koszt tx, session id) + `ledger-rebalance-summary`

**keywords:** LifecycleTracker, RebalanceData, rebalance_session_id, tx_cost_lamports, old_position, ledger-rebalance-summary, CLMM_IL_LEDGER_PATH
**paths:** `crates/execution/src/lifecycle/tracker.rs`, `crates/execution/src/lifecycle/events.rs`, `crates/execution/src/strategy/rebalance.rs`, `crates/cli/src/commands/ledger_rebalance_summary.rs`, `crates/cli/src/main.rs`

- Wiersz JSONL `event: "rebalance"` (IL ledger): dopisane `old_position`, `tx_cost_lamports`, `rebalance_session_id` (z `CLMM_REBALANCE_SESSION_ID` jak w `orca_position_lifecycle.jsonl`).
- Nowa komenda CLI: **`ledger-rebalance-summary`** — zbiórka wierszy `rebalance` z `--il-ledger` / `CLMM_IL_LEDGER_PATH` oraz sumy po `rebalance_session_id` w lifecycle JSONL (`--lifecycle-ledger` lub domyślna ścieżka z env).

## 2026-04-01 — api + web: `POST /positions/{addr}/decrease` — `liquidity_amount` jako string (u128)

**keywords:** clmm-lp-api, DecreaseLiquidityRequest, decrease_liquidity, positions, web
**paths:** `crates/api/src/models.rs`, `crates/api/src/handlers/positions.rs`, `web/src/lib/api.ts`, `web/src/pages/PositionDetail.tsx`

- Ciało JSON: `liquidity_amount` jest **stringiem** dziesiętnym (pełny zakres u128 bez utraty precyzji w przeglądarce). UI: przycisk „Decrease liquidity” na szczegółach pozycji.

## 2026-04-01 — api + web + tools: manifest skryptów, `script_runs.jsonl`, runner localhost, strony Scripts / Wallet, ledger na pozycji

**keywords:** clmm-lp-api, web, scripts-manifest.json, script_runs.jsonl, SCRIPT_RUNNER_URL, tools/script_runner, UI_REQUIREMENTS_PHASE1, bot-activity ledger, rebalance_session_id
**paths:** `crates/api/src/handlers/scripts.rs`, `tools/scripts-manifest.json`, `tools/script_runner/Start-ClmmScriptRunner.ps1`, `web/src/pages/Scripts.tsx`, `web/src/pages/Wallet.tsx`, `doc/UI_REQUIREMENTS_PHASE1.md`

- REST: `GET /api/v1/scripts` (manifest + ostatni run z JSONL), `POST /api/v1/scripts/{id}/run` (proxy do runnera z `SCRIPT_RUNNER_*`). Konfiguracja: `CLMM_REPO_ROOT`, opcjonalnie runner URL/token.
- Runner PS: `tools/script_runner/Start-ClmmScriptRunner.ps1` — allowlista `tools/*.ps1`, zapis wierszy do `data/script_runs.jsonl`.
- Web: `/scripts`, `/wallet`; szczegół pozycji: zakładka timeline z `GET /bot-activity/ledger?filter=` i grupowanie po `rebalance_session_id`; akcje collect / rebalance / decrease / close.

## 2026-04-01 — doc + journal: historia rebalansów, koszty, `--il-ledger-path`, szablon tabeli

**keywords:** rebalance, il_ledger_path, CLMM_REBALANCE_SESSION_ID, orca_position_lifecycle.jsonl, OPERATIONS_JOURNAL, ORCA_RUNBOOK
**paths:** `doc/ORCA_RUNBOOK.md`, `data/experiments/wheth-sol-manual-range-25-25p5/OPERATIONS_JOURNAL.md`

- W `ORCA_RUNBOOK.md` dopisana podsekcja **„Historia rebalansów (liczenie zdarzeń) i kosztów tx”**: `--il-ledger-path` → wiersze `event: "rebalance"`; koszty nadal z lifecycle JSONL + suma po `rebalance_session_id`; eksplorator + opcjonalna tabela w dzienniku.
- W `OPERATIONS_JOURNAL.md` (eksperyment whETH/SOL) sekcja **„Rebalanse — historia i koszty”** z tabelą operatora i odnośnikami.

## 2026-04-01 — tools: `quick_verify_data.ps1` preferuje `target/release|debug/clmm-lp-cli.exe`

**keywords:** tools, quick_verify_data, Resolve-ClmmLpCliExe, cargo run, snapshot-readiness, data-health-check
**paths:** `tools/quick_verify_data.ps1`, `tools/clmm_rpc_tools_helpers.ps1`

- **Problem:** `cargo run` rebuilduje i nadpisuje `target/debug/clmm-lp-cli.exe` — przy uruchomionym procesie (blokada pliku) skrypt padał z „failed to remove … exe”.
- **Zmiana:** jeśli istnieje binarka z `Resolve-ClmmLpCliExe` (najpierw release), używamy jej zamiast `cargo run`; w logu widać linię `Using …clmm-lp-cli.exe`.

## 2026-04-01 — doc: szybki stan aktywnych pozycji + linki do wielu strategii / inspiracji zewnętrznych

**keywords:** POSITION_REGISTRY, OPERATIONAL_CONTINUITY, orca-positions-list, Bot activity, StrategyMode, WHETH_SOL
**paths:** `doc/POSITION_REGISTRY.md`, `doc/OPERATIONAL_CONTINUITY.md`

- W `POSITION_REGISTRY.md` dopisana tabela: **on-chain** (`orca-positions-list`) vs **registry/API** vs **monitor w `orca-bot-run`**; odsyłacze do `BACKTEST_OPTIMIZE_STRATEGIES`, `BOT_OPERATIONS_MODEL`, whETH multi-bot; krótki akapit o wzorcach spoza repo (Orca AI agents, ekosystemy z REST vaultów) przy zachowaniu priorytetu darmowego RPC.
- W `OPERATIONAL_CONTINUITY.md` w **Related** dodany link do `POSITION_REGISTRY.md`.

## 2026-04-01 — execution: rebalans częściowy (close OK, open fail) + monitor bez spamu po zamkniętym PDA

**keywords:** clmm-lp-execution, RebalanceResult, StrategyExecutor, PositionMonitor, rebalance, AccountNotFound
**paths:** `crates/execution/src/strategy/rebalance.rs`, `crates/execution/src/strategy/executor.rs`, `crates/execution/src/monitor/position_monitor.rs`

- **`RebalanceResult::old_position_closed_on_chain`:** ustawiane po udanym `close_position`; gdy `open` się nie uda, `success` pozostaje `false`, ale executor usuwa **stary** PDA z monitora (i czyści `retouch_armed` dla `RetouchShift`), żeby nie logować w kółko `Failed to get account`.
- **Monitor:** jeśli `get_position` zwraca błąd typu „brak konta” (łańcuch zawiera m.in. `AccountNotFound`), pozycja jest **usuwana** z monitora zamiast powtarzać `ERROR` co poll.

## 2026-03-31 — CLI: `orca-bot-open-and-run` zapisuje `registry_open` + `position_open` jak `orca-position-open`

**keywords:** orca-bot-open-and-run, registry.jsonl, orca_position_lifecycle.jsonl, position_registry, try_append_registry_open, WhirlpoolExecutor
**paths:** `crates/cli/src/commands/orca_bot.rs`, `crates/cli/src/commands/position_lifecycle_ledger.rs`

- **Problem:** Open wykonany wyłącznie przez SDK w `run_orca_bot_open_and_run` nie wywoływał ścieżek z `orca-position-open`, więc brakowało wierszy **`registry_open`** oraz **`position_open`** w ledgerze — kolektory oparte na `registry.jsonl` nie widziały NFT.
- **Zmiana:** Po udanym `open_position` dopisywane są te same rekordy co po CLI (`try_append_position_open_cost_ledger` z `source: "orca_bot"`, `try_append_registry_open(..., "orca_bot", ...)`). `try_append_position_open_cost_ledger` przyjmuje jawny parametr `source` (`"cli"` z `orca-position-open`).

## 2026-03-31 — doc: whETH/SOL — 2× bot ~10 USD, zakres 25–25,5, dziennik i koszty

**keywords:** WHETH_SOL, two-bots, 10USD, oor_recenter, periodic, ledger, ORCA_RUNBOOK, journal
**paths:** `doc/WHETH_SOL_TWO_BOTS_10USD.md`, `doc/WHETH_SOL_THREE_BOTS_FIRST_RUN.md`

- Plan operacyjny: **dwie** strategie (np. `winner-A` + `winner-B`), deploy **~10 USD** (ok. **5 USD/pozycja** heurystycznie), reszta salda na **fee rebalansów**; ticki **[-55416,-55216]**; monitoring/rebalans przez `orca-bot-run` z **`--execute`**. Dopisany przewodnik: tabele capów (przykład), ledger (`orca_position_lifecycle.jsonl`, registry, `CLMM_REBALANCE_SESSION_ID`), szablon dziennika.

## 2026-03-31 — doc: STARTUP — kolejność `KEYPAIR_PATH` / domyślny plik bota mainnet

**keywords:** KEYPAIR_PATH, SOLANA_KEYPAIR_PATH, clmm_lp_bot_mainnet.json, STARTUP, operations
**paths:** `STARTUP.md` (sekcja Bot keypair)

- Dopisane: **resolution order** dla skryptów/CLI, przypomnienie o braku commitu keypaira, **szybki health check** (istnienie pliku + JSON 64 elementów / opcjonalnie `solana-keygen pubkey`).

## 2026-03-31 — ops: whETH/SOL 3× bot — ręczny zakres 25–25.5 SOL/whETH + JSON strategii

**keywords:** orca-bot-open-and-run, optimize-result-json, WHETH_SOL, oor_recenter, periodic, threshold, tools
**paths:** `data/experiments/wheth-sol-manual-range-25-25p5/winner-*.json`, `tools/wheth_sol_three_bots_manual_range_25_25p5.ps1`, `doc/WHETH_SOL_THREE_BOTS_FIRST_RUN.md`

- Dla puli `Hktf…` (mint A = SOL, mint B = whETH) ticki **[-55416, -55216]** odpowiadają w przybliżeniu **25.0–25.5 SOL za 1 whETH** (weryfikacja: `orca-pool-read`). Trzy pliki `OptimizeResultFile` (schema v1): **oor_recenter**, **periodic 12h**, **threshold 5%**; `width_pct` = **0.02** (~2%) dla spójności z szerokością pasma. Skrypt PowerShell drukuje gotowe komendy `orca-bot-open-and-run` (osobne terminale).

## 2026-03-31 — API: `POST /analytics/simulate` — pełna symulacja (`clmm_lp_simulation`)

**keywords:** clmm-lp-api, analytics, simulate, clmm_lp_simulation, GBM, WhirlpoolReader
**paths:** `crates/api/src/services/simulation_analytics.rs`, `crates/api/src/handlers/analytics.rs`, `crates/api/src/models.rs`, `web/src/lib/api.ts`

- Endpoint **nie jest już placeholderem**: pobiera stan puli z RPC (`WhirlpoolReader`), buduje zakres z `tick_to_price`, generuje **syntetyczną ścieżkę dzienna GBM** od aktualnej ceny, uruchamia `simulate_with_strategy` (strategie z `StrategyType` + domyślne progi), zwraca m.in. `time_in_range_pct`, `vs_hodl_usd`, `methodology_note`.
- `SimulationRequest` rozszerzony o `gbm_volatility` / `gbm_drift` / parametry strategii; `SimulationResponse` o pola jawnej metodologii. Historyczny replay jak `backtest` CLI nadal osobno — opis w `methodology_note`.

## 2026-03-31 — doc: WHETH 3× bot — CLI vs API optimize + symulacja zakresu

**keywords:** backtest-optimize, clmm-lp-cli, clmm-lp-api, analytics simulate, WHETH_SOL_FIRST_RUN, optimize_profile
**paths:** `doc/WHETH_SOL_THREE_BOTS_FIRST_RUN.md`, `crates/api/src/handlers/analytics.rs`, `crates/api/src/services/optimization_runner.rs`

- Dopisane sekcje: **skąd jest optimize** (siatka tylko w CLI; API = subprocess `backtest-optimize` lub `apply-optimize-result`; `POST /analytics/simulate` = placeholder), oraz **procedura symulacji doboru `width_pct`/parametrów** przed openami.

## 2026-03-31 — API + web: historia bota (JSONL) + Slack digest

**keywords:** clmm-lp-api, web, bot-activity, Slack, SLACK_WEBHOOK_URL, orca_position_lifecycle.jsonl, registry.jsonl, OPERATIONAL_CONTINUITY
**paths:** `crates/api/src/handlers/bot_activity.rs`, `web/src/pages/BotActivity.tsx`, `web/src/lib/api.ts`, `doc/OPERATIONAL_CONTINUITY.md`

- **GET** `/api/v1/bot-activity/ledger` i `/registry` — ostatnie wiersze JSONL (opcjonalnie `?filter=` substring, `?limit=`), te same ścieżki co CLI (`CLMM_POSITION_LIFECYCLE_LEDGER_PATH`, `CLMM_POSITION_REGISTRY_PATH`).
- **POST** `/api/v1/bot-activity/slack-summary` — treść z ogonu ledgera → Incoming Webhook (`SLACK_WEBHOOK_URL` w env procesu API).
- **Web:** strona **Bot activity** (`/bot-activity`) — tabele + przycisk wysyłki na Slack.

## 2026-03-31 — doc: pierwsze uruchomienie 3× bot whETH/SOL (strategie + dziennik)

**keywords:** WHETH_SOL, orca-bot-run, StrategyMode, BOT_OPERATIONS_MODEL, ORCA_RUNBOOK, operations
**paths:** `doc/WHETH_SOL_THREE_BOTS_FIRST_RUN.md`, `doc/ORCA_RUNBOOK.md`

- Nowy przewodnik: propozycja zestawu strategii (`oor_recenter` / `periodic` / `threshold` + alternatywy), bezpieczna sekwencja (dry-run → jeden live na raz), szablon tabeli dziennika i checklist; link z `ORCA_RUNBOOK.md`.

## 2026-03-31 — tools: `orca_wheth_sol_three_bots_plan.ps1` (plan 3× pozycja WHETH/SOL)

**keywords:** tools, WHETH_SOL, orca_curated_rebalance, orca-bot-run, capital plan, SCRIPTS_CATALOG
**paths:** `tools/orca_wheth_sol_three_bots_plan.ps1`, `doc/SCRIPTS_CATALOG.md`

- Skrypt **nie wysyła tx**: czyta `solana_account_state.ps1 -Json`, ceny CoinGecko (SOL/ETH), liczy heurystyczne **`AmountA`/`AmountB` na bot** (DeployUsd/NumBots, split 50/50 USD na nogi), sprawdza braki względem **3×** caps + `ReserveSolLamports`; drukuje przykładowe komendy `orca_curated_rebalance -Action Open` i `orca-bot-run`.

## 2026-03-31 — tools: `solana_wallet_usd_estimate.ps1` (portfel w USD)

**keywords:** tools, solana_wallet_usd_estimate, solana_account_state, CoinGecko, USDC, portfolio, SCRIPTS_CATALOG
**paths:** `tools/solana_wallet_usd_estimate.ps1`, `doc/SCRIPTS_CATALOG.md`

- Skrypt woła **`solana_account_state.ps1 -Json`**, sumuje **native SOL + wSOL (ATA)** jako jedną linię, SPL po **mincie**; ceny: **USDC/USDT = 1 USD**, **SOL / cbBTC (jako BTC) / whETH (jako ETH)** z **CoinGecko** `simple/price`; minty bez mapowania → **0 USD** + `unpriced_mint`. Opcja **`-Json`** (jedna linia), **`-SkipPriceFetch`** tylko kwoty UI.

## 2026-03-31 — tools: `orca_curated_rebalance.ps1` + `-OpenOnly` w `orca_position_open_then_close_quick.ps1`

**keywords:** tools, orca_curated_rebalance, orca_swap_curated, orca_position_open_then_close_quick, OpenOnly, curated, ORCA_RUNBOOK, SCRIPTS_CATALOG
**paths:** `tools/orca_curated_rebalance.ps1`, `tools/orca_position_open_then_close_quick.ps1`, `doc/ORCA_RUNBOOK.md`, `doc/SCRIPTS_CATALOG.md`

- **Dispatcher** dla trzech par curated (`SOL_USDC`, `WHETH_SOL`, `CBBTC_USDC`): akcje Help / ListPairs / Preflight / Open (samo `orca-position-open`) / Close / Swap (forward do `orca_swap_curated.ps1`) / FundCbBtc (`orca_fund_cbbtc_usdc_open.ps1`) / Smoke (`orca_position_smoke_curated_pools.ps1`).
- **`orca_position_open_then_close_quick.ps1 -OpenOnly`:** po udanym open pomija sleep/close/verify — pozycja zostaje na łańcuchu (rebalans / produkcja).

## 2026-03-31 — tools: uniwersalny build CLI (`build_clmm_lp_cli.ps1`) + wrapper release

**keywords:** tools, powershell, build_clmm_lp_cli, build_clmm_lp_cli_release, clmm-lp-cli, cargo, SCRIPTS_CATALOG, ORCA_RUNBOOK
**paths:** `tools/build_clmm_lp_cli.ps1`, `tools/build_clmm_lp_cli_release.ps1`, `doc/SCRIPTS_CATALOG.md`, `doc/ORCA_RUNBOOK.md`

- **`build_clmm_lp_cli.ps1`:** `cargo build` dla **`clmm-lp-cli`** z **`-Configuration Release`** (domyślnie) lub **`Debug`**; wypisuje ścieżkę do `target\release|debug\clmm-lp-cli.exe`.
- **`build_clmm_lp_cli_release.ps1`:** cienki wrapper wołający Release — zachowane stare odwołania w skryptach i doku.

## 2026-03-31 — tools: `orca_fund_cbbtc_usdc_open.ps1` — dopłaty po swapach + wyższy domyślny bufor USDC

**keywords:** tools, orca_fund_cbbtc_usdc_open, UsdcHeadroomBps, post-swap, preflight, cbBTC, USDC, ORCA_RUNBOOK
**paths:** `tools/orca_fund_cbbtc_usdc_open.ps1`

- Po zaplanowanych swapach **SOL/USDC** i **cbBTC/USDC** skrypt w pętli (domyślnie do **6** rund, param **`PostSwapTopUpMaxRounds`**) ponownie liczy preflight; jeśli brakuje tylko **USDC**, robi **exact-out USDC** na puli SOL/USDC z buforem **`UsdcHeadroomBps`**; jeśli brakuje tylko **cbBTC**, robi **exact-out cbBTC** na puli cbBTC/USDC — żeby zamknąć lukę między dry-run quote a faktycznym kosztem on-chain.
- Domyślne **`UsdcHeadroomBps`** podniesione **300 → 600** (nadal nadpisywalne z CLI).

## 2026-03-31 — tools: `orca_swap.ps1` — `Start-Process` zamiast `& cargo 2>&1` (stderr / exit code)

**keywords:** tools, powershell, orca_swap, orca_fund_cbbtc_usdc_open, cargo, Start-Process, NativeCommandError, ORCA_RUNBOOK
**paths:** `tools/orca_swap.ps1`

- Wywołanie `clmm-lp-cli` (exe lub `cargo run`) idzie przez **Start-Process** z przekierowaniem stdout/stderr do plików tymczasowych; linie są potem drukowane przez **Write-Host**.
- Cel: komunikaty cargo/rust na stderr nie trafiają do strumienia błędów hosta jako **NativeCommandError**, a kod wyjścia pochodzi z **ExitCode** procesu (stabilniejsze niż `$LASTEXITCODE` po merge `2>&1` w zagnieżdżonych `-File`), m.in. dla `orca_fund_cbbtc_usdc_open.ps1 -Execute`.

## 2026-03-31 — tools + ops: `data_alerts_loop.ps1` + alternatywy dla Task Scheduler

**keywords:** tools, data_alerts_loop, Shawl, NSSM, systemd, snapshot_health_alert, quick_verify_alert, OPERATIONAL_CONTINUITY
**paths:** `tools/data_alerts_loop.ps1`, `doc/OPERATIONAL_CONTINUITY.md`, `tools/README.md`, `deploy/systemd/clmm-lp-data-alerts-loop.service.example`, `deploy/README.md`, `doc/SCRIPTS_CATALOG.md`

- **`data_alerts_loop.ps1`:** jeden długo żyjący proces z interwałami snapshot vs quick-verify; log `data/snapshot_logs/data-alerts-loop.log`; zamiennik harmonogramu Windows.
- **Dokumentacja:** Shawl/NSSM, opcjonalnie Task Scheduler; Linux: `systemd` + `pwsh` (przykładowa jednostka w `deploy/systemd/`).

## 2026-03-31 — tools: `quick_verify_alert.ps1` (GO/NO-GO → Slack + throttle)

**keywords:** tools, quick_verify_data, quick_verify_alert, slack, snapshot-readiness, data-health-check, SCRIPTS_CATALOG
**paths:** `tools/quick_verify_alert.ps1`, `doc/SCRIPTS_CATALOG.md`, `tools/README.md`, `doc/OPERATIONAL_CONTINUITY.md`

- Wrapper woła `quick_verify_data.ps1`; przy exit≠0 (w tym **exit 2** NO-GO) wysyła `notify_slack_webhook.ps1`; throttle `MinMinutesBetweenSameIssues` (domyślnie **60 min**) w `data/agent-alerts/quick-verify-slack-throttle/`; catch na throw z `quick_verify_data`.

## 2026-03-31 — doc + tools: katalog skryptów (`SCRIPTS_CATALOG`) + `snapshot_health_alert` → Slack

**keywords:** scripts, SCRIPTS_CATALOG, snapshot-health, snapshot_health_alert, notify_slack_webhook, tools/README, OPERATIONAL_CONTINUITY
**paths:** `doc/SCRIPTS_CATALOG.md`, `doc/README.md`, `doc/OPERATIONAL_CONTINUITY.md`, `tools/README.md`, `tools/snapshot_health_alert.ps1`

- **`doc/SCRIPTS_CATALOG.md`:** spis `tools/*.ps1` z kolumną keywords, sekcja P0 (snapshot/ciągłość/jakość), CLI powiązane, uwaga na `scripts/` w `.gitignore`.
- **`tools/snapshot_health_alert.ps1`:** woła `snapshot_health_check.ps1`; przy exit≠0 Slack przez `notify_slack_webhook.ps1`; throttle `MinMinutesBetweenSameIssues` (domyślnie 15 min); stan w `data/agent-alerts/snapshot-slack-throttle/`.
- **`tools/README.md`:** skrót + link do katalogu.

## 2026-03-31 — tools: `orca_fund_cbbtc_usdc_open.ps1` (quote dry-run + opcjonalne swapy pod open)

**keywords:** tools, powershell, orca_fund_cbbtc_usdc_open, orca-swap, dry-run, quote, cbBTC, USDC, SOL_USDC, ORCA_RUNBOOK
**paths:** `tools/orca_fund_cbbtc_usdc_open.ps1`, `doc/ORCA_RUNBOOK.md`

Skrypt szacuje braki do `orca-position-open` na puli **cbBTC/USDC** (`HxA6…`): parsuje `token_est_in` / `token_est_out` z stdout `orca-swap --dry-run`, planuje **SOL/USDC** exact-out USDC potem **cbBTC/USDC** exact-out cbBTC; **`-Execute`** woła `orca_swap.ps1`.

## 2026-03-31 — tools: curated Orca swapy (3 pary, wszystkie nogi) + wspólna lista pul

**keywords:** tools, powershell, orca_swap_curated, orca_curated_mainnet_pools, orca_swap, CargoOnly, SOL_USDC, WHETH_SOL, CBBTC_USDC, ORCA_RUNBOOK
**paths:** `tools/orca_curated_mainnet_pools.ps1`, `tools/orca_swap_curated.ps1`, `tools/orca_swap.ps1`, `tools/orca_position_smoke_curated_pools.ps1`, `doc/ORCA_RUNBOOK.md`

- **`orca_curated_mainnet_pools.ps1`:** jedna definicja trzech pul (mint_a/b, symbole) zgodna z `orca-pool-read`.
- **`orca_swap_curated.ps1`:** `-From`/`-To` + `-SwapType` → `--specified-mint` / `exact-in|exact-out`; `-ListPairs`.
- **`orca_swap.ps1`:** `-CargoOnly`, preferencja `Resolve-ClmmLpCliExe`, błąd przy niezerowym `LASTEXITCODE`.
- **Smoke curated** buduje listę pul z `orca_curated_mainnet_pools.ps1`.

## 2026-03-31 — tools: Slack Incoming Webhook helper (`notify_slack_webhook.ps1`)

**keywords:** tools, powershell, slack, SLACK_WEBHOOK_URL, alerts, OPERATIONAL_CONTINUITY
**paths:** `tools/notify_slack_webhook.ps1`, `doc/OPERATIONAL_CONTINUITY.md`

- Skrypt wysyła `text` przez **Incoming Webhook** (kolejność: `-WebhookUrl`, env `SLACK_WEBHOOK_URL`, potem parsowanie **`SLACK_WEBHOOK_URL=` z repo-root `.env`**; opcjonalnie `-DotEnvPath`). Instrukcja Slack w `doc/OPERATIONAL_CONTINUITY.md` (sekcja Slack).

## 2026-03-31 — tools: `solana_account_state.ps1 -Json` jako jedna linia (parsowanie preflight)

**keywords:** tools, powershell, solana_account_state, ConvertTo-Json, orca_position_preflight_core, preflight
**paths:** `tools/solana_account_state.ps1`

Tryb `-Json` wypisywał wieloliniowy JSON; `orca_position_open_preflight.ps1` brał pierwszą linię zaczynającą się od `{` (sam `{`), więc `ConvertFrom-Json` padał. Na stdout **-Json** używa teraz `ConvertTo-Json -Compress` (jedna linia); zapis `-OutJson` nadal pretty-print.

## 2026-03-31 — tools: auto-fund (exact-out swap) przed Orca position open

**keywords:** tools, powershell, orca_position_preflight_core, Invoke-OrcaPositionAutoFundFromPool, AutoFund, orca-swap, exact-out, orca_position_auto_fund_for_open, orca_position_open_then_close_quick, orca_position_open_then_close_fast, orca_position_smoke_curated_pools, ORCA_RUNBOOK
**paths:** `tools/orca_position_preflight_core.ps1`, `tools/orca_position_open_preflight.ps1`, `tools/orca_position_auto_fund_for_open.ps1`, `tools/orca_position_open_then_close_quick.ps1`, `tools/orca_position_open_then_close_fast.ps1`, `tools/orca_position_smoke_curated_pools.ps1`, `doc/ORCA_RUNBOOK.md`

- **`orca_position_preflight_core.ps1`:** `Get-OrcaPositionOpenPreflightState`, `Test-OrcaPositionOpenPreflight`, `Invoke-OrcaPositionAutoFundFromPool` (pętla exact-out na tej samej puli do momentu OK preflightu).
- **`orca_position_open_preflight.ps1`:** tylko param + standalone; wspólna logika w core.
- **`orca_position_auto_fund_for_open.ps1`:** samo auto-fund + końcowy test preflight (bez `orca-position-open`).
- **Quick / fast / smoke:** opcjonalnie **`-AutoFund`** i parametry bufora/slippage/max rund.
- **`doc/ORCA_RUNBOOK.md`:** pod auto-fund dopisane **planowanie swapów** (stała kolejność A-then-B, zapas nogi płacącej, dual-deficit, SOL reserve).

## 2026-03-31 — tools: preflight open + cbBTC/USDC w smoke curated

**keywords:** tools, powershell, orca_position_open_preflight, orca_position_open_then_close_quick, orca_position_open_then_close_fast, orca_position_smoke_curated_pools, SkipPreflight, ReserveSolLamports, cbBTC, ORCA_RUNBOOK
**paths:** `tools/orca_position_open_preflight.ps1`, `tools/orca_position_open_then_close_quick.ps1`, `tools/orca_position_open_then_close_fast.ps1`, `tools/orca_position_smoke_curated_pools.ps1`, `doc/ORCA_RUNBOOK.md`

- `orca_position_open_preflight.ps1`: prawdziwy mint **cbBTC** w etykietach; blok standalone uruchamia się tylko gdy skrypt **nie** jest dot-sourced (`$MyInvocation.InvocationName -ne '.'`), żeby `. .\…preflight.ps1` nie robił `Set-Location`/`exit` w sesji nadrzędnej.
- **Quick** i **fast** open→close: przed `orca-position-open` domyślnie wywołanie preflightu; `-SkipPreflight`, `-ReserveSolLamports` (przekazywane także ze smoke).
- **Smoke curated:** trzeci pool `cbBTC/USDC` (`HxA6SKW5qA4o12fjVgTpXdq2YnZ5Zv1s7SB4FFomsyLM`) jak w `STARTUP.md`.
- **`doc/ORCA_RUNBOOK.md`:** smoke + preflight + usunięcie sprzecznej uwagi „HxA6 nie z STARTUP”.

## 2026-03-31 — ops: ciągłość operacyjna bota (dokument + systemd + Docker + supervised PS1)

**keywords:** operations, orca-bot-run, systemd, docker-compose, Task Scheduler, OPERATIONAL_CONTINUITY, orca_bot_run_supervised
**paths:** `doc/OPERATIONAL_CONTINUITY.md`, `doc/ORCA_RUNBOOK.md`, `doc/README.md`, `doc/MAINNET_OPERATIONAL_CHECKLIST.md`, `deploy/systemd/clmm-lp-orca-bot.service.example`, `deploy/README.md`, `Docker/orca-bot.compose.example.yml`, `Docker/README.md`, `tools/orca_bot_run_supervised.ps1`

- Nowy runbook: **`doc/OPERATIONAL_CONTINUITY.md`** (superwizja procesu, logi, haki alertów, RPC/klucze, checklist).
- **Linux:** szablon `deploy/systemd/clmm-lp-orca-bot.service.example`.
- **Docker:** przykład `Docker/orca-bot.compose.example.yml` (`restart: unless-stopped`, volume na keypair + ledgery).
- **Windows:** `tools/orca_bot_run_supervised.ps1` — pętla restartu po niezerowym exit code; opcjonalnie `-LogDir` (ostrzeżenie dla Windows PowerShell 5.x w kwestii `$LASTEXITCODE` po `Tee-Object`).

## 2026-03-31 — tools: Orca quick (release exe) + `orca_position_smoke_curated_pools` + helper w swap/snapshot verify

**keywords:** tools, powershell, Resolve-ClmmLpCliExe, Invoke-ClmmLpCliStream, orca_position_open_then_close_quick, orca_position_smoke_curated_pools, build_clmm_lp_cli_release, orca_swap, quick_verify_data, run_snapshot_backtest_prep_loop, ORCA_RUNBOOK
**paths:** `tools/clmm_rpc_tools_helpers.ps1`, `tools/orca_position_open_then_close_quick.ps1`, `tools/orca_position_smoke_curated_pools.ps1`, `tools/build_clmm_lp_cli_release.ps1`, `tools/orca_swap.ps1`, `tools/quick_verify_data.ps1`, `tools/run_snapshot_backtest_prep_loop.ps1`, `doc/ORCA_RUNBOOK.md`

- Rozszerzono `clmm_rpc_tools_helpers.ps1` o **Resolve-ClmmLpCliExe**, **Invoke-ClmmLpCliStream**, **Invoke-ClmmLpCliCapture**; `orca_position_open_then_close_quick.ps1` używa release/debug exe gdy istnieje (`-CargoOnly` → zawsze `cargo run`).
- Nowe: **`tools/orca_position_smoke_curated_pools.ps1`** (open+close dla pooli jak w `STARTUP.md` Orca), **`tools/build_clmm_lp_cli_release.ps1`**.
- **Initialize-ClmmToolsRpcEnv** także w `orca_swap.ps1`, `quick_verify_data.ps1`, `run_snapshot_backtest_prep_loop.ps1`.
- **`doc/ORCA_RUNBOOK.md`:** sekcja smoke + rozróżnienie poola z `quick_verify` vs curated.

## 2026-03-31 — tools: Orca ops — `clmm_rpc_tools_helpers.ps1` + close slippage w quick + hint 6018

**keywords:** tools, powershell, CLMM_RPC_DENYLIST, orca-position-close, slippage_bps, execution_ok, TokenMinSubceeded
**paths:** `tools/clmm_rpc_tools_helpers.ps1`, `tools/orca_position_close_quick.ps1`, `tools/orca_position_open_then_close_quick.ps1`, `tools/orca_position_open_then_close_fast.ps1`, `crates/cli/src/commands/orca_position.rs`

- `Initialize-ClmmToolsRpcEnv`: gdy `CLMM_RPC_DENYLIST` jest puste i `SOLANA_RPC_URL` wygląda na mainnet, ustawia `ankr,projectserum`, żeby domyślne fallbacki w `RpcConfig` omijały często blokowane URL-e.
- `orca_position_close_quick.ps1` / `orca_position_open_then_close_quick.ps1`: domyślnie wyższy slippage na close (`-SlippageBps` / `-CloseSlippageBps`, 500 bps).
- `execution_ok` dopina hint przy tekście błędu z **6018** / **0x1782**.

## 2026-03-31 — tools: `restart_snapshot_loop_10m.ps1` (pin RPC; nie dotyka 5m)

**keywords:** tools, powershell, snapshot-loop, run-snapshot-loop, SOLANA_RPC_URL, snapshot_logs
**paths:** `tools/restart_snapshot_loop_10m.ps1`, `scripts/windows/run-snapshot-loop.ps1`

Skrypt zatrzymuje proces PowerShell uruchomiony z `scripts/windows/run-snapshot-loop.ps1` (bez `run-snapshot-loop-5m.ps1`) i startuje pętlę ponownie z domyślnym pinem RPC jak w skrypcie. Jeśli stara pętla działa pod Task Scheduler/NSSM w innej sesji, wyłącz duplikat ręcznie.

## 2026-03-30 — RPC: hard-disable paid/auth endpoints + optional denylist guard

**keywords:** clmm-lp-protocols, rpc, failover, health, 402, Payment Required, denylist, SOLANA_RPC_URL, SOLANA_RPC_FALLBACK_URLS, CLMM_RPC_DENYLIST
**paths:** `crates/protocols/src/rpc/provider.rs`, `crates/protocols/src/rpc/health.rs`, `crates/protocols/src/rpc/config.rs`

RPC failover now **hard-disables** endpoints that return HTTP auth/paywall failures (402/401/403) to avoid repeated rotation into dead URLs causing snapshot gaps. Added optional env `CLMM_RPC_DENYLIST` (comma-separated substrings) to filter such endpoints up-front, plus a startup warning when only one endpoint remains after config/denylist.

## 2026-03-31 — CLI: `orca-position-close --slippage-bps` (Whirlpool 6018 / TokenMinSubceeded)

**keywords:** clmm-lp-cli, orca-position-close, WhirlpoolExecutor, close_position_instructions, slippage_bps, TokenMinSubceeded, 6018, tools, orca_position_open_then_close_fast.ps1
**paths:** `crates/protocols/src/orca/executor.rs`, `crates/cli/src/commands/orca_position.rs`, `crates/cli/src/main.rs`, `crates/api/src/services/orca_tx_service.rs`, `tools/orca_position_open_then_close_fast.ps1`

`WhirlpoolExecutor::close_position` przyjmuje opcjonalny `slippage_bps` (domyślnie jak wcześniej: **100** bps przez `None` / brak flagi). CLI `orca-position-close` ma `--slippage-bps`; `ClosePositionTxRequest` ma `slippage_bps: Option<u16>`. Skrypt `orca_position_open_then_close_fast.ps1` przekazuje domyślnie **500** bps na close (`-CloseSlippageBps`), żeby unikać błędu on-chain **6018** (*TokenMinSubceeded*) przy bardzo małej płynności / szybkim open→close.

## 2026-03-30 — tools: `orca_position_open_then_close_fast.ps1` (close bez czekania na ledger)

**keywords:** tools, powershell, orca-position-open, orca-position-close, timing, confirm->confirm, ledger, getTransaction
**paths:** `tools/orca_position_open_then_close_fast.ps1`

Dodano skrypt `tools/orca_position_open_then_close_fast.ps1`, który startuje `close` natychmiast po wypisaniu `position PDA:` z `open` (nie czeka na post-tx enrichment ledgera, który na public RPC potrafi lagować), i mierzy czas confirm->confirm z timestampów logów `Transaction confirmed signature=...`.

## 2026-03-30 — tools: `orca_position_open_then_close_quick.ps1` mierzy czas confirm->confirm

**keywords:** tools, powershell, orca-position-open, orca-position-close, automation, timing, confirm->confirm
**paths:** `tools/orca_position_open_then_close_quick.ps1`

Skrypt `tools/orca_position_open_then_close_quick.ps1` został rozszerzony o pomiar czasu pomiędzy momentem pojawienia się `signature:` dla open a dla close (na podstawie streamingowego odczytu stdout/stderr cargo), żeby nie mieszać tego z dodatkowym enrichmentem ledgera.

## 2026-03-30 — tools: `orca_position_close_quick.ps1` (szybkie zamknięcie z registry)

**keywords:** tools, powershell, orca-position-close, registry.jsonl, position_registry, automation
**paths:** `tools/orca_position_close_quick.ps1`, `crates/protocols/src/ledger/position_registry.rs`

Dodano skrypt `tools/orca_position_close_quick.ps1`: wybiera ostatnio aktywną pozycję (`registry_open` bez późniejszego `registry_close`) dla właściciela i odpala jedną komendę `clmm-lp-cli orca-position-close` z gotowym `--position` i `--keypair`.

## 2026-03-30 — tools: `orca_position_open_then_close_quick.ps1` (open→close jednym kliknięciem)

**keywords:** tools, powershell, orca-position-open, orca-position-close, automation, Whirlpool
**paths:** `tools/orca_position_open_then_close_quick.ps1`

Dopisano skrypt automatyzujący flow: `orca-position-open` (live, małe `--amount-a/--amount-b`) → parsowanie `position PDA` z outputu → krótki sleep → `orca-position-close` oraz opcjonalna weryfikacja `orca-positions-list entries=0`.

## 2026-03-30 — CLI: `orca-position-close` dopisuje token refund delty (A/B) do ledgera

**keywords:** clmm-lp-cli, orca-position-close, position_lifecycle_ledger, jsonl, token_delta, preTokenBalances, postTokenBalances
**paths:** `crates/cli/src/commands/position_lifecycle_ledger.rs`

Do wierszy `event=position_close` w `data/ledger/orca_position_lifecycle.jsonl` dopisano best-effort delty tokenów A/B (`token_a_net_delta_*`, `token_b_net_delta_*`) jako `post - pre` dla fee-payera (owner) liczonych z `meta.preTokenBalances`/`meta.postTokenBalances`. Dzięki temu “zwroty” są widoczne w ilościach (base units + UI), obok dotychczasowych kosztów SOL/fees.

## 2026-03-30 — Orca: rozróżnienie pool (653 B) vs position PDA (216 B) + komunikat w `PositionReader`

**keywords:** clmm-lp-protocols, clmm-lp-cli, orca-position-close, Whirlpool, OpenPositionWithTokenExtensions, position_reader, pool vs position
**paths:** `crates/protocols/src/orca/position_reader.rs`, `crates/cli/src/main.rs`, `doc/POSITION_REGISTRY.md`

`PositionReader::get_position` wykrywa podanie konta **puli** Whirlpool (653 B + discriminator puli) zamiast **PDA pozycji** (216 B) i zwraca czytelny błąd (kolejność kont w `OpenPositionWithTokenExtensions` na Solscan). `doc/POSITION_REGISTRY.md` — sekcja „Pula vs PDA”; help CLI `orca-position-close --position` doprecyzowany.

## 2026-03-30 — STARTUP: Shawl/NSSM — druga usługa dla pętli snapshotów 5m

**keywords:** STARTUP, Shawl, NSSM, Windows Service, run-snapshot-loop, run-snapshot-loop-5m, snapshots_5m, snapshot-loop-5m.log
**paths:** `STARTUP.md`

W sekcji *Alternatives to Task Scheduler* dopisano **drugą** usługę równoległą do `clmm-snapshot-loop`: **`clmm-snapshot-loop-5m`** (`run-snapshot-loop-5m.ps1` → `snapshots_5m.jsonl`, log `snapshot-loop-5m.log`). Tabela NSSM i osobne `shawl add` dla 10m vs 5m; ścieżki nadal wymagają dopasowania do lokalnego klonu.

## 2026-03-30 — `position_registry.jsonl`: otwarte/zamknięte pozycje + sygnał dla kolektorów

**keywords:** clmm-lp-protocols, clmm-lp-cli, clmm-lp-execution, position_registry, registry_open, registry_close, collectors, jsonl, CLMM_POSITION_REGISTRY_PATH
**paths:** `crates/protocols/src/ledger/position_registry.rs`, `crates/cli/src/commands/orca_position.rs`, `crates/execution/src/strategy/rebalance.rs`, `doc/POSITION_REGISTRY.md`

Dodano append-only **`data/positions/registry.jsonl`** (`CLMM_POSITION_REGISTRY_PATH`): `registry_open` / `registry_close`, `source` cli vs `orca_bot`, opcjonalnie `rebalance_session_id`. CLI `orca-position-open` / `close` oraz udane open/close w **`RebalanceExecutor`** dopisują wiersze — kolektory mogą wyliczać aktywne pozycje (ostatni event per `position_pubkey`) i **kończyć** zbieranie danych per pozycja po `registry_close`. Dokumentacja: `doc/POSITION_REGISTRY.md`.

## 2026-03-30 — ORCA_RUNBOOK: rebalance (ticki), swap vs `RebalanceExecutor`, `CLMM_REBALANCE_SESSION_ID`

**keywords:** ORCA_RUNBOOK, rebalance, Whirlpool, tick range, close position, open position, orca-swap, CLMM_REBALANCE_SESSION_ID, RebalanceExecutor
**paths:** `doc/ORCA_RUNBOOK.md`

Rozszerzono runbook: **immutable** zakres ticków na jednym NFT Whirlpool → typowy flow collect → decrease → close → open (nowy PDA); alternatywa dwóch pozycji; **`RebalanceExecutor`** bez wbudowanego swapu — swap przez CLI/skrypt + ledger `cli_swap`; **`CLMM_REBALANCE_SESSION_ID`** jako spójne sumowanie kosztów w jednej sesji; przyszłość: id sesji z konfiguracji/UUID w Rust zamiast wyłącznie env.

## 2026-03-30 — Ledger: `cli_swap` + `CLMM_REBALANCE_SESSION_ID` (pełny koszt swap + rebalans + open)

**keywords:** clmm-lp-protocols, clmm-lp-cli, orca-swap, tx_lifecycle, rebalance_session_id, jsonl, fee_payer_net_lamports_delta
**paths:** `crates/protocols/src/ledger/tx_lifecycle.rs`, `crates/cli/src/commands/orca_swap.rs`, `crates/cli/src/commands/position_lifecycle_ledger.rs`

Po udanym **`orca-swap`** dopisywany jest wiersz do tego samego pliku co lifecycle (`event=cli_swap`, `operation=orca_whirlpool_swap`, `source=cli`). Opcjonalnie **`CLMM_REBALANCE_SESSION_ID`** (to samo wartość w całej sekwencji: swap → close/open → bot) jest zapisywane do **`rebalance_session_id`** na wierszach: `cli_swap`, `position_open` / `position_close`, oraz `orca_bot` (rebalance executor) — suma **`tx_fee_lamports`** lub delt płatnika po tym samym id daje **całościowy** koszt operacji łączonej.

## 2026-03-30 — protocols + execution: rebalance tx lifecycle ledger (`orca_bot`)

**keywords:** clmm-lp-protocols, clmm-lp-execution, rebalance, tx_lifecycle, ledger, jsonl, orca_bot, position_lifecycle, enrich_tx_costs
**paths:** `crates/protocols/src/ledger/tx_lifecycle.rs`, `crates/cli/src/commands/position_lifecycle_ledger.rs`, `crates/execution/src/strategy/rebalance.rs`

Shared append-only JSONL path (`data/ledger/orca_position_lifecycle.jsonl`, same env vars as CLI) and **`enrich_tx_costs`** (RPC `getTransaction` + `meta.fee` + fee payer `preBalances`/`postBalances`) live in **`clmm_lp_protocols::ledger::tx_lifecycle`**. After successful Orca ops in **`RebalanceExecutor`** (`collect_fees`, `decrease_liquidity`, `close_position`, `open_full_range_position`, `open_position`), a row is appended with **`source=orca_bot`**, **`event=bot_*`**, **`operation`** (internal op name), optional **`pool_address`** / **`position_pubkey`** (open flows fill position from `created_position`). CLI lifecycle rows add **`source=cli`** on the same **schema_version=2** file.

## 2026-03-30 — CLI: ledger cyklu życia pozycji Orca (`orca_position_lifecycle.jsonl`)

**keywords:** clmm-lp-cli, orca-position-open, orca-position-close, position_lifecycle_ledger, jsonl, meta.fee, preBalances, postBalances, fee_payer_net_lamports_delta, mint
**paths:** `crates/cli/src/commands/position_lifecycle_ledger.rs`, `crates/cli/src/commands/orca_position.rs`

Po **udanym** `orca-position-open` i `orca-position-close` dopisywany jest wiersz JSONL (**schema_version=2**): domyślnie `data/ledger/orca_position_lifecycle.jsonl`; ścieżka: `CLMM_POSITION_LIFECYCLE_LEDGER_PATH` lub legacy `CLMM_POSITION_OPEN_LEDGER_PATH`. Pola: mint A/B, limity open (raw+UI), `tx_fee_lamports` (`meta.fee`), oraz **`fee_payer_pre/post` + `fee_payer_net_lamports_delta`** (dla płatnika z `preBalances`/`postBalances` w tej samej transakcji). Przy **open** delta jest zwykle ujemna (fee+rent+depozyt SOL do puli); przy **close** często dodatnia (zwrot rent + SOL z płynności) minus skutek fee — suma delt po obu tx daje przybliżony **bilans SOL** z tych operacji (nogi tokenowe USDC itd. osobno).

## 2026-03-30 — tools: `solana_account_state.ps1` (SOL + SPL snapshot via JSON-RPC)

**keywords:** tools, powershell, solana, rpc, getBalance, getTokenAccountsByOwner, account-state, spl-token, token-2022, mainnet
**paths:** `tools/solana_account_state.ps1`

Read-only skrypt Windows: zbiera **lamports + SPL** dla podanego ownera (`spl-token` i **Token-2022**), bez `solana`/`spl-token` CLI. Parametry RPC: `getTokenAccountsByOwner` z filtrem `{ programId }` i **osobnym** trzecim obiektem `{ encoding: jsonParsed }` (wymóg RPC). Kolejka URL: `SOLANA_RPC_URL` → `SOLANA_RPC_FALLBACK_URLS` → domyślne publiczne fallbacki (mniej 429 na pojedynczym hoście). Wyjście: konsola lub `-Json` / `-OutJson` pod kolejne kroki automatyzacji.

## 2026-03-30 — Bot Tier3: position-fee ledger + feeGrowthInside (Whirlpool)

**keywords:** clmm-lp-execution, clmm-lp-protocols, clmm-lp-domain, PositionTruthMode, position_fee_ledger, PositionFeeCheckpoint, orca, whirlpool, tick_array, fee_growth_inside, fee_growth_outside, fee_growth_global
**paths:** `crates/execution/src/strategy/executor.rs`, `crates/execution/src/lifecycle/tracker.rs`, `crates/domain/src/position_fee_checkpoint.rs`, `crates/protocols/src/orca/tick_reader.rs`, `crates/protocols/src/orca/tick_array.rs`

Dodano rozszerzony JSONL ledger checkpointów pozycji (schema_version=2) oraz runtime capture w pętli bota: `event_type=poll` + pre/post dla collect/decrease/close (gdy `fee_mode=position_truth`). W `clmm-lp-protocols` dodano reader TickArray (PDA `tick_array`) i wyliczanie `feeGrowthInside` na podstawie `feeGrowthGlobal` i `feeGrowthOutside` dla granic ticków, zapisywane do ledger dla audytu i walidacji vs real fees.

## 2026-03-30 — Backtest: snapshot `liquidity_active` fee share + debug/tick modes

**keywords:** clmm-lp-cli, backtest_engine, run_single, fee-source snapshots, liquidity_active_raw, dynamic-liquidity-share, tick-aligned-inrange, CLMM_DEBUG_STEP_LIQ_SHARE, CLMM_IN_RANGE_TICK, orca, raydium
**paths:** `crates/cli/src/backtest_engine.rs`, `crates/cli/src/commands/snapshot_price_path.rs`, `crates/cli/src/local_swap_fees.rs`, `crates/cli/src/engine/tests.rs`

W trybie `--fee-source snapshots` backtest teraz przenosi `liquidity_active_raw` z snapshotów do `StepDataPoint` i atrybuuje pool fees per krok dynamicznie jako `position_liquidity / liquidity_active_at_step` (zamiast stałego `pool_active_liquidity` dla całego runu). Dodatkowo:
- env `CLMM_DEBUG_STEP_LIQ_SHARE` wypisuje mechanikę in-range i podział fees (pierwsze N kroków)
- env `CLMM_IN_RANGE_TICK=1` przełącza in-range z floatowych granic (`--lower/--upper`) na tickowo (`tick_current` vs wyznaczane ticki); działa gdy snapshot dostarcza `tick_current`.
---

## 2026-03-30 — CLI: `snapshot-run-curated-all --snapshots-suffix` + pętla 5m

**keywords:** clmm-lp-cli, snapshot-run-curated-all, snapshots-suffix, snapshot-jsonl-suffix, snapshot-backtest-prep, prepared-snapshot-window, powershell, snapshot-loop, orca, raydium, meteora
**paths:** `crates/cli/src/main.rs`, `crates/cli/src/commands/snapshot_backtest_prep.rs`, `crates/cli/src/commands/snapshot_price_path.rs`, `scripts/windows/run-snapshot-loop-5m.ps1`

`snapshot-run-curated-all` obsługuje teraz `--snapshots-suffix <SUFFIX>`: zapisuje snapshoty do `data/pool-snapshots/{protocol}/{pool}/snapshots_<SUFFIX>.jsonl` (zamiast `snapshots.jsonl`) oraz status do `data/snapshot_logs/snapshot-run-curated-all_<SUFFIX>.jsonl`. Dodano skrypt Windows `scripts/windows/run-snapshot-loop-5m.ps1`, który uruchamia zbieranie co 5 minut do wariantu `snapshots_5m.jsonl`.

W praktyce odpalasz oba skrypty równolegle: `scripts/windows/run-snapshot-loop.ps1` (wariant domyślny `snapshots.jsonl`, co 10 minut) oraz `scripts/windows/run-snapshot-loop-5m.ps1` (wariant `snapshots_5m.jsonl`, co 5 minut). Oba procesy zapisują do osobnych plików, więc nie nadpisują się.

Dodano też obsługę wariantu w backtestach: `backtest` / `backtest-optimize` mają flagę `--snapshot-jsonl-suffix 5m` (czytanie `snapshots_5m.jsonl`) oraz `snapshot-backtest-prep --snapshots-suffix 5m`, który zapisuje osobny cache pod `data/backtest-snapshot-cache/orca_5m/...` (manifest do `data/backtest-snapshot-cache/manifest_5m.json`).
---

## 2026-03-30 — CLI: accept datetime in `--start-date/--end-date` + end-exclusive snapshot filtering

**keywords:** clmm-lp-cli, backtest, backtest-optimize, start-date, end-date, datetime, RFC3339, snapshot_price_path, end-exclusive
**paths:** `crates/cli/src/main.rs`, `crates/cli/src/commands/snapshot_price_path.rs`, `crates/cli/src/commands/snapshot_backtest_prep.rs`

Extended snapshots-mode window parsing so `--start-date/--end-date` accept timestamps like `2026-03-24T11:00:00Z` (in addition to `YYYY-MM-DD`). Snapshot JSONL parsing and snapshot cache prep now treat `end_ts` as **exclusive** (`ts >= end_ts` filtered out) to match intended “withdraw at 10:00” semantics.

---

## 2026-03-30 — Snapshot-fee sanity-check override via env var

**keywords:** clmm-lp-cli, backtest, fee-source snapshots, snapshot fee sanity check, CLMM_SNAPSHOT_FEE_SANITY_MAX_RATIO
**paths:** `crates/cli/src/main.rs`

Guardrail „snapshot pool fees vs candle baseline” (ratio default `10x`) was causing `--fee-source snapshots` to fall back when `--price-path-source birdeye` runs without Dune volume scaling (different unit scale between Birdeye `step_volume_usd` and snapshot `fee_growth` deltas). Added env var `CLMM_SNAPSHOT_FEE_SANITY_MAX_RATIO` to override the threshold for experiments/debug runs.

---

## 2026-03-28 — Snapshot JSONL: `resolve_snapshot_jsonl_path` (nie zawsze `.repaired`)

**keywords:** clmm-lp-cli, snapshot_price_path, snapshots.jsonl.repaired, resolve_snapshot_jsonl_path, backtest, calendar window
**paths:** `crates/cli/src/commands/snapshot_price_path.rs`, `crates/cli/src/commands/snapshot_backtest_prep.rs`

Wcześniej przy istnieniu **`snapshots.jsonl.repaired`** wybierano go **zawsze** zamiast `snapshots.jsonl`. Plik naprawczy często **zostaje w tyle** względem append-only kolekcji → okna `--start-date` / `--hours` „ostatnie dni” były **puste** mimo świeżych wierszy w `snapshots.jsonl`. Teraz wybór: **`mtime` nowszy wygrywa** (remis → raw). To nie zastępuje ręcznego usunięcia przestarzałego `.repaired`, ale przy typowym flow collector + stary repair znów widać aktualne timestampy.

---

## 2026-03-28 — CLI: `snapshot-backtest-prep` + `--prepared-snapshot-window` (cache pod szybkie backtesty)

**keywords:** clmm-lp-cli, snapshot-backtest-prep, backtest-snapshot-cache, prepared_snapshot_window, Orca, snapshots.jsonl, backtest, backtest-optimize
**paths:** `crates/cli/src/commands/snapshot_backtest_prep.rs`, `crates/cli/src/commands/snapshot_price_path.rs`, `crates/cli/src/main.rs`, `tools/run_snapshot_backtest_prep_loop.ps1`

Komenda **`snapshot-backtest-prep`** czyta `data/pool-snapshots/orca/<POOL>/snapshots.jsonl` i zapisuje przycięte okna czasowe do **`data/backtest-snapshot-cache/orca/<POOL>/window_h24.jsonl`** (oraz `h48`, `h96`, `d7`, `d30` wg flag) + **`data/backtest-snapshot-cache/manifest.json`**. Domyślne pool-e: SOL/USDC + whETH/SOL (lista jak w module). **`backtest`** / **`backtest-optimize`** przy **`--price-path-source snapshots`** mogą użyć **`--prepared-snapshot-window h24`** (tylko Orca) — wtedy `build_from_orca_snapshots` czyta plik cache zamiast pełnego JSONL (nadal przecięcie z `--hours` / datami). Uruchomienie z root workspace: **`cargo run -p clmm-lp-cli --bin clmm-lp-cli -- snapshot-backtest-prep`**. Skrypt **`tools/run_snapshot_backtest_prep_loop.ps1`**: opcjonalnie **`snapshot-run-curated-all`** + **`snapshot-backtest-prep`** w pętli / Task Scheduler.

---

## 2026-03-28 — `run_single` / snapshot path: human `price_ab` must map to raw before sqrt valuation

**keywords:** clmm-lp-cli, backtest_engine, run_single, price_ab_human_to_raw, price_to_sqrt_q64, token decimals, SOL/USDC, final_value, PnL, snapshot_price_path
**paths:** `crates/cli/src/backtest_engine.rs`, `crates/cli/src/engine/tests.rs`

`estimate_position_liquidity` już używa `price_ab_human_to_raw` dla widełek i ceny wejścia; **`run_single`** liczył `sqrt` dla lower/upper/spot **bez** tego kroku. Przy **różnych `dec_a` / `dec_b`** (np. 9/6) sqrt i kwoty tokenów były w złej przestrzeni względem **L** → absurdalne **final_value / PnL** na ścieżce snapshotów. Dodano **`sqrt_q64_from_price_ab_human`** i użyto go przy wycenie krok po kroku oraz na końcu runu. Test regresji: **`run_single_sol_usdc_decimals_position_value_sane_at_flat_price`**. Test **`birdeye_volume_fees_match_equivalent_snapshot_fee_index`**: przy identycznych krokach cenowych i **`pool_fees_usd` = `step_volume_usd * fee_rate`** wynik `run_single` jest taki sam co przy samym indeksie snapshotów (`step_volume_usd = 0`).

---

## 2026-03-28 — `backtest-optimize`: wire snapshot `fee_growth` index into `run_grid` / `total_fees`

**keywords:** clmm-lp-cli, backtest-optimize, snapshot_fee_index_full, run_grid, run_single, total_fees, snapshot_price_path, fee_growth
**paths:** `crates/cli/src/backtest_engine.rs`, `crates/cli/src/main.rs`, `crates/cli/src/engine/tests.rs`

Ścieżka `--price-path-source snapshots` budowała `per_step_fees_usd` (log „N step buckets”), ale symulacja brała opłaty wyłącznie z `step_volume_usd * fee_rate` albo z Dune swaps — snapshotowe kroki mają `step_volume_usd = 0`, więc **`total_fees` w `TrackerSummary` było zerem**. Dodano opcjonalny map `snapshot_pool_fees_usd` do `run_single` / `run_grid`; `backtest-optimize` przekazuje go gdy `prefer_snapshot_fee_idx` (to samo co dotychczasowy `Auto` + niepusty indeks lub `--fee-source snapshots`). Okna rolling: remap indeksów globalnych na lokalne slice. Test: `snapshot_pool_fee_index_accrues_lp_share_when_in_range`.

---

## 2026-03-27 — CLI: `orca-pool-read` (mainnet RPC, Whirlpool tick / price B/A / liquidity)

**keywords:** clmm-lp-cli, orca-pool-read, mainnet, RpcProvider, WhirlpoolReader, SOLANA_RPC_URL, read-only
**paths:** `crates/cli/src/main.rs`

Nowa subkomenda **`orca-pool-read --pool-address <WHIRLPOOL>`**: tylko odczyt przez `RpcProvider::mainnet()` + `WhirlpoolReader::get_pool_state` — wypisuje m.in. `tick_current`, `sqrt_price_x64`, `price_token_b_per_token_a` (surowy stosunek B/A, nie USD), `liquidity`, minty/vaulty oraz skrót opłat jak przy `orca-pool-fee`.

---

## 2026-03-27 — `swaps-enrich-curated-all`: fail-fast when `SOLANA_RPC_URL` is devnet

**keywords:** clmm-lp-cli, swap_sync, swaps-enrich-curated-all, SOLANA_RPC_URL, devnet, mainnet-beta, STARTUP.md
**paths:** `crates/cli/src/swap_sync.rs`

Curated pools in `STARTUP.md` are mainnet; `getTransaction` against devnet for mainnet signatures fails endlessly. **`swaps-enrich-curated-all`** now errors immediately if the resolved primary RPC URL looks like devnet, with a message to switch or unset `SOLANA_RPC_URL`.

---

## 2026-03-27 — Mainnet prep: `CLMM_EXPECTED_CLUSTER`, RPC cluster guard; CLI backtest-optimize sync; docs

**keywords:** clmm-lp-protocols, clmm-lp-cli, rpc, CLMM_EXPECTED_CLUSTER, mainnet, dry-run, backtest-optimize, run_grid, StratConfig, DuneClient, from_env_swaps_only, MAINNET_OPERATIONAL_CHECKLIST, ORCA_RUNBOOK
**paths:** `crates/protocols/src/rpc/cluster.rs`, `crates/protocols/src/rpc/provider.rs`, `crates/protocols/src/rpc/config.rs`, `crates/data/src/providers/dune.rs`, `crates/cli/src/main.rs`, `crates/cli/src/backtest_engine.rs`, `crates/cli/src/output/optimize_result_json.rs`, `doc/MAINNET_OPERATIONAL_CHECKLIST.md`, `doc/ORCA_RUNBOOK.md`, `doc/README.md`

- **`RpcProvider::new`** runs optional URL-vs-intent validation when **`CLMM_EXPECTED_CLUSTER`** is set (`mainnet-beta` \| `devnet` \| `testnet` \| `localnet`); custom RPC hostnames without keywords are skipped.
- **`clmm-lp-cli`:** zsynchronizowano `backtest-optimize` / `backtest` z aktualnym `run_grid` / `StratConfig` (tylko Static, Threshold, Periodic); `DuneClient::from_env_swaps_only`; usunięto przestarzałe `GridRunParams` / per-step `pool_liquidity_active` z `StepDataPoint`.
- **Docs:** [`doc/MAINNET_OPERATIONAL_CHECKLIST.md`](MAINNET_OPERATIONAL_CHECKLIST.md) + wpis w [`doc/ORCA_RUNBOOK.md`](ORCA_RUNBOOK.md) i indeks [`doc/README.md`](README.md).

---

## 2026-03-27 — CLI tests: no committed snapshot JSONL; inline bytes + temp JSONL

**keywords:** clmm-lp-cli, snapshot_readiness, decode_fixture_tests, snapshot_readiness_regression_test, pool-snapshots, ci, raydium, meteora
**paths:** `crates/cli/tests/decode_fixture_tests.rs`, `crates/cli/tests/snapshot_readiness_regression_test.rs`

Workspace `data/` stays gitignored — we **do not** commit `pool-snapshots/*.jsonl`. Parser regression tests embed one `data_b64` account sample per protocol in Rust source. `snapshot_readiness` regression writes **minimal synthetic JSONL** under a temp `data/pool-snapshots/...` tree (tier-2 fields only) and runs the binary with that cwd.

---

## 2026-03-27 — Orca: full-range (Splash) open, `fetch_positions_for_owner`, Splash pool lookup

**keywords:** clmm-lp-protocols, clmm-lp-execution, clmm-lp-api, clmm-lp-cli, orca, whirlpools, full_range, splash, open_full_range_position_instructions, fetch_positions_for_owner, fetch_splash_pool, BuildUnsignedTxRequest, OpenPositionRequest, ENGINEERING_NOTES
**paths:** `crates/protocols/src/orca/executor.rs`, `crates/protocols/src/orca/pool_reader.rs`, `crates/execution/src/strategy/rebalance.rs`, `crates/api/src/handlers/tx.rs`, `crates/api/src/models.rs`, `crates/api/src/services/orca_tx_service.rs`, `crates/cli/src/commands/orca_position.rs`, `crates/cli/src/main.rs`

- **Full-range open:** `WhirlpoolExecutor::open_full_range_position` + `OpenFullRangeParams`; `full_range` flag on `POST /positions` (`OpenPositionRequest`), `BuildUnsignedTxRequest` (`full_range: true` skips tick fields for `/tx/open/build`), and `OrcaTxService::OpenPositionTxRequest`. `StrategyExecutor::execute_open_position` takes `full_range` and records effective tick range in fee checkpoints.
- **Discovery CLI:** `orca-positions-list` (`fetch_positions_for_owner`) and `orca-splash-pool` (`fetch_splash_pool`). **CLI open:** `--full-range` on `orca-position-open` / `orca-position-open-and-close`.
- **Helper:** `full_range_tick_indexes` in `pool_reader` (uses `orca_whirlpools_core`).

---

## 2026-03-27 — CLI + PS: bot JSONL ledgers (`il` + position-fee) and default `data/bot-runs/devnet/`

**keywords:** clmm-lp-cli, orca-bot-run, orca-bot-open-and-run, il_ledger_path, position_fee_ledger_path, powershell, bot_run_devnet, bot_session_devnet, jsonl, backtest
**paths:** `crates/cli/src/commands/orca_bot.rs`, `crates/cli/src/main.rs`, `tools/bot_run_devnet.ps1`, `tools/bot_session_devnet.ps1`, `doc/ORCA_RUNBOOK.md`

Dodano flagi `--il-ledger-path` i `--position-fee-ledger-path` do `orca-bot-run` / `orca-bot-open-and-run` (podpięte pod `StrategyExecutor::set_il_ledger_path` / `set_position_fee_ledger_path`; katalogi nadrzędne tworzone przed startem). Skrypty `bot_run_devnet.ps1` i `bot_session_devnet.ps1` domyślnie zakładają run w `data/bot-runs/devnet/<timestamp>/` z plikami `il_ledger.jsonl` i `position_fee_ledger.jsonl`), z wyłączeniem przez `-SkipLedger`.

## 2026-03-27 — API: add unsigned tx `increase` + one-command devnet smokes

**keywords:** clmm-lp-api, tx-build, increase-liquidity, orca, whirlpools, devnet, e2e, powershell
**paths:** `crates/api/src/handlers/tx.rs`, `crates/api/src/routes.rs`, `crates/api/src/openapi.rs`, `crates/api/src/handlers/devnet_e2e_tests.rs`, `tools/run_devnet_smokes.ps1`

Dodano brakujący endpoint `POST /tx/increase/build` (unsigned tx flow) oparty o `orca_whirlpools::increase_liquidity_instructions` + smoke test `devnet_unsigned_increase_liquidity_smoke`. Dorzucono też skrypt `tools/run_devnet_smokes.ps1`, który pozwala odpalić cały pakiet `devnet_` ignored testów jedną komendą (z ustawieniem env).

---

## 2026-03-27 — Devnet testability: safer RPC defaults + bot action smoke

**keywords:** clmm-lp-protocols, rpc, devnet, fallback, ankr, unauthorized, clmm-lp-api, bot, soak, e2e
**paths:** `crates/protocols/src/rpc/config.rs`, `crates/api/src/handlers/devnet_e2e_tests.rs`, `tools/run_devnet_smokes.ps1`, `crates/execution/src/monitor/position_monitor.rs`

Zmieniono domyślne fallbacki dla devnet tak, aby **nie dodawać automatycznie** endpointów wymagających API key (np. Ankr) — fallbacki są teraz wyłącznie z env (`SOLANA_RPC_FALLBACK_URLS`). Dodano `PositionMonitor::refresh_position` oraz nowy smoke `devnet_bot_actions_smoke` (open → collect → close) jako szybki test akcji bota bez długiej pętli.

---

## 2026-03-27 — CLI: Orca Whirlpool swap (`orca-swap`) for devnet funding automation

**keywords:** cli, orca, swap, devnet, sol-usdc, automation, whirlpools, sdk
**paths:** `crates/cli/src/commands/orca_swap.rs`, `crates/cli/src/main.rs`, `doc/ORCA_RUNBOOK.md`

Dodano komendę `orca-swap`, która buduje i wysyła swap na Orca Whirlpool przez `orca_whirlpools::swap_instructions` (ExactIn/ExactOut, slippage bps). Pozwala to automatycznie uzyskać (dev)USDC z SOL na devnecie bez ręcznego korzystania z UI.

---

## 2026-03-27 — PowerShell automation: wallet/position rebalance to ~50/50 on devnet

**keywords:** powershell, devnet, automation, rebalance, 50-50, orca-swap, close-position, open-position
**paths:** `tools/devnet_rebalance_wallet_half.ps1`, `doc/ORCA_RUNBOOK.md`

Rozbudowano skrypt `devnet_rebalance_wallet_half.ps1`:
- obsługuje obie strony (SOL->devUSDC oraz devUSDC->SOL, zależnie od overweight),
- opcjonalny tryb pozycji: `close -> rebalance -> open` dla automatyzacji „rebalance po połowie” bez ręcznego przepisywania kroków.

---

## 2026-03-27 — Safer open defaults in CLI (`amount_a/b`) to avoid SDK overflow path

**keywords:** cli, orca, open-position, amount-cap, devnet, sdk, overflow
**paths:** `crates/cli/src/commands/orca_position.rs`, `crates/cli/src/main.rs`

W komendach open (`orca-position-open`, `orca-position-open-and-close`, `orca-bot-open-and-run`) dodano jawne limity `amount_a/amount_b` i bezpieczne domyślne wartości (1000/1000) zamiast `u64::MAX`, aby uniknąć ścieżki overflow po stronie SDK przy wyznaczaniu token amountów dla open.

---

## 2026-03-27 — CLI devnet convenience: `orca-position-open-and-close`

**keywords:** cli, devnet, orca, open-and-close, sol-usdc, automation, smoke-flow
**paths:** `crates/cli/src/commands/orca_position.rs`, `crates/cli/src/main.rs`, `doc/ORCA_RUNBOOK.md`

Dodano komendę `orca-position-open-and-close`, która otwiera pozycję, czeka `--sleep-secs`, a następnie zamyka pozycję (pełne `close`). Ułatwia to szybkie devnet smoke testy “open -> close” bez ręcznego kopiowania `position_address`.

---

## 2026-03-27 — CLI: `orca-position-close` and `orca-position-collect-fees`

**keywords:** cli, orca, devnet, close-position, collect-fees, lifecycle, execution
**paths:** `crates/cli/src/commands/orca_position.rs`, `crates/cli/src/main.rs`, `doc/ORCA_RUNBOOK.md`

Dodano brakujące komendy operacyjne CLI do domykania sesji na devnecie: `orca-position-collect-fees` oraz `orca-position-close`. Obie komendy biorą `--position` i (poza `--dry-run`) używają signing wallet do wykonania ścieżek `collect_fees` i pełnego `close`.

---

## 2026-03-27 — New CLI flow `orca-bot-open-and-run` for devnet operations

**keywords:** cli, orca, bot, devnet, open-and-run, position-address, automation, runbook
**paths:** `crates/cli/src/commands/orca_bot.rs`, `crates/cli/src/main.rs`, `doc/ORCA_RUNBOOK.md`

Dodano komendę `orca-bot-open-and-run`, która wykonuje on-chain `open_position` (SDK path), pobiera realny `created_position` i natychmiast uruchamia na nim `orca-bot-run`. To upraszcza devnetowy flow operatorski (open -> monitor/strategy) i eliminuje ręczne przenoszenie adresu pozycji między krokami.

---

## 2026-03-27 — Orca hardening handoff: real `created_position` + unsigned lifecycle smoke

**keywords:** orca, sdk, created-position, position-address, unsigned-tx, lifecycle, open-decrease-collect-close, devnet, powershell, runbook
**paths:** `crates/protocols/src/orca/executor.rs`, `crates/execution/src/strategy/rebalance.rs`, `crates/cli/src/commands/orca_position.rs`, `crates/api/src/services/position_service.rs`, `crates/api/src/handlers/devnet_e2e_tests.rs`, `tools/bot_run_devnet.ps1`, `tools/bot_session_devnet.ps1`, `doc/DEVNET_WALLET_BOT_LAUNCH_RUNBOOK_V1.md`, `doc/ORCA_RUNBOOK.md`

Dopięto handoff realnego adresu pozycji z Orca SDK do warstw konsumenckich: `WhirlpoolExecutor::open_position` zwraca teraz `created_position` (PDA liczone z faktycznego `position_mint`), a execution/CLI/API przestały polegać na zgadywaniu pozycji po `(pool,ticks)` dla ścieżek open. Dodano także ignored smoke test dla pełnego unsigned lifecycle (`open -> read/decode -> decrease-all -> collect -> close`) oraz wsparcie w skryptach botowych dla wejścia `-OpenBuildResponseJson` (czytanie `position_address` z odpowiedzi `/tx/open/build`), z aktualizacją runbooków operacyjnych.

---

## 2026-03-27 — Devnet e2e open/read coverage for Orca proxy pairs (Nebula pools)

**keywords:** devnet, e2e, orca, proxy-pairs, open-position, read-back, position-address, nebula, smoke-tests
**paths:** `crates/api/src/handlers/devnet_e2e_tests.rs`

Dodano ignored smoke test `devnet_open_and_read_position_proxy_pairs_smoke`, który przechodzi po trzech devnetowych parach proxy (SOL/devUSDC, devSAMO/devUSDC, devTMAC/devUSDC) i dla każdej wykonuje pełny flow: `tx/open/build` -> podpis walletem -> `tx/submit-signed` -> odczyt konta pozycji po `position_address` zwróconym przez API -> deserializacja `WhirlpoolPosition`. Adresy puli pochodzą z tabeli devToken Nebula (Orca Whirlpools, devnet).

---

## 2026-03-27 — `/tx/open/build` now returns `position_mint` and `position_address` + open/read smoke

**keywords:** api, tx-open-build, orca, whirlpools, position-mint, position-address, automation, devnet, smoke-test
**paths:** `crates/api/src/models.rs`, `crates/api/src/handlers/tx.rs`, `crates/api/src/handlers/devnet_e2e_tests.rs`

Rozszerzono kontrakt `BuildUnsignedTxResponse` o pola `position_mint` i `position_address` dla ścieżki `POST /tx/open/build`, aby automatyzacja nie musiała zgadywać adresu pozycji po open. Dla `open` adres pozycji jest liczony z rzeczywistego `position_mint` zwracanego przez Orca SDK (`position = PDA("position", position_mint)`), co eliminuje błędne założenie deterministycznego wyliczania tylko z `(pool,tick_lower,tick_upper)`. Dodano też devnet smoke test `devnet_open_and_read_position_smoke` pokrywający sekwencję open -> submit -> odczyt i deserializację konta pozycji.

---

## 2026-03-27 — Orca devnet bot: WhirlpoolPosition deserialization + tx policy fixes

**keywords:** bot, devnet, orca, whirlpools, position-reader, borsh, policy-gate, allowlist, aToken, token-2022, executor, signer
paths: `crates/protocols/src/orca/position_reader.rs`, `crates/api/src/handlers/tx.rs`, `crates/protocols/src/orca/executor.rs`

Naprawiono wczytywanie on-chain pozycji dla `orca-bot-run` (dodano brakujące `reward_infos` do modelu `WhirlpoolPosition`, żeby `BorshDeserialize` nie kończyło się błędem `Not all bytes read`). Dodatkowo skorygowano policy-gate allowlist w `/tx/submit-signed` (brakujący program-id dla wariantu ATA z `orca_whirlpools` SDK) oraz usunięto błędne wymaganie podpisu dla `position_mint` w `WhirlpoolExecutor::open_position` (fix panic `NotEnoughSigners`).

---

## 2026-03-27 — Session timeout control for devnet bot wrapper

**keywords:** bot, devnet, powershell, timeout, max-runtime, session-wrapper, operations
**paths:** `tools/bot_session_devnet.ps1`, `doc/DEVNET_WALLET_BOT_LAUNCH_RUNBOOK_V1.md`

`bot_session_devnet.ps1` dostal parametr `-MaxRuntimeMinutes`, ktory uruchamia `bot_run_devnet` w osobnym procesie i automatycznie zatrzymuje sesje po zadanym czasie. Skrypt nadal zapisuje raport post-run i oznacza status `run_status=timeout`, co pozwala bezpiecznie uruchamiac ograniczone czasowo sesje pod scheduler/ops.

---

## 2026-03-27 — Devnet bot ops scripts: preflight, run wrapper, post-run report

**keywords:** bot, devnet, runbook, powershell, preflight, orca-bot-run, operations, reports
**paths:** `tools/bot_preflight.ps1`, `tools/bot_run_devnet.ps1`, `tools/bot_postrun_report.ps1`, `doc/DEVNET_WALLET_BOT_LAUNCH_RUNBOOK_V1.md`

Dodano trzy skrypty operacyjne pod powtarzalne uruchamianie bota na devnecie: `bot_preflight.ps1` (fail-fast check env/RPC/keypair), `bot_run_devnet.ps1` (wrapper na `orca-bot-run` z trybem dry-run/execute i domyslnym preflight) oraz `bot_postrun_report.ps1` (raport sesji JSON do `data/reports/`). Runbook v1 uzupelniono o gotowe komendy dla tych skryptow.

---

## 2026-03-27 — One-command devnet bot session wrapper

**keywords:** bot, devnet, powershell, session-wrapper, automation, preflight, report
**paths:** `tools/bot_session_devnet.ps1`, `doc/DEVNET_WALLET_BOT_LAUNCH_RUNBOOK_V1.md`

Dodano nadrzedny skrypt `bot_session_devnet.ps1`, ktory spina caly przebieg sesji w jednej komendzie: preflight (opcjonalnie), uruchomienie `orca-bot-run`, a nastepnie zapis raportu post-run. Przy bledzie uruchomienia skrypt nadal zapisuje raport z `run_status=failed`, co poprawia audyt i niezawodnosc operacyjna pod scheduler.

---

## 2026-03-27 — Tier3 usability: per-position readiness + MVP position-truth report CLI

**keywords:** tier3, position-truth, snapshot-readiness, position-address, position-truth-report, jsonl, clmm-lp-cli
**paths:** `crates/cli/src/bin/snapshot_readiness.rs`, `crates/cli/src/bin/position_truth_report.rs`, `crates/cli/tests/snapshot_readiness_regression_test.rs`

W trybie `--fee-mode position-truth` Tier3 readiness jest teraz liczone **per pozycja** (filtruje checkpointy po `pool+position`). Jeśli `--position-address` nie jest podany, narzędzie auto-wykrywa pozycje z ledgeru dla danego poola: gdy jest dokładnie jedna, używa jej automatycznie; gdy jest wiele, wypisuje listę i wymaga wyboru. Dodano nowy bin `position-truth-report` (MVP), który czyta `data/position-fee-checkpoints.jsonl` i wypisuje podsumowanie oraz tail checkpointów dla wskazanego `(pool, position)`. Dodano testy na fixture JSONL.

---

## 2026-03-27 — Tier3 wiring: default checkpoint ledger path enabled in CLI bot and API strategy start

**keywords:** tier3, position-truth, checkpoint-ledger, orca-bot, api-strategy, jsonl, clmm-lp-cli, clmm-lp-api
**paths:** `crates/cli/src/commands/orca_bot.rs`, `crates/api/src/handlers/strategies.rs`, `crates/api/src/services/strategy_service.rs`

Domyślnie włączono zapisywanie checkpointów fee pozycji do `data/position-fee-checkpoints.jsonl` podczas uruchamiania bota CLI (`orca_bot`) oraz startu strategii w API/StrategyService. Dzięki temu Tier3 `snapshot-readiness --fee-mode position-truth` ma z czego czytać bez dodatkowej konfiguracji ścieżki.

---

## 2026-03-27 — Tier3 (PR3 WIP): snapshot-readiness reads position-fee checkpoint ledger

**keywords:** tier3, position-truth, snapshot-readiness, checkpoint-ledger, jsonl, clmm-lp-cli
**paths:** `crates/cli/src/bin/snapshot_readiness.rs`, `crates/cli/tests/snapshot_readiness_regression_test.rs`

`snapshot-readiness` w trybie `--fee-mode position-truth` potrafi teraz czytać lokalny JSONL z checkpointami (`data/position-fee-checkpoints.jsonl` lub `--position-fee-ledger-path`) i na tej podstawie wylicza Tier3 READY/NOT READY wraz z listą braków (min. 2 checkpointy dla poola + `open_position` + postęp typu `collect/close/rebalance`). Dodano test integracyjny z tempowym ledgerem checkpointów.

---

## 2026-03-27 — Tier3 prep (PR2): position-fee checkpoint ledger wired into lifecycle/strategy flow

**keywords:** tier3, position-truth, lifecycle, strategy-executor, position-fee-checkpoint, jsonl, clmm-lp-execution
**paths:** `crates/execution/src/lifecycle/tracker.rs`, `crates/execution/src/strategy/executor.rs`

Dodano dedykowany ledger JSONL dla checkpointów fee pozycji (`set_position_fee_ledger_path` + `record_fee_checkpoint`) w `LifecycleTracker`. `StrategyExecutor` emituje teraz checkpointy dla kluczowych operacji (`open_position`, `decrease_liquidity`, `collect_fees`, `close_position`) oraz podczas udanego `rebalance` (checkpoint `rebalance_out` dla starej pozycji i `rebalance_in` dla nowej). Dzięki temu zaczyna powstawać timeline danych pod tryb `position_truth` bez zmiany domyślnego flow `heuristic`.

---

## 2026-03-27 — Tier3 prep (PR1): fee mode switch + domain checkpoint model skeleton

**keywords:** tier3, position-truth, heuristic, fee-mode, checkpoint, clmm-lp-domain, clmm-lp-execution, snapshot-readiness
**paths:** `crates/domain/src/position_fee_checkpoint.rs`, `crates/domain/src/lib.rs`, `crates/domain/src/prelude.rs`, `crates/execution/src/strategy/executor.rs`, `crates/cli/src/bin/snapshot_readiness.rs`

Dodano szkielet pod drugi tryb fee accounting: `PositionTruthMode` (`heuristic` vs `position_truth`) oraz minimalny model `PositionFeeCheckpoint` w crate `domain`. `ExecutorConfig` w `execution` dostał pole `fee_mode` (domyślnie `Heuristic`, więc brak regresji obecnego flow). CLI `snapshot-readiness` przyjmuje teraz `--fee-mode` i raportuje aktywny tryb; ścieżka `position_truth` jest jawnie oznaczona jako jeszcze niepodpięta do evaluatora Tier3.

---

## 2026-03-27 — Meteora snapshots: always emit vault_amount fields for Tier1 readiness

**keywords:** meteora, snapshot-collector, snapshot-readiness, tier1, vault-amount, token-account, clmm-lp-cli
**paths:** `crates/cli/src/snapshots/collector.rs`

W collectorze Meteora dopięto stabilne emitowanie `vault_amount_a` i `vault_amount_b` w każdym nowym wierszu snapshotu: gdy RPC decode reserve-account się powiedzie, zapisujemy realne wartości; gdy odczyt jest niedostępny, zapisujemy fallback `0` oraz `vault_amount_source="missing_fallback_zero"`. Dzięki temu `snapshot-readiness` ma komplet pól wymaganych przez Tier1 (`LP-share`) i po dosnapshotowaniu co najmniej 2 nowych wierszy zaczyna raportować `Tier1 READY`.

---

## 2026-03-26 — tx unsigned build: Orca SDK open_position instruction builder

**keywords:** tx-build, unsigned-tx, orca_whirlpools, open_position_instructions_with_tick_bounds, partial-sign, clmm-lp-api
**paths:** `crates/api/src/handlers/tx.rs`, `crates/api/src/handlers/devnet_e2e_tests.rs`

W `POST /tx/*/build` unsigned flow wdrożono realne instrukcje z `orca_whirlpools` SDK (dla `open` przez `open_position_instructions_with_tick_bounds`, a dla `decrease/collect/close` wyprowadzamy `position_mint` z on-chain `WhirlpoolPosition` i używamy odpowiednich `*_instructions`). Dodatkowo server pre-signuje wymagane `additional_signers` (partial signatures), a testy Phantom-emulacji ustawiają wyłącznie signature wallet w odpowiednim slocie.

---

## 2026-03-26 — Strategy-driven bot: wallet + monitor seeding on start

**keywords:** bot, strategy-executor, auto_execute, wallet, KEYPAIR_PATH, position-monitor, devnet-e2e, clmm-lp-api
**paths:** `crates/api/src/handlers/strategies.rs`, `crates/api/src/handlers/devnet_e2e_tests.rs`

`POST /strategies/{id}/start` może teraz zasilić `PositionMonitor` listą pozycji z `parameters.position_addresses`. Dodatkowo, gdy `auto_execute=true` i `dry_run=false`, API wymusza i ładuje signing wallet z `KEYPAIR_PATH`/`SOLANA_KEYPAIR_PATH` oraz podpina go do `StrategyExecutor`, dzięki czemu strategie realnie sterują rebalance na devnecie (patrz `devnet_strategy_driven_rebalance_smoke`).

---

## 2026-03-27 — Quick data verifier (snapshot + decode + health, GO/NO-GO)

**keywords:** operations, quick-verify, snapshot-readiness, decode-audit, data-health-check, go-no-go, powershell
**paths:** `tools/quick_verify_data.ps1`, `doc/ORCA_RUNBOOK.md`

Dodano jedno-komendowy verifier operacyjny (`tools/quick_verify_data.ps1`) łączący `snapshot-readiness`, `data-health-check` i `swaps-decode-audit` w raport GO/NO-GO (`data/reports/quick_verify_*.json`) z kodem wyjścia 2 przy FAIL (pod scheduler/CI). W runbooku dodano sekcję z szybkim uruchomieniem.

---

## 2026-03-26 — Devnet production-readiness checklist (3 phases)

**keywords:** devnet, bot, production-readiness, checklist, go-no-go, operations, tx-safety
**paths:** `doc/DEVNET_BOT_PRODUCTION_READINESS.md`, `doc/README.md`

Dodano dedykowany dokument z checklista przejscia z devnet MVP do trybu production-like: faza 1 (must-have, blokery), faza 2 (stabilnosc operacyjna), faza 3 (hardening/rollout), wraz z Definition of Ready i kolejnoscia wdrozenia.

---

## 2026-03-26 — tx unsigned build: real Whirlpool instructions (not empty shell)

**keywords:** tx-build, unsigned-tx, phantom-flow, whirlpool-instruction, clmm-lp-api
**paths:** `crates/api/src/handlers/tx.rs`, `crates/api/src/handlers/devnet_e2e_tests.rs`

W `POST /tx/*/build` unsigned flow przestał budować pusty shell tx i zamiast tego generuje transaction z instrukcjami programu Whirlpool (open/decrease/collect/close), tak aby policy-gate i client-signing działały na realnym program-id/strukturze. Nadal jest to MVP względem pełnych list wymaganych kont (tick arrays / vaults) i docelowo zostanie rozszerzone o produkcyjną poprawność kont.

---

## 2026-03-26 — BuildUnsignedTxRequest: tick bounds required for `open` unsigned build

**keywords:** tx-build, unsigned-tx, open, whirlpool, tick-lower, tick-upper, api-validation, clmm-lp-api
**paths:** `crates/api/src/models.rs`, `crates/api/src/handlers/tx.rs`, `crates/api/src/handlers/devnet_e2e_tests.rs`

Dodano do `BuildUnsignedTxRequest` pola `tick_lower`/`tick_upper` oraz zaostrzono walidacje `POST /tx/open/build`: teraz `open` wymaga tych pól i encoduje je w danych instrukcji Whirlpool `open_position` zamiast `0/0`.

---

## 2026-03-26 — tx build/submit API: fail-safe request validation

**keywords:** tx-build, unsigned-tx, submit-signed, api-validation, clmm-lp-api
**paths:** `crates/api/src/handlers/tx.rs`, `crates/api/src/handlers/tx_tests.rs`, `crates/api/src/handlers/devnet_e2e_tests.rs`

Dodano twarde walidacje w `POST /tx/*/build` (wymagane pola dla open/decrease/collect/close + sanity check slippage), aby uniknac budowania niekompletnych/ryzykownych transakcji w trybie unsigned flow. Zaktualizowano devnet E2E testy unsigned flow pod nowe wymagania requestu.

---

## 2026-03-26 — Devnet E2E hardening: fail-fast keypair + negative submit tests

**keywords:** devnet, e2e, hardening, keypair, fail-fast, unsigned-tx, api-validation, clmm-lp-api
**paths:** `crates/api/src/handlers/devnet_e2e_tests.rs`

Usunięto „ciche” przechodzenie testów bez portfela: testy lifecycle i unsigned flow wymagają teraz jawnie `KEYPAIR_PATH`/`SOLANA_KEYPAIR_PATH` (fail-fast). Dodano negatywne testy submit (`unsigned tx` oraz `invalid base64`) żeby walidować granice API i policy flow na devnecie.

---

## 2026-03-26 — Devnet bot E2E pack: lifecycle endpoint + unsigned tx API + policy gate

**keywords:** devnet, e2e, bot-simulation, positions-decrease, unsigned-tx, phantom-flow, submit-signed, policy-gate, clmm-lp-api
**paths:** `crates/api/src/handlers/positions.rs`, `crates/api/src/handlers/tx.rs`, `crates/api/src/handlers/devnet_e2e_tests.rs`, `crates/api/src/routes.rs`

Dodano endpoint `POST /positions/{address}/decrease` oraz nowy zestaw endpointów unsigned tx (`/tx/*/build`, `/tx/submit-signed`) z policy gate (allowlist programów + preflight simulate). Rozszerzono pakiet `#[ignore]` o testy devnet lifecycle keypair i flow build->sign->submit (emulator Phantom przez keypair).

---

## 2026-03-26 — Async communication layer v2 scaffold (`EventBus`, contract, broker mode, metrics)

**keywords:** async-communication, event-bus, inprocess, broker, kafka, nats, redis, event-contract, correlation-id, clmm-lp-api
**paths:** `crates/api/src/events.rs`, `crates/api/src/state.rs`, `crates/api/src/websocket.rs`, `crates/api/src/main.rs`, `doc/ASYNC_COMMUNICATION_LAYER.md`

Dodano podstawową warstwę komunikacji eventowej: wersjonowany `EventEnvelope`, `EventBus` trait, `InProcessEventBus`, scaffold `BrokerEventBus` (z `EVENT_BUS_MODE` i feature `broker-event-bus`), retry publish + DLQ oraz metryki busa podpinane do `/metrics`. WebSockety subskrybują teraz eventy (`position.updated`, `alert.raised`) z busa.

---

## 2026-03-26 — API coverage suite: wszystkie endpointy z `routes` (REST + WS) mają testy

**keywords:** api, test-coverage, axum-router, websocket, routes, clmm-lp-api, endpoint-tests
**paths:** `crates/api/src/handlers/endpoint_coverage_tests.rs`, `crates/api/src/handlers/mod.rs`

Dodano router-level test suite, która uderza we wszystkie endpointy z `create_router` (w tym `/ws/positions` i `/ws/alerts`) i weryfikuje reachability/statusy na poziomie HTTP/upgrade. Testy są stabilizowane przez mocki dla `/orca/*` i przez asercje akceptujące warianty statusów zależne od live RPC.

---

## 2026-03-26 — Devnet smoke pack rozszerzony: `/orca/pools`, `/orca/tokens`, `/orca/protocol`

**keywords:** devnet, smoke, orca, live-api, ignored-tests, clmm-lp-api
**paths:** `crates/api/src/handlers/devnet_e2e_tests.rs`

Rozszerzono ręczny pakiet smoke (`#[ignore]`) o testy live dla proxy Orca REST, tak aby jednym zestawem móc szybko sprawdzić ścieżkę API→Orca oraz API→RPC devnet po zmianach.

---

## 2026-03-26 — Orca REST proxy: `/orca/pools/*` + `/orca/lock/*` (client + API + tests)

**keywords:** orca, orca-rest, clmm-lp-data, clmm-lp-api, axum, openapi, pools-search, lock, httpmock
**paths:** `crates/data/src/providers/orca_rest.rs`, `crates/api/src/handlers/orca.rs`, `crates/api/src/routes.rs`, `crates/api/src/openapi.rs`

Rozszerzono `OrcaRestClient` o `GET /pools/search`, `GET /pools/{address}` i `GET /lock/{address}` oraz wystawiono je w naszym API jako proxy pod `/orca/...` (z OpenAPI i testami `httpmock`, bez wywołań sieci).

---

## 2026-03-26 — Phantom auth foundations: challenge/verify (`signMessage`) + nonce store

**keywords:** phantom, auth, signMessage, ed25519, jwt, clmm-lp-api, axum, replay-protection
**paths:** `crates/api/src/handlers/phantom_auth.rs`, `crates/api/src/state.rs`, `crates/api/src/routes.rs`, `crates/api/src/models.rs`

Dodano minimalne, bezpieczne fundamenty pod komunikację Phantom ↔ bot: endpointy `POST /auth/phantom/challenge` i `POST /auth/phantom/verify` (challenge–response), in-memory nonce store z TTL oraz odrzucanie replay (nonce jednokrotnego użytku). To umożliwia model “bot układa tx, Phantom podpisuje”.

---

## 2026-03-26 — Orca REST proxy domknięty o tokeny/protocol + devnet API smoke test

**keywords:** orca, tokens, protocol, api-proxy, clmm-lp-data, clmm-lp-api, devnet, e2e-smoke, httpmock
**paths:** `crates/data/src/providers/orca_rest.rs`, `crates/api/src/handlers/orca.rs`, `crates/api/src/handlers/devnet_e2e_tests.rs`, `crates/api/src/routes.rs`

Dodano brakujące endpointy Orca Public API (`/tokens`, `/tokens/search`, `/tokens/{mint}`, `/protocol`) w kliencie i proxy `/orca/*` wraz z testami `httpmock`. Dodatkowo dodano ręczny test smoke `#[ignore]` pod devnet (`devnet_pool_state_smoke`) do szybkiej walidacji ścieżki API→RPC.

---

## 2026-03-26 — CLI: local-first `studio-stream-plan` (AI stream agent MVP)

**keywords:** clmm-lp-cli, studio-stream-plan, ai-narrator, stream, obs, youtube, local-first, jsonl
**paths:** `crates/cli/src/main.rs`, `crates/cli/src/commands/studio.rs`, `doc/AI_STREAM_AGENT.md`

Dodano minimalną komendę CLI `studio-stream-plan`, która czyta lokalny JSONL z “itemami do narracji” i generuje JSONL segmentów z szablonem narracji (PL/EN, `style`, `pause_secs`). To jest warstwa przygotowująca artefakty do późniejszego TTS/OBS bez wiązania projektu z konkretnym dostawcą i bez zależności od płatnych feedów.

---

## 2026-03-26 — Rebranding: “Bociarz LP Strategy Lab” (public-facing docs/UI)

**keywords:** rebrand, branding, README, openapi, cli-about, web-title, attribution, MIT
**paths:** `README.md`, `STARTUP.md`, `Cargo.toml`, `web/index.html`, `web/package.json`, `web/README.md`, `crates/api/src/openapi.rs`, `crates/api/src/main.rs`, `crates/cli/src/main.rs`, `crates/domain/src/lib.rs`, `ATTRIBUTION.md`

Wprowadzono rebranding repo na “Bociarz LP Strategy Lab” w user-facing tekstach (README, STARTUP, CLI/API/OpenAPI oraz web title). Dodano `ATTRIBUTION.md` i zachowano upstream `LICENSE` (MIT) zgodnie z wymogami licencyjnymi.

## 2026-03-26 — Orca integration: `OrcaReadService` + `OrcaTxService` skeleton contract

**keywords:** OrcaReadService, OrcaTxService, clmm-lp-api, REST, tx-service, WhirlpoolReader, PositionReader, WhirlpoolExecutor, endpoint-map
**paths:** `crates/api/src/services/orca_read_service.rs`, `crates/api/src/services/orca_tx_service.rs`, `doc/ORCA_API_SERVICE_CONTRACT.md`, `crates/api/src/services/mod.rs`, `crates/api/src/prelude.rs`

Dodano szkielety serwisów jako jednowymiarowy kontrakt integracyjny (read REST + on-chain fallback, write on-chain) z gotową mapą endpointów/metod w `doc/ORCA_API_SERVICE_CONTRACT.md`.

---

## 2026-03-26 — API: PositionService open/close/collect wykonuje tx przez executor (dry-run testowane)

**keywords:** clmm-lp-api, PositionService, open_position, close_position, collect_fees, OrcaTxService, RebalanceExecutor, execute_open_position, executor-delegation, dry-run-tests
**paths:** `crates/api/src/services/position_service.rs`, `crates/api/src/handlers/positions.rs`, `crates/execution/src/strategy/rebalance.rs`, `crates/execution/src/strategy/executor.rs`

Zrobiono kolejne domknięcie MVP: serwis pozycji ma realna delegacje do executor-a dla `open_position/close_position/collect_fees` (z dry-runem bez wymagania walleta), a endpointy pozycji w API korzystaja z PositionService zamiast placeholderow. Dodano testy jednostkowe dla ścieżek dry-run i walidacji.

---

## 2026-03-26 — Automation: `ops-ingest-cycle` wrapper command + JSON report

**keywords:** ops-ingest-cycle, automation, Task Scheduler, snapshots, swaps-sync, swaps-enrich, decode-audit, data-health-check, clmm-lp-cli
**paths:** `crates/cli/src/main.rs`, `doc/PROJECT_OVERVIEW.md`

Dodano komendę `ops-ingest-cycle` jako „one-shot” wrapper uruchamiający cykl ingestu i metryk (snapshots → sync → enrich → audit → health-check) w jednym procesie. Komenda zapisuje raport JSON w `data/reports/` oraz ma `--fail-on-alert` do integracji z schedulerem.

---

## 2026-03-26 — Automation: `ops-ingest-loop` long-lived runner (Windows Service friendly)

**keywords:** ops-ingest-loop, windows service, nssm, automation, long-lived, backoff, jitter, clmm-lp-cli
**paths:** `crates/cli/src/main.rs`, `doc/TODO_ONCHAIN_NEXT_STEPS.md`

Dodano `ops-ingest-loop`: ciągły runner wykonujący cykl ingestu w pętli z interwałem, jitterem oraz backoff po błędach. Docelowo uruchamiany jako Windows Service (np. przez NSSM) zamiast Task Scheduler.

---

## 2026-03-26 — `swaps-subscribe-mentions`: presety `--mentions-preset` (Orca/Raydium/Meteora)

**keywords:** swaps-subscribe-mentions, mentions-preset, websocket, logsSubscribe, program-id, orca, raydium, meteora, clmm-lp-cli
**paths:** `crates/cli/src/main.rs`, `crates/cli/src/swap_sync.rs`, `doc/PROJECT_OVERVIEW.md`

Dodano `--mentions-preset <orca|raydium|meteora>` jako wygodny skrót do gotowych Program ID (z możliwością ręcznego override przez `--mentions`). Dzięki temu uruchomienie subskrypcji nie wymaga każdorazowego wpisywania pubkey.

---

## 2026-03-26 — Robust pull sync: paged `getSignaturesForAddress` + retry/backoff

**keywords:** swaps-sync-curated-all, getSignaturesForAddress, pagination, retry, backoff, max-pages, clmm-lp-cli, swap_sync
**paths:** `crates/cli/src/swap_sync.rs`, `crates/cli/src/main.rs`, `doc/PROJECT_OVERVIEW.md`

`swaps-sync-curated-all` dostał ulepszenie ścieżki pull (Opcja 3): paginację po `before` (arg `--max-pages`) oraz retry z backoff dla każdej strony RPC. Dzięki temu przy publicznych endpointach można zbierać więcej historii na run i ograniczyć dropy przy transient timeout/rate-limit bez zmiany formatu `data/swaps/.../swaps.jsonl`.

---

## 2026-03-26 — `logsSubscribe` po `mentions` do lokalnego `swaps.jsonl`

**keywords:** swaps, logsSubscribe, mentions, websocket, Solana RPC, clmm-lp-cli, swap_sync, ingest
**paths:** `crates/cli/src/swap_sync.rs`, `crates/cli/src/main.rs`, `doc/PROJECT_OVERVIEW.md`

Dodano komendę CLI `swaps-subscribe-mentions`, która otwiera websocket do RPC (`logsSubscribe` z filtrem `mentions`) i dopisuje nowe sygnatury do `data/swaps/<protocol>/<pool>/swaps.jsonl` z deduplikacją po `signature`. To jest opcjonalna ścieżka near-real-time obok istniejącego pull (`getSignaturesForAddress`) i utrzymuje ten sam format artefaktów wejściowych dla dalszego enrich/decode.

---

## 2026-03-26 — Strategy loop: `CollectFees` / `Close` on-chain + kolejność decyzji

**keywords:** StrategyExecutor, DecisionEngine, CollectFees, Close, RebalanceExecutor, execute_collect_fees_only, execute_full_close_only, auto_collect_fees, clmm-lp-execution
**paths:** `crates/execution/src/strategy/decision.rs`, `crates/execution/src/strategy/rebalance.rs`, `crates/execution/src/strategy/executor.rs`

`decide()` najpierw liczy decyzję strategii (`StaticRange` … `IlLimit`); `CollectFees` tylko gdy wynik to `Hold` i `fees_usd > min_fees_to_collect` — wcześniejszy wczesny return nie zagłusza już Periodic/OorRecenter/Threshold/RetouchShift. `execute_decision` woła `RebalanceExecutor::execute_collect_fees_only` / `execute_full_close_only` (Orca), po sukcesie lifecycle + monitor (`remove_position` po close).

---

## 2026-03-26 — Cursor rule: priorytet darmowych danych on-chain (bez płatnych zewnętrznych API)

**keywords:** cursor rules, free-onchain-data-priority, RPC, snapshots, decoded_swaps, data quality, product philosophy, no paid APIs
**paths:** `.cursor/rules/free-onchain-data-priority.mdc`

New **always-apply** rule: default design assumes **no paid external data/RPC vendors**; maximize signal from chain + local artifacts; document noise/incompleteness; prefer engineering on free inputs over buying feeds.

---

## 2026-03-26 — `swaps-enrich-curated-all`: bounded parallel `getTransaction` (M2)

**keywords:** swaps-enrich-curated-all, swap_sync, getTransaction, decode-concurrency, decode-jitter-ms, CLMM_ENRICH_DECODE_INFLIGHT, CLMM_ENRICH_DECODE_JITTER_MS, M2, B4, clmm-lp-cli, futures buffer_unordered
**paths:** `crates/cli/src/swap_sync.rs`, `crates/cli/src/main.rs`, `crates/cli/Cargo.toml`, `doc/ORCA_RUNBOOK.md`

Enrich decodes signatures with `futures::stream::buffer_unordered(decode_concurrency)` (cap 32) instead of ad-hoc `JoinSet`/`Semaphore`. New CLI flags: `--decode-concurrency` (default 4), `--decode-jitter-ms` (default 0; random delay before each decode attempt). Environment variables `CLMM_ENRICH_DECODE_INFLIGHT` and `CLMM_ENRICH_DECODE_JITTER_MS` still override when set. `decode_one_signature_with_retry` takes jitter for all paths.

---

## 2026-03-25 — Doc: work queue + phase M (M1 Meteora TVL, M2 RPC enrich queue)

**keywords:** TODO_ONCHAIN_NEXT_STEPS, ORCA_RUNBOOK, doc README, roadmap, M1, M2, B4, SOLANA_RPC_URL, Meteora, swap_sync, documentation
**paths:** `doc/TODO_ONCHAIN_NEXT_STEPS.md`, `doc/README.md`, `doc/ORCA_RUNBOOK.md`

Added *Od czego zacząć* (RPC → A1/A2 → M2 → M1 → D/E2), explicit **Faza M** checkboxes aligned with implementation plan, B4↔M2 cross-link, execution log row. README TOC points to TODO as the canonical “what to do next”. ORCA_RUNBOOK: env vars + pointer to M2 before decode params.

---

## 2026-03-25 — `optimize_apply_policy`, shared `optimization_busy`, agent JSON contract

**keywords:** optimize_apply_policy, optimization_busy, apply-optimize-result, StrategyService, AgentDecision, AgentApplyEnvelope, serde deny_unknown_fields, clmm-lp-api, clmm-lp-domain, PROJECT_OVERVIEW
**paths:** `crates/api/src/models.rs`, `crates/api/src/state.rs`, `crates/api/src/handlers/strategies.rs`, `crates/api/src/services/strategy_service.rs`, `crates/domain/src/agent_decision.rs`, `doc/PROJECT_OVERVIEW.md`

Introduced `OptimizeApplyPolicy` on `StrategyParameters` (`periodic_subprocess` | `external_http` | `combined` default): HTTP apply returns 409 when policy is subprocess-only; `external_http` + `optimize_interval_secs > 0` is rejected in `StrategyService::start_strategy`. Moved per-strategy optimize locks to `AppState.optimization_busy` so `POST /apply-optimize-result` and periodic subprocess cycles share the same `AtomicBool`; cleanup on stop/delete. `AgentDecision` and `AgentApplyEnvelope` use `#[serde(deny_unknown_fields)]` for a strict agent contract. Documented operator matrix in `PROJECT_OVERVIEW.md`.

---

## 2026-03-25 — Agent decision layer + apply-optimize HTTP + optimize JSON history

**keywords:** agent, AgentDecision, apply-optimize-result, backtest-optimize, optimize-result-json, optimize-result-json-copy-dir, StrategyExecutor, clmm-lp-api, clmm-lp-cli, clmm-lp-domain, clmm-lp-execution
**paths:** `crates/domain/src/agent_decision.rs`, `crates/execution/src/agent_decision.rs`, `crates/api/src/services/optimization_runner.rs`, `crates/api/src/handlers/strategies.rs`, `crates/cli/src/output/optimize_result_json.rs`, `crates/cli/src/main.rs`, `doc/PROJECT_OVERVIEW.md`

Added `AgentDecision` (approve/reject + optional `OptimizeResultFile`), `validate_agent_decision` with optional `agent_max_width_pct_delta` vs baseline, `POST /strategies/{id}/apply-optimize-result` applying parsed JSON without subprocess, `apply_optimize_result_parsed` shared helper, and CLI `--optimize-result-json-copy-dir` for timestamped + `latest.json` copies. Documented `StrategyService` vs HTTP + external scheduler in `PROJECT_OVERVIEW.md`.

---

## 2026-03-25 — Doc: Solana indexing concepts (`SOLANA_INDEXING.md`)

**keywords:** solana, indexing, RPC, WebSocket, Geyser, swaps-sync, clmm-lp-cli, documentation
**paths:** `doc/SOLANA_INDEXING.md`, `doc/README.md`, `doc/PROJECT_OVERVIEW.md`

Added a standalone doc describing why an SPL token does not “replicate to collect txs”, trade-offs of JSON-RPC vs subscriptions vs Geyser/providers, filtering strategies, and how that maps to the existing pull pipeline (`swaps-sync-curated-all`, `swap_sync.rs`, RPC env vars). Linked from `doc/README.md` and `PROJECT_OVERVIEW.md`.

---

<!--
Template — copy, fill, paste above the line "---" that follows the newest entry.

## YYYY-MM-DD — Short title (what you did)

**keywords:** crate-name, domain, orca|raydium|meteora, cli-flag, topic
**crates:** clmm-lp-cli, …
**paths:** `crates/.../file.rs` (optional; main touch points)

2–4 sentences: what changed, why, impact. If breaking: say **BREAKING:** explicitly.
-->

