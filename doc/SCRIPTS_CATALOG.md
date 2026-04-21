# Catalog: PowerShell tools & data pipelines

**keywords:** scripts, catalog, tools, snapshot, snapshot-health, data-health-check, snapshot-readiness, slack, alerts, monitoring, STARTUP, OPERATIONAL_CONTINUITY

**Purpose:** jeden spis tego, co leży w `tools/*.ps1`, jak to się łączy z **snapshotami / jakością danych / backtest-prep**, oraz **czy i jak** warto to spiąć z alertami (docelowo **Slack** jako wspólny punkt zborny).

**Slack jako „miejsce spotkania”:** repozytorium daje [`tools/notify_slack_webhook.ps1`](../tools/notify_slack_webhook.ps1) + `.env` (`SLACK_WEBHOOK_URL`). **Nie** ma jeszcze centralnego dyspozytora, który sam zbiera wszystkie zdarzenia — trzeba **jawnie** wołać skrypt (z cron/Task Scheduler/wrappera) przy exit code ≠ 0 lub przy zmianie pliku alertu (patrz niżej: P0).

**Powiązane dokumenty:** [`STARTUP.md`](../STARTUP.md) (pętle `scripts/windows/*` — **lokalne**, patrz sekcja *Poza repozytorium*), [`doc/SNAPSHOT_ISSUES_PLAYBOOK.md`](SNAPSHOT_ISSUES_PLAYBOOK.md), [`doc/OPERATIONAL_CONTINUITY.md`](OPERATIONAL_CONTINUITY.md), [`doc/BOT_OPERATIONS_MODEL_2026-03-23.md`](BOT_OPERATIONS_MODEL_2026-03-23.md).

---

## Model pracy skryptów (manual / triggered / automatic)

Poniższy podział odpowiada na “jak to u nas działa operacyjnie” — czyli **kto/ co odpala skrypt** i **gdzie trafiają alerty na Slacka**.

### 1) Manual (jednorazowe)
Odpala je operator “na żądanie”, gdy chce audyt/porównanie/diagnostykę.

Przykłady: `tools/quick_verify_data.ps1`, `tools/compare_orca_snapshots_5m_vs_10m_last_full_hour.ps1`, `tools/check_collector_5m_status.ps1`, `tools/restart_snapshot_loop_10m.ps1`, `tools/count_snapshot_rows_last_full_hour.ps1`.

### 2) Triggered (wyzwalane przez harmonogram)
Harmonogram odpala jeden shot; skrypt wysyła Slacka, gdy wykryje problemy (z throttle).

Przykłady: `tools/snapshot_health_alert.ps1` i `tools/quick_verify_alert.ps1` (w środku wywołują `snapshot_health_check.ps1` oraz `quick_verify_data.ps1`).

### 3) Automatic (ciągłe pętle / usługi)
Długowieczne procesy utrzymujące spójność snapshotów i alertów bez zewnętrznych wyzwalaczy.

Mechanizmy automatyzacji:
- `tools/data_alerts_loop.ps1`: pętla “co X sekund/minut” uruchamia `snapshot_health_alert.ps1` oraz opcjonalnie `quick_verify_alert.ps1`. Docelowo uruchamiane pod **Shawl / NSSM**.
- `tools/register_snapshot_health_scheduled_task.ps1`: **jednorazowa** rejestracja zadania Harmonogramu Windows (**CLMM-SnapshotHealthAlert**), które co kilka minut odpala `snapshot_health_alert.ps1` — bez ręcznego sprawdzania (wymaga `.env` z `SLACK_WEBHOOK_URL`, chyba że `-SkipSlack`).
- `tools/run_snapshot_health_monitor_loop.ps1`: pętla “health-only” (bez Slacka), stale dopisuje logi snapshotów.
- `tools/run_snapshot_backtest_prep_loop.ps1`: pętla odświeżająca snapshoty i przygotowująca cache przez `snapshot-backtest-prep` (może być `-Loop` jako tryb ciągły, albo jednorazowo pod Task Scheduler).

### Task Scheduler (Windows) — które skrypty i po co
Na Windows najczęściej używamy Task Scheduler do **jednorazowych** “shotów”, żeby uruchamiać wrappery alertów (Slack przez throttle) oraz cache przygotowania pod backtesty.

Skrypty:
- `tools/snapshot_health_alert.ps1`: okresowo uruchamiany shot (najczęściej co ~10 minut) — gdy `snapshot_health_check` wykryje problemy, wyśle Slack.
- `tools/quick_verify_alert.ps1`: okresowo uruchamiany shot (najczęściej co ~60 minut) — gdy `quick_verify_data` zwróci NO-GO, wyśle Slack.
- `tools/run_snapshot_backtest_prep_loop.ps1`: okresowo (najczęściej co ~30 minut, one-shot) — robi `snapshot-run-curated-all` + `snapshot-backtest-prep` + (opcjonalnie) `snapshot-readiness`, żeby cache było świeże pod `backtest-optimize --price-path-source snapshots`.
- `tools/orca_bot_run_supervised.ps1`: może być odpalany “At startup” przez Task Scheduler jako wrapper restartujący `orca-bot-run` po błędach (albo alternatywnie pod NSSM/Shawl).

Slack: wspólny punkt przez wrappery
Slack jest “spięty” przez `tools/notify_slack_webhook.ps1` oraz wrappery `snapshot_health_alert.ps1` / `quick_verify_alert.ps1`, które decydują o wysyłce i throttle.

Cache do backtestów:
`tools/run_snapshot_backtest_prep_loop.ps1` (przez `snapshot-backtest-prep`) generuje `data/backtest-snapshot-cache/*/pool_meta.json`, żeby `backtest-optimize --price-path-source snapshots` nie zależał od RPC dla `*_decimals`/`tick_spacing`.

## Recommended Automatic set (Windows)
Jeśli chcesz “bez Task Scheduler” i 24/7, rekomendowany minimalny zestaw usług:

1. **Alerty danych (Slack):** `tools/data_alerts_loop.ps1` (Shawl/NSSM)
2. **Cache do backtestów (okna + meta):** `tools/run_snapshot_backtest_prep_loop.ps1 -Loop -SlackOnError -LogFile data/snapshot_logs/snapshot-backtest-prep-loop.log`
3. **Ingest (snapshots+swaps pipeline) jako CLI:** `tools/run_ops_ingest_loop.ps1` (wrapuje `clmm-lp-cli ops-ingest-loop` pod usługę)
4. **Bot runtime:** `tools/orca_bot_run_supervised.ps1 ...` (jeśli bot ma działać stale)

### Shawl — przykładowe komendy
Uruchamiasz raz (rejestracja), potem startujesz usługę:

- `shawl add --name clmm-data-alerts --cwd F:\CLMM-Liquidity-Provider\CLMM-Liquidity-Provider -- powershell.exe -NoProfile -ExecutionPolicy Bypass -File F:\CLMM-Liquidity-Provider\CLMM-Liquidity-Provider\tools\data_alerts_loop.ps1`
- `shawl add --name clmm-snapshot-backtest-prep --cwd F:\CLMM-Liquidity-Provider\CLMM-Liquidity-Provider -- powershell.exe -NoProfile -ExecutionPolicy Bypass -File F:\CLMM-Liquidity-Provider\CLMM-Liquidity-Provider\tools\run_snapshot_backtest_prep_loop.ps1 -Loop -SlackOnError -LogFile F:\CLMM-Liquidity-Provider\CLMM-Liquidity-Provider\data\snapshot_logs\snapshot-backtest-prep-loop.log`
- `shawl add --name clmm-ops-ingest-loop --cwd F:\CLMM-Liquidity-Provider\CLMM-Liquidity-Provider -- powershell.exe -NoProfile -ExecutionPolicy Bypass -File F:\CLMM-Liquidity-Provider\CLMM-Liquidity-Provider\tools\run_ops_ingest_loop.ps1 -SlackOnError`

NSSM jest analogiczne (aplikacja `powershell.exe`, argumenty `-File ...`), patrz `STARTUP.md` oraz `doc/OPERATIONAL_CONTINUITY.md`.

### Weryfikacja: co faktycznie odpala Task Scheduler u Ciebie
Na tej maszynie Task Scheduler nie odpala `.ps1` (brak wpisów z `powershell.exe`/`*.ps1` w akcjach). Aktualnie odpala **Rust CLI** zadania:

1. `clmm-lp snapshot-run-curated-all`
   - tryb: daily
   - akcja: `target\release\clmm-lp-cli.exe snapshot-run-curated-all`
2. `clmm-lp swaps-sync+enrich-curated-all`
   - tryb: daily
   - akcja: uruchamia `target\release\clmm-lp-cli.exe` z komendy `cmd.exe` (z root repo jako `cd /d ...`)

To oznacza, że Twoje PS-wrappers (`snapshot_health_alert.ps1`, `quick_verify_alert.ps1`, `run_snapshot_backtest_prep_loop.ps1`) są u Ciebie najpewniej uruchamiane innym mechanizmem (np. Shawl/NSSM) albo ręcznie, a Task Scheduler robi “ciężkie” kroki (snapshoty + swaps sync/enrich) jako CLI exec.

### Owner / run-mode (operacyjny łańcuch danych -> Slack)
Poniżej jest dopięte “kto to obsługuje” (od strony odpowiedzialności) i “jaki mechanizm uruchomieniowy” dla skryptów, które realnie spinają kompletność/synchronizację i alerty.

| Skrypt / komenda | Kto obsługuje | Mechanizm uruchomienia | Co sprawdza / robi |
|---|---|---|---|
| `clmm-lp snapshot-run-curated-all` | Data Collector (bot/ops) | Task Scheduler (daily) | Odświeża snapshoty oraz JSONL statusy dla snapshotów 10m/5m |
| `clmm-lp swaps-sync+enrich-curated-all` | Data Collector (bot/ops) | Task Scheduler (daily) | Sync swapów + enrich/decoded dane dla pipeline’u backtest/fees |
| `tools/snapshot_health_check.ps1` | Data Quality Gate | wywoływane przez wrapper (one-shot) | Świeżość OK-runów JSONL + (opcj.) heartbeat pętli `run-snapshot-loop*.ps1` + ERROR w logach |
| `tools/snapshot_health_alert.ps1` | Data Ops Alerting | triggered shot (najczęściej Task Scheduler / cron) albo wywołania w `data_alerts_loop` | Na NOT OK wysyła Slack (z throttle) |
| `tools/quick_verify_data.ps1` | Data Quality Gate | wywoływane przez wrapper (one-shot) | Agreguje readiness + `data-health-check` + opcjonalnie decode audit |
| `tools/quick_verify_alert.ps1` | Data Ops Alerting | triggered shot (najczęściej hourly) albo wywołania w `data_alerts_loop` | Na NO-GO wysyła Slack (throttle) |
| `tools/data_alerts_loop.ps1` | Data Ops Alerting | automatic long-lived (Shawl/NSSM) | Jednym procesem robi cyklicznie `snapshot_health_alert` i `quick_verify_alert` |
| `tools/register_snapshot_health_scheduled_task.ps1` | Data Ops Alerting | **jednorazowa** rejestracja taska Windows (potem automatycznie) | Harmonogram: co N min `snapshot_health_alert` → Slack przy NOT OK |
| `tools/run_snapshot_backtest_prep_loop.ps1` | Research / Backtest Ops | triggered shot albo loop (one-shot + scheduler) | Buduje cache `data/backtest-snapshot-cache` pod `backtest-optimize --price-path-source snapshots` |
| `tools/run_ops_ingest_loop.ps1` | Data Collector (bot/ops) | automatic long-lived (Shawl/NSSM) | Wrapuje `clmm-lp-cli ops-ingest-loop` pod usługę (ingest: snapshots → swaps sync → enrich → audit → health-check) + opcjonalny Slack na non-zero exit |
| `tools/run_snapshot_health_monitor_loop.ps1` | Data Quality Gate | automatic long-lived (loop) | Tylko loguje “health” do plików, bez Slacka |
| `tools/orca_bot_run_supervised.ps1` | Bot Supervisor | OS-level supervision: Task Scheduler (At startup) albo NSSM/Shawl | Restartuje `orca-bot-run` jeśli proces wyjdzie z błędem |
| `tools/log_rotate.ps1` | Data Ops / Operator | triggered shot (Task Scheduler, daily) | Retencja: usuwa stare logi/raporty pod `data/` (żeby repo nie rosło bez końca przy usługach) |

### Task Scheduler (Windows) — gotowy task dla `tools/log_rotate.ps1`
Skrypt `tools/log_rotate.ps1` jest celowo **one-shot** i najlepiej odpalać go jako jeden task dziennie.

**Proponowana nazwa taska:** `clmm-lp log-rotate`

**Trigger (przykład):**
- Daily, godzina np. **03:30**

**Action:**
- **Program/script:** `powershell.exe`
- **Add arguments:**
  - `-NoProfile -ExecutionPolicy Bypass -File "F:\CLMM-Liquidity-Provider\CLMM-Liquidity-Provider\tools\log_rotate.ps1" -KeepDays 14`
- **Start in:**
  - `F:\CLMM-Liquidity-Provider\CLMM-Liquidity-Provider`

**Test (bez kasowania):**
- `powershell.exe -NoProfile -ExecutionPolicy Bypass -File "F:\CLMM-Liquidity-Provider\CLMM-Liquidity-Provider\tools\log_rotate.ps1" -KeepDays 14 -WhatIf`

Uwaga: `log_rotate.ps1` usuwa tylko stare pliki spod `data/` (logi/raporty/alert-state). Nie dotyka `data/pool-snapshots/**` ani ledgerów bota, więc jest bezpieczny jako automatyczna retencja.

## Owner / run-mode dla `tools/*.ps1`
Poniższa tabela opisuje **kto** typowo “odpala” skrypt i **jaki mechanizm** jest do tego używany:
- `Automatic` = long-lived proces (Shawl/NSSM)
- `Triggered` = one-shot cyklicznie od harmonogramu (Task Scheduler / cron)
- `Manual` = operator uruchamia ręcznie (albo używa jako helper/dot-source)

| Script | Owner | Run-mode |
|---|---|---|
| `tools/bot_postrun_report.ps1` | Operator | Manual / one-shot (operator) |
| `tools/bot_preflight.ps1` | Operator | Manual / one-shot (operator) |
| `tools/bot_run_devnet.ps1` | Operator | Manual / one-shot (operator) |
| `tools/bot_session_devnet.ps1` | Operator | Manual / one-shot (operator) |
| `tools/build_clmm_lp_cli_release.ps1` | Operator | Manual / one-shot (operator) |
| `tools/build_clmm_lp_cli.ps1` | Operator | Manual / one-shot (operator) |
| `tools/check_collector_5m_status.ps1` | Operator | Manual / one-shot (operator) |
| `tools/clmm_rpc_tools_helpers.ps1` | Shared helpers | Dot-source library (manual usage by other scripts) |
| `tools/compare_orca_snapshots_5m_vs_10m_last_full_hour.ps1` | Operator | Manual / one-shot (operator) |
| `tools/count_snapshot_rows_last_full_hour.ps1` | Operator | Manual / one-shot (operator) |
| `tools/data_alerts_loop.ps1` | Data Ops Alerting | Automatic (Shawl/NSSM long-lived loop) |
| `tools/devnet_rebalance_wallet_half.ps1` | Operator | Manual / one-shot (operator) |
| `tools/mainnet_rpc_env.example.ps1` | Operator | Manual / one-shot (operator) |
| `tools/mainnet_rpc_env.ps1` | Operator | Manual / one-shot (operator) |
| `tools/new_bot_keypair.ps1` | Operator | Manual / one-shot (operator) |
| `tools/log_rotate.ps1` | Data Ops / Operator | Triggered (Task Scheduler daily one-shot retention) |
| `tools/notify_slack_webhook.ps1` | Operator | Manual / one-shot (operator) |
| `tools/orca_bot_run_supervised.ps1` | Bot Supervisor | Automatic (NSSM/Shawl long-lived restart loop) |
| `tools/orca_curated_mainnet_pools.ps1` | Operator | Manual / one-shot (operator) |
| `tools/orca_curated_rebalance.ps1` | Operator | Manual / one-shot (operator) |
| `tools/orca_fund_cbbtc_usdc_open.ps1` | Operator | Manual / one-shot (operator) |
| `tools/orca_position_auto_fund_for_open.ps1` | Operator | Manual / one-shot (operator) |
| `tools/orca_position_close_quick.ps1` | Operator | Manual / one-shot (operator) |
| `tools/orca_position_open_preflight.ps1` | Operator | Manual / one-shot (operator) |
| `tools/orca_position_open_then_close_fast.ps1` | Operator | Manual / one-shot (operator) |
| `tools/orca_position_open_then_close_quick.ps1` | Operator | Manual / one-shot (operator) |
| `tools/orca_position_preflight_core.ps1` | Operator | Manual / one-shot (operator) |
| `tools/orca_position_smoke_curated_pools.ps1` | Operator | Manual / one-shot (operator) |
| `tools/orca_swap_curated.ps1` | Operator | Manual / one-shot (operator) |
| `tools/orca_swap.ps1` | Operator | Manual / one-shot (operator) |
| `tools/orca_wheth_sol_three_bots_plan.ps1` | Operator | Manual / one-shot (operator) |
| `tools/quick_verify_alert.ps1` | Data Ops Alerting | Triggered (Task Scheduler / cron one-shot; Slack on NO-GO) |
| `tools/quick_verify_data.ps1` | Data Quality Gate | One-shot audit (called by quick_verify_alert / data_alerts_loop) |
| `tools/restart_snapshot_loop_10m.ps1` | Operator | Manual / one-shot (operator) |
| `tools/run_devnet_smokes.ps1` | Operator | Manual / one-shot (operator) |
| `tools/run_snapshot_backtest_prep_loop.ps1` | Research / Backtest Ops | Triggered (Task Scheduler one-shot every ~30m, or -Loop) |
| `tools/run_ops_ingest_loop.ps1` | Data Collector (bot/ops) | Automatic (Shawl/NSSM long-lived runner; `-SlackOnError` on non-zero exit) |
| `tools/run_snapshot_health_monitor_loop.ps1` | Data Quality Gate | Automatic (Shawl/NSSM long-lived loop; check-only, no Slack) |
| `tools/snapshot_health_alert.ps1` | Data Ops Alerting | Triggered (Task Scheduler / cron one-shot; Slack on NOT OK) |
| `tools/snapshot_health_check.ps1` | Data Quality Gate | One-shot check (called by snapshot_health_alert / data_alerts_loop / monitor loop) |
| `tools/solana_account_state.ps1` | Operator | Manual / one-shot (operator) |
| `tools/solana_rpc_env.ps1` | Operator | Manual / one-shot (operator) |
| `tools/solana_wallet_usd_estimate.ps1` | Operator | Manual / one-shot (operator) |
| `tools/Start-Dashboard.ps1` | Operator | Manual / one-shot (operator) |
| `tools/Stop-ClmmApi.ps1` | Operator | Manual / one-shot (operator) |
| `tools/wheth_sol_three_bots_manual_range_25_25p5.ps1` | Operator | Manual / one-shot (operator) |

## P0 — Ciągłość snapshotów i jakość danych (pierwsze do alertów)

W praktyce “kompletność / brak błędów / sync / luki” mapujemy na zestaw checków:
- `snapshot-readiness`: bramka Tier1/Tier2/Tier3 (gotowość pod fees/IL) dla konkretnego poola
- `data-health-check`: świeżość (max age) + jakość decode (min % ok)
- `snapshot_health_check`: wykrywanie świeżych ERROR w logach pętli snapshotów
- (opcjonalnie) `swaps-decode-audit`: audyt jakości dekodowania swapów

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
