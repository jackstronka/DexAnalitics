# Operational continuity (bot & API)

**Purpose:** turn the long-running **CLI bot** (`orca-bot-run`) or **API + StrategyService** into something that survives crashes, reboots, and is observable — without changing strategy code.

**Related:** [`ORCA_RUNBOOK.md`](ORCA_RUNBOOK.md) (Orca CLI), [`MAINNET_OPERATIONAL_CHECKLIST.md`](MAINNET_OPERATIONAL_CHECKLIST.md), [`BOT_OPERATIONS_MODEL_2026-03-23.md`](BOT_OPERATIONS_MODEL_2026-03-23.md) (modes & alerts), [`RPC_SOLANA_BOT_NOTES.md`](RPC_SOLANA_BOT_NOTES.md), [`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md) (optimize → apply paths), [`POSITION_REGISTRY.md`](POSITION_REGISTRY.md) (**szybki podgląd aktywnych pozycji:** replay `registry.jsonl`, API, `orca-positions-list`).

## What ships in-repo

| Artifact | Role |
| -------- | ---- |
| [`deploy/systemd/clmm-lp-orca-bot.service.example`](../deploy/systemd/clmm-lp-orca-bot.service.example) | **Linux:** `systemd` unit template (`Restart=always`, env file, `ExecStart`). |
| [`tools/orca_bot_run_supervised.ps1`](../tools/orca_bot_run_supervised.ps1) | **Windows:** restart loop around `clmm-lp-cli orca-bot-run` + optional log files. |
| [`tools/notify_slack_webhook.ps1`](../tools/notify_slack_webhook.ps1) | **Slack:** POST `text` to Incoming Webhook (`SLACK_WEBHOOK_URL`). |
| **`clmm-lp-api`** + **web** | **Historia operacji (3Commas‑style timeline):** `GET /api/v1/bot-activity/ledger` i `/registry` czytają te same pliki JSONL co CLI (`data/ledger/orca_position_lifecycle.jsonl`, `data/positions/registry.jsonl`). Strona dashboardu **Bot activity** (`/bot-activity`). `POST /api/v1/bot-activity/slack-summary` wysyła skrót ostatnich wierszy ledgera na Slack (wymaga `SLACK_WEBHOOK_URL` w **env procesu API**, nie tylko w `.env` użytkownika). |
| [`tools/snapshot_health_alert.ps1`](../tools/snapshot_health_alert.ps1) | **Snapshots:** `snapshot_health_check` → przy błędzie Slack (throttle). |
| [`tools/quick_verify_alert.ps1`](../tools/quick_verify_alert.ps1) | **Dane:** `quick_verify_data` → przy NO-GO Slack (throttle 60m). |
| [`doc/SCRIPTS_CATALOG.md`](SCRIPTS_CATALOG.md) | Spis skryptów + priorytety alertów. |
| [`tools/data_alerts_loop.ps1`](../tools/data_alerts_loop.ps1) | **Bez Task Scheduler:** jedna długa pętla → `snapshot_health_alert` + `quick_verify_alert` (pod Shawl/NSSM). |
| [`Docker/orca-bot.compose.example.yml`](../Docker/orca-bot.compose.example.yml) | **Docker:** `restart: unless-stopped` + CLI image; adjust secrets/volumes locally. |

## 1. Process supervision

The bot is a **normal OS process** (Tokio loop). You add **supervision** underneath.

### Linux (`systemd`)

1. Copy [`deploy/systemd/clmm-lp-orca-bot.service.example`](../deploy/systemd/clmm-lp-orca-bot.service.example) to `/etc/systemd/system/clmm-lp-orca-bot.service` and edit:
   - `User` / `Group`
   - `WorkingDirectory`
   - `ExecStart` (absolute path to `clmm-lp-cli` and full `orca-bot-run …` arguments)
   - `EnvironmentFile=` path (RPC, cluster guard, optional `KEYPAIR_PATH`)
2. `sudo systemctl daemon-reload && sudo systemctl enable --now clmm-lp-orca-bot`
3. `journalctl -u clmm-lp-orca-bot -f` for logs.

Use `Restart=always` and `RestartSec=10` for crash recovery. For **API** instead of CLI, duplicate the unit with `ExecStart=` pointing at `clmm-lp-api` and depend on PostgreSQL (see [`Docker/docker-compose.yml`](../Docker/docker-compose.yml) for env hints).

### Windows

- **Preferowane zamiast Task Scheduler (alerty danych):** jeden proces **`tools/data_alerts_loop.ps1`** — wewnątrz `Start-Sleep` i kolejne wywołania `snapshot_health_alert` / `quick_verify_alert`. Uruchom **raz** „po starcie systemu” przez:
  - **[Shawl](https://github.com/mtkennerly/shawl)** (jak w [`STARTUP.md`](../STARTUP.md) dla pętli snapshotów):  
    `shawl add --name clmm-data-alerts --cwd F:\path\to\CLMM-Liquidity-Provider -- powershell.exe -NoProfile -ExecutionPolicy Bypass -File F:\path\to\CLMM-Liquidity-Provider\tools\data_alerts_loop.ps1`
  - **[NSSM](https://nssm.cc/)** — jako aplikacja `powershell.exe`, argumenty `-NoProfile -ExecutionPolicy Bypass -File …\data_alerts_loop.ps1`, katalog roboczy = root repo; stdout/stderr do pliku (np. `data/snapshot_logs/`).
  - **Ręcznie / RDP:** jedno okno PowerShell z pętlą (tylko na dev).
- **Task Scheduler:** nadal OK dla **jednorazowych** zadań lub jeśli wolisz harmonogram OS; nie jest wymagany, gdy używasz `data_alerts_loop.ps1`.
- **Orca bot (CLI):** jak wcześniej — Task Scheduler lub NSSM/Shawl z `orca_bot_run_supervised.ps1`.

### Docker

Build the CLI image per [`Docker/cli.Dockerfile`](../Docker/cli.Dockerfile). Use [`Docker/orca-bot.compose.example.yml`](../Docker/orca-bot.compose.example.yml) as a starting point: **`restart: unless-stopped`** gives container-level continuity. Mount keypair **read-only** and pass `KEYPAIR_PATH` / `SOLANA_RPC_URL` via env or env_file — never commit keys.

## 2. Logs and retention

- **Application:** `RUST_LOG` (e.g. `info`, `debug` for short investigations only).
- **systemd:** `journalctl` + optional `StandardOutput=append:` in a custom override.
- **Windows script:** `-LogDir` appends timestamped `.log` per process start; add **log rotation** (scheduled task deleting files older than N days) or ship files to your log stack.
- **Ledgers:** `--il-ledger-path` / `--position-fee-ledger-path` — back up the directory with the same cadence as wallet ops (see [`BOT_OPERATIONS_MODEL_2026-03-23.md`](BOT_OPERATIONS_MODEL_2026-03-23.md)).

## 3. Alerts (pragmatic)

The codebase does not ship a hosted alerting product. Hook **your** channel to:

- **Supervisor failure:** `systemd` `OnFailure=`, Task Scheduler “if task fails, send email” (limited), or external **health ping** (cron hits a URL you control; if the bot host stops pinging, page).
- **Log-based:** ship `journald` or file logs to Loki/CloudWatch/ELK; alert on `ERROR` rate or “restarted N times in 5m”.
- **RPC:** see `CLMM_RPC_DENYLIST`, fallbacks, and paid-endpoint hard-disable in [`ENGINEERING_NOTES.md`](ENGINEERING_NOTES.md) (keywords: `rpc`, `failover`).

Align severities with [`BOT_OPERATIONS_MODEL_2026-03-23.md`](BOT_OPERATIONS_MODEL_2026-03-23.md).

### Snapshot / data quality → Slack (recommended)

| Skrypt | Co robi | Jak często |
| ------ | ------- | ---------- |
| [`tools/snapshot_health_alert.ps1`](../tools/snapshot_health_alert.ps1) | Kolektory 10m/5m, wiek OK runów, ERROR w logach | co **~10 min** (domyślnie w pętli) |
| [`tools/quick_verify_alert.ps1`](../tools/quick_verify_alert.ps1) | `snapshot-readiness` + `data-health-check` + (opcj.) decode audit | co **~60 min** |
| [`tools/data_alerts_loop.ps1`](../tools/data_alerts_loop.ps1) | **Oba powyższe w jednym procesie** — bez Task Scheduler | parametry `-SnapshotIntervalSeconds` / `-QuickVerifyIntervalSeconds` |

Oba używają [`tools/notify_slack_webhook.ps1`](../tools/notify_slack_webhook.ps1) i **throttle**, żeby nie zalewać kanału. Szczegóły: [`doc/SCRIPTS_CATALOG.md`](SCRIPTS_CATALOG.md).

**Linux / serwer (zamiast Task Scheduler):** `systemd` **timer** wywołujący `pwsh -File tools/snapshot_health_alert.ps1` oraz drugi timer dla `quick_verify_alert.ps1` — analog dwóch harmonogramów, ale w jednym stylu z `journalctl`. Albo jedna usługa `Restart=always` z [`deploy/systemd/clmm-lp-data-alerts-loop.service.example`](../deploy/systemd/clmm-lp-data-alerts-loop.service.example) (wymaga **PowerShell Core** `pwsh`). **Docker:** kontener z `restart: unless-stopped` i pętlą `sleep` + wywołanie skryptów (albo cron w obrazie).

### Cron (Linux) — dwa wpisy w `crontab`

**Windows:** masz już **Windows PowerShell** (`powershell.exe`) — **nie musisz** instalować **PowerShell Core** (`pwsh`). Wszystkie `tools/*.ps1` uruchamiasz normalnie z `powershell.exe` (Task Scheduler, Shawl, ręcznie). **`pwsh` instalujesz tylko na Linuxie** (albo macOS), bo tam nie ma wbudowanego `powershell.exe` — bez `pwsh` nie odpalisz tych skryptów.

Skrypty są **PowerShell** (`.ps1`) — na **Linuxie** potrzebujesz **PowerShell Core**: `pwsh` (np. `sudo apt install powershell` według [dokumentacji Microsoft](https://learn.microsoft.com/powershell/scripting/install/installing-powershell-on-linux)).

**`quick_verify_alert` → `quick_verify_data`** woła `cargo run … clmm-lp-cli` — użytkownik crona musi mieć **`cargo` i zbudowany projekt** (albo najpierw `cargo build --release` i wtedy zmiana skryptu na exe — obecnie domyślnie jest `cargo run`). W `crontab` ustaw **`PATH`** tak, by zawierał `~/.cargo/bin`.

```cron
# crontab -e (jako ten sam user, który ma repo i .env)
SHELL=/bin/bash
PATH=/home/TWOJ_USER/.cargo/bin:/usr/local/bin:/usr/bin:/bin

# co 10 minut — tylko JSONL kolektorów / logi (bez pełnego cargo w ścieżce wystarczy sam pwsh)
*/10 * * * * cd /ścieżka/do/CLMM-Liquidity-Provider && /usr/bin/pwsh -NoProfile -File ./tools/snapshot_health_alert.ps1 >>/tmp/clmm-snapshot-health-cron.log 2>&1

# co 60 min (start o pełnej godzinie); do quick_verify musi działać cargo z tego PATH
0 * * * * cd /ścieżka/do/CLMM-Liquidity-Provider && /usr/bin/pwsh -NoProfile -File ./tools/quick_verify_alert.ps1 >>/tmp/clmm-quick-verify-cron.log 2>&1
```

- Zamień **`/ścieżka/do/CLMM-Liquidity-Provider`** na absolutną ścieżkę do **rootu repo** (tam gdzie leży `.env` i `Cargo.toml`).
- **`SLACK_WEBHOOK_URL`:** skrypty wczytują ją z **`.env` w rootcie** (`notify_slack_webhook.ps1`) — ustaw **`chmod 600 .env`** dla użytkownika crona.
- Logi: dowolna ścieżka zamiast `/tmp/…` (np. pod `data/snapshot_logs/` w repo).

**Jeden wpis zamiast dwóch:** uruchom [`tools/data_alerts_loop.ps1`](../tools/data_alerts_loop.ps1) z **jednego** cronu co boot (`@reboot`) z długim `sleep` nie — lepiej **systemd** z `Restart=always` (patrz przykład w `deploy/systemd/`) albo `@reboot` + `pwsh -File data_alerts_loop.ps1` (proces zostaje w tle).

### Task Scheduler (Windows) — dwa zadania

1. **Task Scheduler** → **Create Task…** (nie „Basic”, żeby mieć pełną kontrolę).
2. **General:** zaznacz *Run whether user is logged on or not* (jeśli ma działać 24/7); *Configure for:* Twój Windows.
3. **Zadanie A — co 10 min:**
   - **Triggers:** New → *On a schedule* → *Daily* → Advanced: **Repeat task every: 10 minutes**, **for a duration of:** *Indefinitely*.
   - **Actions:** New → **Program/script:** `powershell.exe`  
     **Add arguments:** `-NoProfile -ExecutionPolicy Bypass -File "F:\pełna\ścieżka\CLMM-Liquidity-Provider\tools\snapshot_health_alert.ps1"`  
     **Start in:** `F:\pełna\ścieżka\CLMM-Liquidity-Provider`
4. **Zadanie B — co 60 min:** analogicznie, interwał **1 hour** (lub *One time* + repeat every **1 hour**), plik `quick_verify_alert.ps1`, ten sam **Start in**.
5. **Uprawnienia:** konto z dostępem do repo, `.env` i do uruchomienia `cargo` (dla quick verify).

**Jedno zadanie zamiast dwóch:** **Shawl / NSSM** + [`tools/data_alerts_loop.ps1`](../tools/data_alerts_loop.ps1) — patrz sekcja *Windows* powyżej.

### Co ustawić u siebie (checklist — alerty danych → Slack)

| # | Co | Uwaga |
|---|-----|--------|
| 1 | **`.env`** w rootcie repo z `SLACK_WEBHOOK_URL=…` | Nie commituj; `chmod 600` na Linuxie. |
| 2 | **Kolektory snapshotów** działają (`snapshot-run-curated-all` w pętli) | Inaczej `snapshot_health_alert` będzie stale NOT OK. |
| 3 | **Rust / `cargo`** | Wymagane dla **`quick_verify_alert`** (wewnątrz `cargo run` CLI). |
| 4 | **Shell:** **`pwsh` tylko na Linuxie** (cron); na **Windows** wystarczy **`powershell.exe`** (wbudowany) | Ścieżki bezwzględne w cronie / „Start in”. |
| 5 | **Test ręczny** z rootu repo | Windows: `powershell` → `.\tools\…`; Linux: `pwsh` → `./tools/…` |
| 6 | (Opcja) **Slack** — kanał, webhook, filtry powiadomień | Po stronie UI Slacka. |

### Slack (Incoming Webhook)

1. W Slacku: **utwórz kanał** pod alerty (np. `#lp-bot-alerts`). Adres `app.slack.com/client/…/D…` z **prefiksem `D`** to zwykle **DM**, nie kanał — webhook i tak konfiguruje się na **wybrany kanał** przy tworzeniu integracji.
2. [api.slack.com/apps](https://api.slack.com/apps) → **Create New App** → **From scratch** → włącz **Incoming Webhooks** → **Add New Webhook to Workspace** → wybierz kanał → skopiuj URL (`https://hooks.slack.com/services/…`).
3. **Gdzie trzymać URL:** najwygodniej **jedna linia w pliku `.env` w katalogu głównym repo** (`SLACK_WEBHOOK_URL=https://hooks.slack.com/...`). Plik `.env` jest w **`.gitignore`** — nie trafia do gita. Skopiuj z [`.env.example`](../.env.example) (tam jest szablon bez prawdziwego sekretu). Alternatywy: zmienna użytkownika w Windows, lub osobny plik poza repo / w systemd `EnvironmentFile=` z **chmod 600** — **nigdy** commituj prawdziwego URL-a.
4. **Test z `.env`:** z katalogu głównego repo wystarczy  
   `.\tools\notify_slack_webhook.ps1 -Text 'test'`  
   — skrypt sam wczyta `SLACK_WEBHOOK_URL` z pliku `.env` w rootcie, jeśli zmienna środowiskowa nie jest ustawiona. (Możesz też najpierw ustawić `$env:SLACK_WEBHOOK_URL` albo użyć `-WebhookUrl`.)
5. **Linux / CI:** `export SLACK_WEBHOOK_URL='https://hooks.slack.com/...'` (lub `source` pliku z sekretem), potem  
   `curl -sS -X POST -H 'Content-type: application/json' --data "{\"text\":\"test\"}" "$SLACK_WEBHOOK_URL"`
6. **Plan Free:** m.in. limit **liczby aplikacji** w workspace — jedna mini-apka na webhook zwykle mieści się w budżecie; szczegóły: [slack.com/pricing/free](https://slack.com/pricing/free).
7. **Digest z API (bez PowerShell):** po uruchomieniu `clmm-lp-api` z ustawionym `SLACK_WEBHOOK_URL` możesz wysłać skrót ostatnich wierszy ledgera: `POST /api/v1/bot-activity/slack-summary` z ciałem `{"limit":40}` (albo z dashboardu **Bot activity** → przycisk). API musi widzieć ten sam katalog `data/` co bot (np. uruchomienie z rootu repo lub override `CLMM_POSITION_LIFECYCLE_LEDGER_PATH`).

Opcjonalnie wywołuj `notify_slack_webhook.ps1` z **osobnego** zadania cron / `OnFailure=` (np. gdy `systemctl is-active` zwróci błąd), albo dopisz wywołanie do własnego wrappera po przekroczeniu `MaxRestarts` w `orca_bot_run_supervised.ps1`.

## 4. RPC and cluster

- Set **`SOLANA_RPC_URL`** and optional **`SOLANA_RPC_FALLBACK_URLS`** (same cluster only).
- **`CLMM_EXPECTED_CLUSTER=mainnet-beta`** when you intend mainnet (fail-fast guard).
- Re-measure latency after provider changes; treat public RPC as **best-effort** (rate limits).

## 5. Keys and config refresh

- **Keys:** prefer **`KEYPAIR_PATH`** / **`--keypair`** to a file with restrictive ACLs; avoid logging paths or env values. In Docker, mount a read-only secret file.
- **DecisionConfig refresh without restart:** use **`clmm-lp-api`** + `POST …/apply-optimize-result` or `StrategyService` periodic optimize (see [`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md)). **CLI-only** deployments typically restart the process after updating `--optimize-result-json` or wrap reload in your own supervisor (stop → swap file → start).

## 6. Checklist (minimal)

- [ ] Supervisor installed (systemd / Task Scheduler / Docker restart).
- [ ] `EnvironmentFile` or equivalent: RPC, cluster, key path, `RUST_LOG`.
- [ ] Log destination + rotation or journal limits.
- [ ] Backup path for ledger JSONL (if used).
- [ ] One alert rule: “process down” or “too many restarts”.
- [ ] Runbook link for operator: [`ORCA_RUNBOOK.md`](ORCA_RUNBOOK.md).
