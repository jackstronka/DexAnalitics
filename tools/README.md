# `tools/` — PowerShell helpers

**Spis, keywords i priorytety alertów (snapshot → Slack):** [`doc/SCRIPTS_CATALOG.md`](../doc/SCRIPTS_CATALOG.md)

**Zalecane (bez Task Scheduler):** jeden proces — pętla z interwałami:

```powershell
.\tools\data_alerts_loop.ps1
```

Uruchom **raz** po starcie systemu przez **Shawl** lub **NSSM** (patrz [`doc/OPERATIONAL_CONTINUITY.md`](../doc/OPERATIONAL_CONTINUITY.md)). Log: `data/snapshot_logs/data-alerts-loop.log`.

**Pojedyncze strzały (ręcznie / cron / inny harmonogram):**

```powershell
.\tools\snapshot_health_alert.ps1
.\tools\quick_verify_alert.ps1
```

Wymagane `.env` z `SLACK_WEBHOOK_URL`. Katalog skryptów: [`doc/SCRIPTS_CATALOG.md`](../doc/SCRIPTS_CATALOG.md).

**Linux:** [`deploy/systemd/clmm-lp-data-alerts-loop.service.example`](../deploy/systemd/clmm-lp-data-alerts-loop.service.example) wymaga zainstalowanego **`pwsh`**. **Windows:** użyj wbudowanego **`powershell.exe`** — instalacja `pwsh` nie jest potrzebna.
