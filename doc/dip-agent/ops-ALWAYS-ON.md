# Always-on — Dip monitoring w Dex Analytics

**Faza A:** nie uruchamiamy osobnego Node daemona z `jupiter-dip-agent`.  
Canonical always-on = istniejące procesy Dex Analytics (API, CLI ingest, Task Scheduler / supervised tools już w repo).

## Model v1 (zalecany)

| Warstwa | Gdzie |
|---------|--------|
| Ceny | `GET /api/v1/prices/jupiter` / `JupiterProvider` |
| LP live | `orca-bot-run` / API strategies (jak dziś) |
| Dip kontekst | pliki w `config/dip/context/` (+ lokalnie `data/dip/context/`) |
| Decision (człowiek / Cursor) | checklist [`DECISION-FRAMEWORK.md`](DECISION-FRAMEWORK.md) w trybie propose-only |

Opcjonalny historyczny Node daemon (`Documents/jupiter-dip-agent` → `npm run daemon`) = **legacy only** do researchu — nie produkcja obok API.

## Setup (Windows) — Dex

1. PostgreSQL + API na porcie lokalnym (zwykle **8081** w tym workspace; sprawdź health).  
2. Env: `JUPITER_API_KEY` w `.env` (jak dla wyceny pozycji).  
3. Skopiuj seed kontekstu przy pierwszym setupie:

```bat
mkdir data\dip\context 2>nul
xcopy /E /I /Y config\dip\context data\dip\context
```

4. Autostart API / botów: istniejące runbooki (`STARTUP.md`, `doc/PRODUCTION_FAST_PATH.md`, supervised scripts w `tools/`) — **nie** `start-daemon.cmd` z dip-agenta.

## Recovery

- RPC / Price fail → nie składaj live Trigger; Decision = `hold` / `skip`.  
- Restart laptopa → API + bot wg Dex; odśwież `portfolio-state.md` ręcznie jeśli limity Jupiter zmieniły status w UI.  
- Faza B+: heartbeat Trigger history w API zamiast Node pid file.

## NO-GO

- Drugi always-on Node obok `clmm-lp-api` podpisujący tx.  
- Współdzielony keypair Orca + Jupiter bez osobnych `mode`.
