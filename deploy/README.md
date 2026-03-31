# Deploy templates

- **`systemd/clmm-lp-orca-bot.service.example`** — Linux service unit for `clmm-lp-cli orca-bot-run`. See [`doc/OPERATIONAL_CONTINUITY.md`](../doc/OPERATIONAL_CONTINUITY.md).
- **`systemd/clmm-lp-data-alerts-loop.service.example`** — `pwsh` + `tools/data_alerts_loop.ps1` (Slack alerty snapshotów / jakości danych).

Docker stack templates live under [`../Docker/`](../Docker/).
