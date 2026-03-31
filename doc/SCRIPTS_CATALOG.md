# Catalog: PowerShell tools & data pipelines

**keywords:** scripts, catalog, tools, snapshot, snapshot-health, data-health-check, snapshot-readiness, slack, alerts, monitoring, STARTUP, OPERATIONAL_CONTINUITY

**Purpose:** jeden spis tego, co leży w `tools/*.ps1`, jak to się łączy z **snapshotami / jakością danych / backtest-prep**, oraz **czy i jak** warto to spiąć z alertami (docelowo **Slack** jako wspólny punkt zborny).

**Slack jako „miejsce spotkania”:** repozytorium daje [`tools/notify_slack_webhook.ps1`](../tools/notify_slack_webhook.ps1) + `.env` (`SLACK_WEBHOOK_URL`). **Nie** ma jeszcze centralnego dyspozytora, który sam zbiera wszystkie zdarzenia — trzeba **jawnie** wołać skrypt (z cron/Task Scheduler/wrappera) przy exit code ≠ 0 lub przy zmianie pliku alertu (patrz niżej: P0).

**Powiązane dokumenty:** [`STARTUP.md`](../STARTUP.md) (pętle `scripts/windows/*` — **lokalne**, patrz sekcja *Poza repozytorium*), [`doc/SNAPSHOT_ISSUES_PLAYBOOK.md`](SNAPSHOT_ISSUES_PLAYBOOK.md), [`doc/OPERATIONAL_CONTINUITY.md`](OPERATIONAL_CONTINUITY.md), [`doc/BOT_OPERATIONS_MODEL_2026-03-23.md`](BOT_OPERATIONS_MODEL_2026-03-23.md).

---

## P0 — Ciągłość snapshotów i jakość danych (pierwsze do alertów)

Te elementy najlepiej **najpierw** przełożyć na Slack: świeżość kolekcji, brak OK runów, błędy w logach pętli, regres jakości decode/snapshot readiness.

| Zasób | Tryb | Co robi | Wynik / artefakty | Alert → Slack (rekomendacja) |
| ----- | ---- | ------- | ------------------- | ----------------------------- |
| [`tools/snapshot_health_check.ps1`](../tools/snapshot_health_check.ps1) | **one-shot** | Czyta `snapshot-run-curated-all*.jsonl`, wiek ostatniego OK, `orca.target/success`, szuka `ERROR` w logach pętli 10m/5m | Dopisuje `data/snapshot_logs/snapshot-health.jsonl`; przy problemie **exit 1**; opcjonalnie **edge-trigger** `data/agent-alerts/snapshot-health/latest.json` | **Tak — priorytet 1:** użyj [`tools/snapshot_health_alert.ps1`](../tools/snapshot_health_alert.ps1) (wrapper + throttle) lub ręcznie `notify_slack_webhook.ps1` |
| [`tools/snapshot_health_alert.ps1`](../tools/snapshot_health_alert.ps1) | **one-shot** | Wywołuje `snapshot_health_check.ps1`; przy **exit 1** wysyła Slack z listą `issues` (throttle domyślnie **15 min** dla tego samego zestawu problemów) | Stan throttlingu: `data/agent-alerts/snapshot-slack-throttle/state.json` (w `data/` — nie commituj) | **Tak — gotowe pod harmonogram** |
| [`tools/run_snapshot_health_monitor_loop.ps1`](../tools/run_snapshot_health_monitor_loop.ps1) | **pętla** | Co `IntervalSeconds` odpala `snapshot_health_check.ps1`, loguje do `data/snapshot_logs/snapshot-health-loop.log` | Ciągły nadzór; **nie wysyła Slacka** sam z siebie | **Tak:** albo wrapper „jeśli exit 1 → slack”, albo osobny harmonogram co N min robi **jeden** shot `snapshot_health_check` + Slack (prostsze) |
| [`tools/run_snapshot_backtest_prep_loop.ps1`](../tools/run_snapshot_backtest_prep_loop.ps1) | **one-shot lub `-Loop`** | `snapshot-run-curated-all` → `snapshot-backtest-prep` → (opcj.) `snapshot-readiness` na wybranych poolach Orca | Cache pod `data/backtest-snapshot-cache/`; **throw** przy NOT READY | **Tak:** catch w schedulerze / `-Loop` wrapper → Slack przy wyjątku |
| [`tools/quick_verify_data.ps1`](../tools/quick_verify_data.ps1) | **one-shot** | Agreguje: `snapshot-readiness` (kilka pooli), `data-health-check`, (opcj.) `swaps-decode-audit` | Podsumowanie GO/NO-GO w stdout; **exit 2** przy NO-GO | **Tak:** użyj [`tools/quick_verify_alert.ps1`](../tools/quick_verify_alert.ps1) (rzadziej niż snapshot health, np. co godzinę) |
| [`tools/quick_verify_alert.ps1`](../tools/quick_verify_alert.ps1) | **one-shot** | Woła `quick_verify_data.ps1`; przy błędzie Slack + throttle (domyślnie **60 min**) | `data/agent-alerts/quick-verify-slack-throttle/state.json` | **Tak — P0 szeroki audyt** |
| [`tools/check_collector_5m_status.ps1`](../tools/check_collector_5m_status.ps1) | **one-shot** | Diagnostyka 5m: tail statusu + logu + statystyki plików pooli | Tylko konsola | **Opcjonalnie:** ręcznie / jako załącznik do incydentu; rzadziej automat |
| [`tools/count_snapshot_rows_last_full_hour.ps1`](../tools/count_snapshot_rows_last_full_hour.ps1) | **one-shot** | Liczy wiersze snapshotów w oknie godziny | Konsola | **Opcjonalnie:** alert gdy poniżej progu (wymagałoby progów w skrypcie lub zewnętrznego parsera) |
| [`tools/compare_orca_snapshots_5m_vs_10m_last_full_hour.ps1`](../tools/compare_orca_snapshots_5m_vs_10m_last_full_hour.ps1) | **one-shot** | Porównanie backtestów 5m vs 10m dla ostatniej pełnej godziny | CSV w `data/reports/` | **Opcjonalnie:** Slack tylko gdy różnice przekroczą próg (nie zaimplementowane) |
| [`tools/restart_snapshot_loop_10m.ps1`](../tools/restart_snapshot_loop_10m.ps1) | **one-shot** | Restart lokalnej pętli 10m (zakłada proces z `scripts/windows/…`) | Procesy PowerShell | **Operacyjne:** po restarcie można Slack „info”, nie krytyczne |

### CLI (Rust) używane przez powyższe — też kandydaci do alertów

| Komenda | Rola |
| ------- | ---- |
| `snapshot-run-curated-all` | Odświeża snapshoty + status JSONL pod `data/snapshot_logs/` |
| `snapshot-backtest-prep` | Przygotowuje wąskie okna do szybkiego backtestu |
| `snapshot-readiness --protocol … --pool-address …` | Tier1/2/3 gotowość pod fees/IL dla danej puli |
| `data-health-check --max-age-minutes … --min-decode-ok-pct …` | Świeżość raw/decoded/snapshot + jakość decode |
| `swaps-decode-audit` | Raport jakości dekodowania swapów |

---

## Pełny spis: `tools/*.ps1`

Szukaj po **keywords** w tej tabeli (Ctrl+F) lub po nazwie pliku.

| Plik | keywords | Tryb | Krótki opis |
| ---- | -------- | ---- | ----------- |
| `notify_slack_webhook.ps1` | slack, webhook, alert | one-shot | POST na Incoming Webhook; czyta `.env` |
| `orca_bot_run_supervised.ps1` | orca-bot, supervisor, restart | pętla | Wrapper `orca-bot-run` z restartem po błędzie |
| `clmm_rpc_tools_helpers.ps1` | rpc, denylist, Resolve-ClmmLpCliExe | library | Dot-source dla innych skryptów |
| `build_clmm_lp_cli.ps1` | build, cargo, clmm-lp-cli | one-shot | Build `clmm-lp-cli` (**Release** domyślnie, `-Configuration Debug`) |
| `build_clmm_lp_cli_release.ps1` | build, release, cargo | wrapper | To samo co `build_clmm_lp_cli.ps1 -Configuration Release` (kompatybilność wstecz) |
| `snapshot_health_check.ps1` | snapshot-health, jsonl, exit1 | one-shot | Zdrowie kolektorów 10m+5m |
| `snapshot_health_alert.ps1` | snapshot-health, slack, throttle, alert | one-shot | `snapshot_health_check` + Slack przy błędzie |
| `data_alerts_loop.ps1` | slack, snapshot-health, quick-verify, loop, shawl, nssm | pętla | `snapshot_health_alert` + `quick_verify_alert` bez Task Scheduler |
| `run_snapshot_health_monitor_loop.ps1` | snapshot-health, loop | pętla | Okresowe `snapshot_health_check` |
| `run_snapshot_backtest_prep_loop.ps1` | backtest-prep, snapshot-readiness, loop | one-shot / `-Loop` | Kolekcja + prep + readiness |
| `restart_snapshot_loop_10m.ps1` | snapshot-loop, restart | one-shot | Restart pętli 10m (Windows) |
| `check_collector_5m_status.ps1` | 5m, collector, diagnostic | one-shot | Diagnostyka 5m |
| `count_snapshot_rows_last_full_hour.ps1` | rows, hour, orca | one-shot | Liczniki wierszy w godzinie |
| `compare_orca_snapshots_5m_vs_10m_last_full_hour.ps1` | 5m, 10m, backtest, compare | one-shot | Raport CSV porównawczy |
| `quick_verify_data.ps1` | verify, readiness, health-check, decode | one-shot | Szybki audyt GO/NO-GO |
| `quick_verify_alert.ps1` | quick-verify, slack, throttle, alert | one-shot | `quick_verify_data` + Slack przy NO-GO |
| `mainnet_rpc_env.ps1` | rpc, mainnet, env | one-shot / dot-source | Ustawia RPC (lokalna kopia z example) |
| `mainnet_rpc_env.example.ps1` | rpc, template | template | Wzorzec bez sekretów |
| `solana_rpc_env.ps1` | rpc, solana | env | Pomocnicze RPC |
| `orca_swap.ps1` | orca-swap, jupiter | one-shot | Wywołanie CLI swap |
| `orca_curated_rebalance.ps1` | orca, curated, swap, preflight, open, close, rebalance | dispatcher | Jedna bramka: ListPairs / Preflight / Open / Close / Swap / FundCbBtc / Smoke (3 pary mainnet) |
| `orca_swap_curated.ps1` | orca-swap, curated | one-shot | Swapy na pulach curated |
| `orca_curated_mainnet_pools.ps1` | pools, curated | library | Lista pul |
| `orca_position_preflight_core.ps1` | preflight, open | library | Wspólna logika preflight |
| `orca_position_open_preflight.ps1` | preflight, open | one-shot | Preflight przed open |
| `orca_position_auto_fund_for_open.ps1` | autofund, swap | one-shot | Auto-fund przed open |
| `orca_position_open_then_close_quick.ps1` | open, close, smoke | one-shot | Open→close (quick) |
| `orca_position_open_then_close_fast.ps1` | open, close, fast | one-shot | Open→close bez czekania na ledger |
| `orca_position_close_quick.ps1` | close | one-shot | Szybkie zamknięcie |
| `orca_position_smoke_curated_pools.ps1` | smoke, curated | one-shot | Smoke na wielu poolach |
| `orca_fund_cbbtc_usdc_open.ps1` | cbbtc, usdc, fund | one-shot | Fundowanie pod konkretną pulę |
| `solana_account_state.ps1` | account, rpc | one-shot | Stan konta Solana |
| `solana_wallet_usd_estimate.ps1` | wallet, usd, portfolio, CoinGecko | one-shot | Szacunek wartości portfela w USD (SOL+wSOL, USDC, cbBTC~BTC, whETH~ETH) |
| `orca_wheth_sol_three_bots_plan.ps1` | WHETH_SOL, plan, 3 bots, capital | one-shot | Plan podziału kapitału na 3 pozycje + komendy (bez tx) |
| `new_bot_keypair.ps1` | keypair, wallet | one-shot | Nowy keypair bota |
| `bot_run_devnet.ps1` | devnet, bot | one-shot / sesja | Run bota devnet + ledgery |
| `bot_session_devnet.ps1` | devnet, session | one-shot | Sesja devnet |
| `bot_preflight.ps1` | devnet, preflight | one-shot | Preflight bota |
| `bot_postrun_report.ps1` | devnet, report | one-shot | Raport po runie |
| `run_devnet_smokes.ps1` | devnet, smoke | one-shot | Pakiet smoke devnet |
| `devnet_rebalance_wallet_half.ps1` | devnet, wallet | one-shot | Operacja na portfelu devnet |

---

## Poza repozytorium (ważne)

Folder **`/scripts/` jest w `.gitignore`** — typowo trzymasz tam **własne** pętle, np. opisane w [`STARTUP.md`](../STARTUP.md):

- `scripts/windows/run-snapshot-loop.ps1` — snapshot 10m → `snapshot-loop.log`, `snapshots.jsonl`
- `scripts/windows/run-snapshot-loop-5m.ps1` — snapshot 5m → `snapshots_5m.jsonl`
- `scripts/windows/run-swaps-pipeline-loop.ps1` — pipeline swapów

**Nie ma ich w git clone** — katalog trzeba mieć lokalnie lub odtworzyć z runbooka. `snapshot_health_check.ps1` **zakłada**, że te pętle zasilają `data/snapshot_logs/*.jsonl` i logi.

---

## Wrapper alertów (P0 — zaimplementowane)

1. **`tools/snapshot_health_alert.ps1`** — wywołuje `snapshot_health_check.ps1`; przy **exit 1** woła `notify_slack_webhook.ps1`; parametr **`-MinMinutesBetweenSameIssues`** (domyślnie 15) ogranicza powtórki przy tym samym zestawie `issues`.
2. **Harmonogram:** co 5–10 min **jeden** shot `snapshot_health_alert.ps1` (nie wysyłaj Slacka z każdej iteracji `run_snapshot_health_monitor_loop` — tam tylko log lokalny).
3. **`tools/quick_verify_alert.ps1`** — ten sam wzorzec dla `quick_verify_data` (domyślny throttle **60 min**); harmonogram np. **co 60 min** (osobno od `snapshot_health_alert`).

---

## Jak szybko wyszukiwać

- **Po domenie:** sekcja P0 (snapshot) vs tabela pełna + kolumna `keywords`.
- **Po nazwie:** `Get-ChildItem tools\*.ps1` lub w IDE wyszukiwanie w `tools/`.
- **Po CLI:** sekcja „CLI używane przez powyższe” + [`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md).
