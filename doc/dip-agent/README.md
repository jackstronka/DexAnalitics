# Dip Agent (Jupiter Trigger) — warstwa w Dex Analytics

**Status:** Faza A (docs + config + seed kontekstu). Canonical home: ten katalog w **Bociarz LP / Dex Analytics**.  
Dawny szkic Node: `C:\Users\jacks\Documents\jupiter-dip-agent` — nie rozwijaj tam nowych features.

## Dwie nogi jednego produktu

| Noga | Venue | Rola |
|------|-------|------|
| **LP** | Orca Whirlpool (live bot już w tym repo) | Zakresy, fee, IL, rebalance |
| **Dip** | Jupiter Trigger / Limit (Buy the Dip) | Spot limity na przecenę; v1 = **propose-only** |

Wspólne: Jupiter Price (`GET /prices/jupiter`), audyt `data/agent/agent_decisions.jsonl`, dashboard.  
**Nie** mylić z [`ROADMAP_JUPITER_MULTI_VENUE_LP.md`](../ROADMAP_JUPITER_MULTI_VENUE_LP.md) (routing swapów → fee LP).

## Pliki

| Plik | Treść |
|------|--------|
| [`DECISION-FRAMEWORK.md`](DECISION-FRAMEWORK.md) | Checklist Analysis → Decision |
| [`dip-rotation.md`](dip-rotation.md) | Strategia rotacji dipów / strefy |
| [`jupiter-API.md`](jupiter-API.md) | Allowlista endpointów Trigger / Swap / Price |
| [`API-CAPABILITY-MATRIX.md`](API-CAPABILITY-MATRIX.md) | Katalog funkcji P/T/S/D — pokrycie API vs status implementacji |
| [`IMPLEMENTATION_PLAN_DIP_AGENT_UI.md`](../IMPLEMENTATION_PLAN_DIP_AGENT_UI.md) | Plan UI: Jupiter/Dip, bot settings, agent; **§1b UX 3Commas** (drabinka + Summary) |
| [`jupiter-ALLOWED.md`](jupiter-ALLOWED.md) | Dozwolone produkty Jupitera |
| [`LP-HYGIENE.md`](LP-HYGIENE.md) | Orca range out-of-range → `adjust_lp` |
| [`BRIDGE.md`](BRIDGE.md) | Most dip ↔ LP (fazy B–E) |
| [`ops-ALWAYS-ON.md`](ops-ALWAYS-ON.md) | Always-on: **reuse API/CLI Dex**, nie Node daemon |
| [`intel/`](intel/) | News sources, scoring aktywów, must-follow 20 |

Konfiguracja: [`config/dip/`](../../config/dip/)  
Seed kontekstu (tracked): [`config/dip/context/`](../../config/dip/context/)  
Lokalna kopia robocza (gitignore `/data/`): `data/dip/context/`

## Mode

Domyślnie `config/dip/runtime.yaml` → `mode: propose-only`.  
Live Trigger create/cancel = Faza E (osobny GO + osobna policy signera). Nie łączyć live Orca rebalance i live Jupiter create w jednym procesie bez osobnych `mode`.

## Fazy (skrót)

- **A:** docs + config + seed — DONE  
- **B:** Trigger read + zones — DONE (`GET /dip/snapshot`, `dip-snapshot` CLI)  
- **C:** propose `buy_dip|hold|…` → `agent_decisions.jsonl` — DONE (`POST /dip/propose`, CLI `dip-propose`, `kind=dip_propose`)  
- **D:** UI limity + OOR LP — **U1**+**U1b**+**U2** DONE (`/dip` + Settings runtime); U3+ open  
- **E:** live create/cancel za flagą  

Szczegóły: [`BRIDGE.md`](BRIDGE.md). Powierzchnia AI: [`AI_AGENT_LAYER.md`](../AI_AGENT_LAYER.md) § D.

### Operator (Faza B)

```bash
cargo run -p clmm-lp-cli -- dip-snapshot --skip-orders
set CLMM_JUPITER_TRIGGER_JWT=...
cargo run -p clmm-lp-cli -- dip-snapshot
curl -s "http://127.0.0.1:8081/api/v1/dip/snapshot"
```

## Testy i kryteria regresji

**Obszar:** Decision layer / agent.  
**Pozycja w mapie:** Faza A+B merge.

| Co chronimy | Typ | Fixture | Stan |
|-------------|-----|---------|------|
| Dip docs reachable z indeksów | manual checklist | ścieżki w repo | ✅ |
| Trigger filter + HTTP parse | unit + httpmock | `jupiter_trigger::tests` | ✅ |
| Zone A/B/C classification | unit | `dip::zones::tests` | ✅ |

**Test exit gate:** `cargo test -p clmm-lp-data jupiter_trigger` + `dip::zones`; `cargo check -p clmm-lp-api -p clmm-lp-cli`.  
**Golden delta:** nie dotyczy.  
**Poza automatem:** JWT challenge/verify w portfelu.
