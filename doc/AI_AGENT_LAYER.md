# AI / agent layer w Bociarz LP

**Cel:** jedno miejsce, które tłumaczy, co w tym repozytorium znaczy „agent” / „AI” — bez mylenia z ogólnym buzzwordem z rynku DeFi.

**Powiązane:** [`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md) (apply optimize, polityki), [`TODO_CHART_AGENT_LAYER.md`](TODO_CHART_AGENT_LAYER.md) (plan warstwy wykresu / wieloagentowej, **jeszcze nie wdrożone**), [`AI_STREAM_AGENT.md`](AI_STREAM_AGENT.md) (**osobny** produkt: narrator streamu / studio — nie ten dokument).

---

## 1. Trzy różne rzeczy, które ludzie nazywają „agentem”

| Powierzchnia | Co to jest | Gdzie w kodzie / API |
| ------------ | ---------- | -------------------- |
| **A. Envelope optymalizacji (`AgentDecision`)** | Zewnętrzny byt (cron, skrypt, ewentualnie LLM) wysyła **ustrukturyzowaną zgodę** na zastosowanie wyniku siatki `backtest-optimize` albo **odmowę** (`approved: false`). To **nie** jest model trenowany w repo — to **kontrakt JSON**. | Typ: [`crates/domain/src/agent_decision.rs`](../crates/domain/src/agent_decision.rs). Endpoint: `POST /api/v1/strategies/{id}/apply-optimize-result` (ciało: surowy `OptimizeResultFile` **albo** `{ "decision": AgentDecision, "baseline_optimize_result": ... }`). Walidacja: `clmm-lp-execution::agent_decision`. |
| **B. Position Agent (nadzór nad pozycją)** | **UI + API** pod jedną otwartą pozycję: czat, skan, snapshot „superwizora”, opcjonalnie odpowiedź z **zewnętrznego LLM** (OpenAI-compatible) lub fallback heurystyczny. Służy do **wyjaśniania / sugestii** przy nadzorze operatora, nie zastępuje samodzielnie executora bez dalszej integracji. | Trasy: `/api/v1/positions/{address}/agent/*`, `/api/v1/agent/worker/*`. Stan lokalny: m.in. `data/agent/position_agent_state.json`, `data/agent/position_agent_events.jsonl`. Tryb LLM: env `CLMM_AGENT_LLM_*` (patrz `ENGINEERING_NOTES`, wpisy *position agent*). |
| **C. Silnik decyzji live (`DecisionEngine`)** | **Reguły deterministyczne** w pętli bota: hold vs rebalance wg `StrategyMode` (progi, okresowość, IL, świeca, Bollinger itd.). To **nie** jest „AI” w sensie ML/LLM — to warstwa **policy** wbudowana w execution. | [`crates/execution/src/strategy/decision.rs`](../crates/execution/src/strategy/decision.rs), użycie w [`executor.rs`](../crates/execution/src/strategy/executor.rs). |

**Świadomie brakuje** dziś jednego centralnego **orkiestratora**, który łączyłby wszystkie sygnały (zdrowie danych, wiele pooli, reżim rynku) w jedną spójną „meta-decyzję” — patrz sekcja 4.

---

## 2. Ścieżka „agent zatwierdza siatkę” (A)

- Wynik `clmm-lp-cli backtest-optimize --optimize-result-json …` może trafić do API **bezpośrednio** albo przez envelope z `AgentDecision`.
- Strategia ma **`optimize_apply_policy`**, żeby **nie było wyścigu** między subprocessem okresowej optymalizacji a zewnętrznym HTTP (`optimization_busy` per strategia w trybie `combined`).
- Opcyjny **cap ryzyka** przy envelope: `parameters.agent_max_width_pct_delta` ogranicza `|Δ winner.width_pct|` względem baseline z envelope.
- **Rekomendacja bez egzekucji:** `approved: false` → API zwraca sukces, executor **bez zmian** (audyt / podgląd).

Pełniejsze tabele i przepływ: [`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md) § *Periodic `backtest-optimize`…*.

---

## 3. Position Agent i log decyzji (B + audyt)

- **Czat / skan / supervisor** są **local-first**; LLM jest **opcjonalny** i wyłączalny — zgodnie z priorytetem taniego on-chain + lokalnych plików jako domyślnego źródła prawdy dla strategii.
- **Rejestr decyzji (dowolnego źródła):** append-only `data/agent/agent_decisions.jsonl` z API:
  - `GET /api/v1/data/agent/decisions` (filtry m.in. `strategy_id`, `source`, zakres czasu),
  - `POST /api/v1/data/agent/decisions` (dopisanie wiersza).

To jest pod **orchestrację zewnętrzną** i przyszłe agregaty — same endpointy **nie** uruchamiają transakcji.

---

## 4. Roadmap: „prawdziwsza” warstwa decyzyjna

- **Rolling memory (pamięć operacyjna poza LLM, restart-safe):** [`AGENT_ROLLING_MEMORY_PLAN.md`](AGENT_ROLLING_MEMORY_PLAN.md) — fazy O1 → M1 → M2 → M3; kontrakt `data/agent/memory/*`.
- **Orkiestrator LP, shadow/symulacje równoległe, fazy analiza → wykonanie:** [`DECISION_LAYER.md`](DECISION_LAYER.md) — główny dokument wizji i kontraktu (nie mylić z samym Position Agentem).
- **Plan produktowy** warstwy wykresu / wielu agentów / konsensusu: [`TODO_CHART_AGENT_LAYER.md`](TODO_CHART_AGENT_LAYER.md) (profil `agent_layer_profile`, reguły, ewaluacja).
- **Event bus / miks async:** [`ASYNC_COMMUNICATION_LAYER.md`](ASYNC_COMMUNICATION_LAYER.md).
- **Normatywne zachowanie** (w tym apply + busy lock): dopracowanie w [`FUNCTIONAL_SPECIFICATION.md`](FUNCTIONAL_SPECIFICATION.md) §7 oraz odsyłacze do tego pliku.

---

## 5. Słownik (keywords pod grep / AI)

`AgentDecision`, `apply-optimize-result`, `optimize_apply_policy`, `optimization_busy`, `agent_max_width_pct_delta`, Position Agent, `CLMM_AGENT_LLM_MODE`, `agent_decisions.jsonl`, `DecisionEngine`, `StrategyMode`, orchestration, chart agent (planned)

---

## Document status

| Field | Value |
| ----- | ----- |
| Role | **Canonical** overview of agent/AI surfaces in this repo |
| Created | 2026-05-13 |
