# Most: Dip Agent ↔ Dex Analytics (Bociarz LP)

**Kierunek (Faza A+):** kanon = to repozytorium Dex Analytics.  
Źródło historyczne: `C:\Users\jacks\Documents\jupiter-dip-agent\` (szkic 2026-11-04).

**Zasada:** wnosimy docs, config, propose-only Trigger — **nie** drugi live bot Node i **nie** port całego `orca-bot-run`.

---

## Podział odpowiedzialności

| Noga | System | Co robi |
|------|--------|---------|
| **LP** | Orca bot + `DecisionEngine` (to repo) | Zakresy, rebalance, IL, fee LP |
| **Dip / cash** | warstwa `doc/dip-agent/` + przyszły Trigger client | Buy the Dip, news, selection |
| **Wspólne** | Jupiter Price API, `agent_decisions.jsonl`, dashboard | Jedna prawda o cenie i audycie |

Nie mieszaj live Orca rebalance z live Jupiter create w jednym procesie bez osobnych `mode` / signerów.

---

## Faza A — docs (DONE w merge)

- Katalog [`doc/dip-agent/`](README.md)
- Config [`config/dip/`](../../config/dip/)
- Seed kontekstu [`config/dip/context/`](../../config/dip/context/)
- Indeksy: `AI_AGENT_LAYER`, `DECISION_LAYER` §1b, `DATA_CATALOG`, `doc/README`, `AGENTS.md`

---

## Faza B — Price + Trigger read (DONE 2026-11-09)

Reuse Jupiter Price + strefy z [`config/dip/zones.json`](../../config/dip/zones.json).

| Surface | Path |
|---------|------|
| Trigger client | [`crates/data/src/providers/jupiter_trigger.rs`](../../crates/data/src/providers/jupiter_trigger.rs) |
| Zones | [`crates/data/src/dip/`](../../crates/data/src/dip/) |
| API | `GET /api/v1/dip/snapshot` |
| CLI | `cargo run -p clmm-lp-cli -- dip-snapshot [--skip-orders]` |

Auth challenge/verify/refresh w kliencie; **create/cancel NO-GO** (Faza E).

**Nie przenoś:** osobnego Node `priceClient` / daemona jako produkcji.

---

## Faza C — Propose-only Decision envelope

Nie rozszerzaj [`AgentDecision`](../../crates/domain/src/agent_decision.rs) (`deny_unknown_fields`) — to tylko apply-optimize.

Dip zapisuje do `data/agent/agent_decisions.jsonl` wiersz z osobnym `decision.kind`, np.:

```json
{
  "action": "buy_dip|sell|hold|adjust_lp|skip",
  "mode": "propose-only|paper|live",
  "requires_user_confirm": true,
  "rationale": "...",
  "refs": ["config/dip/context/news-digest.md", "config/dip/context/assets/SOL/news.md"]
}
```

---

## Faza D — UI

Panel open Jupiter limits + alert gdy LP OOR (patrz [`LP-HYGIENE.md`](LP-HYGIENE.md)).  
Markdown (`candidates.md`, journals) wystarczy do czasu stabilnego Trigger history.

**Plan szczegółowy (funkcje Jupiter UI + bot settings + agent + UX 3Commas DCA / drabinka + reuse Dex):**  
[`IMPLEMENTATION_PLAN_DIP_AGENT_UI.md`](../IMPLEMENTATION_PLAN_DIP_AGENT_UI.md) §1b · §3b.

---

## Faza E — Live (osobny GO)

Trigger create/cancel za flagą + osobny signer. Nadal: nie jeden shared keypair z Orca bez policy.

Katalog funkcji do pokrycia (T4–T16, S1–S2, DoD): [`API-CAPABILITY-MATRIX.md`](API-CAPABILITY-MATRIX.md).

---

## Jawne NO-GO

- Port całego `orca-bot-run` do Node daemona dip  
- Współdzielony keypair file między dwoma botami bez policy  
- Auto-live Jupiter create z tego samego procesu co Orca rebalance  
- Założenie że backtest fee CLMM = realny PnL limitu Jupiter  
- Memecoin universe z Discover bez scoringu  
- Mylenie z [`ROADMAP_JUPITER_MULTI_VENUE_LP.md`](../ROADMAP_JUPITER_MULTI_VENUE_LP.md)

---

## Ścieżki w Dex Analytics

| Temat | Path |
|-------|------|
| Agent map | [`AGENTS.md`](../../AGENTS.md) |
| AI layer | [`doc/AI_AGENT_LAYER.md`](../AI_AGENT_LAYER.md) |
| Decision layer | [`doc/DECISION_LAYER.md`](../DECISION_LAYER.md) |
| Orca runbook | [`doc/ORCA_RUNBOOK.md`](../ORCA_RUNBOOK.md) |
| DecisionEngine | `crates/execution/src/strategy/decision.rs` |
| Jupiter Price | `crates/data/src/providers/jupiter.rs` |
