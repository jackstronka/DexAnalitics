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

