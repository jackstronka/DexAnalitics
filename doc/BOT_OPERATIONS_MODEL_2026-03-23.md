# Bot Operations Model (2026-03-23)

## Purpose

This document defines how the bot is operated day-to-day, not only how it is implemented.

It covers:
- operating modes,
- operator procedures,
- alert handling and escalation,
- manual override and emergency actions,
- post-session review and audit trail.

The model is Orca-first and aligned with hybrid architecture decisions.

## Design Inputs Borrowed From Reviewed Models

### From Hummingbot (controllers/executors)
- Clear split between decision policy and execution.
- Explicit lifecycle of actions (create/stop/monitor).
- Config-driven operation and multi-step status checks.

### From Orca Whirlpools repositioning examples
- Practical Solana transaction handling concerns:
  - retries,
  - priority fee policy,
  - simulation-aware send path,
  - edge cases like SOL/wSOL and transaction size.
- Operational emphasis on readable CLI output and failure transparency.

### From Gelato/G-UNI style automation
- Checker-first pattern: verify preconditions before execution.
- Guardrails as operational safety, not strategy replacement.
- Time-based/condition-based cadence for supervision.

## Operating Modes

### Mode 1: Dry-Run
Purpose:
- verify strategy logic, trigger behavior, and lifecycle events without on-chain risk.

Allowed actions:
- decision loop active,
- simulated transaction path,
- no on-chain state changes.

Exit criteria:
- no critical errors in N consecutive cycles,
- strategy triggers observed as expected,
- logs/lifecycle artifacts complete.

### Mode 2: Limited Live (Single-Market)
Purpose:
- real execution with small test capital and strict supervision.

Scope constraints:
- one pool only (`HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF` by current preflight),
- small capital test profile,
- operator on-call during session.

Exit criteria:
- stable execution across planned window,
- no unresolved critical alerts,
- net behavior consistent with strategy semantics.

### Mode 3: Standard Live
Purpose:
- normal operation after successful limited-live validation.

Scope:
- still Orca-first in Stage 1,
- scale only after explicit approval.

## Daily Operator Procedure

### Pre-Start Checklist
- [ ] Confirm RPC health and latency baseline.
- [ ] Confirm env variables loaded (`PRIVATE_KEY`, RPC, mode flags).
- [ ] Confirm selected strategy config and pool address.
- [ ] Confirm operating mode (`dry-run` or `limited-live`).
- [ ] Confirm alert channel is active.

### Start Procedure
- [ ] Start services (API/CLI executor) using selected mode.
- [ ] Verify first monitor cycle completed.
- [ ] Verify lifecycle events are being recorded.
- [ ] Verify no immediate configuration errors.

### In-Session Supervision
- [ ] Review trigger decisions vs market moves.
- [ ] Confirm no repeated failure loop (same error across cycles).
- [ ] Confirm rebalance reason tags are correct (`Periodic`, `RangeExit`, `RetouchShift`, etc.).
- [ ] Confirm fee collection attempts happen before range migration.

### End-of-Session
- [ ] Stop strategy gracefully.
- [ ] Export/retain logs and lifecycle summary.
- [ ] Mark incidents and action items in worklog.

## Alert Model and Escalation

### Severity Levels

Info:
- normal lifecycle transitions, successful actions.

Warning:
- temporary RPC issues, transient simulation/send failures, skipped action due to guardrails.

Critical:
- repeated tx failures past retry budget,
- inconsistent strategy state,
- unexpected position state drift,
- key/wallet/config integrity issue.

### Response Targets

Critical:
- acknowledge in <= 5 minutes,
- decide continue/stop in <= 15 minutes.

Warning:
- triage in <= 30 minutes.

Info:
- review in routine post-session checks.

## Guardrails Policy (Minimal, Strategy-Respecting)

Principle:
- strategy remains primary driver for price reaction,
- guardrails only prevent pathological operational behavior.

Enabled minimal guardrails:
- retry budget for tx send failures,
- short anti-loop protection for repeated identical failures,
- emergency stop path.

Not used as hard blockers by default:
- aggressive expected-benefit gating that suppresses valid strategy triggers.

## Manual Override and Emergency Controls

### Manual Override Triggers
- repeated critical failures,
- suspected wrong config in live mode,
- unexpected token balance behavior,
- suspicious divergence from expected strategy behavior.

### Manual Actions
- soft stop: pause decision/execution loop safely,
- hard stop: disable strategy and stop tx attempts,
- emergency exit (if implemented for stage): close position path with operator confirmation.

### Recovery Procedure
- identify root cause from logs + lifecycle,
- apply fix,
- restart in dry-run first if cause was unclear,
- only then return to limited-live.

## Operational Metrics to Track

Core:
- decision cycles count,
- rebalance count by reason,
- fee collection attempts/success rate,
- tx success/failure rate,
- retry count distribution,
- mean time to recovery for incidents.

Quality:
- strategy-semantic parity checks (expected vs observed triggers),
- number of manual interventions,
- incident recurrence.

## Runbook Integration

This operations model complements:
- `doc/ORCA_RUNBOOK.md`
- `doc/BOT_HYBRID_DEFINITION_OF_READY_2026-03-23.md`
- `doc/BOT_WORKLOG_2026-03-23.md`

If procedures conflict, this file defines operator behavior and escalation precedence.

## Cyclic optimization + IL ledger (2026-03)

### Optimize result JSON (`backtest-optimize`)

- CLI flag: `--optimize-result-json <PATH>` writes schema version `1` (see `clmm_lp_domain::optimize_result::OptimizeResultFile`).
- Live API applies this file via `clmm_lp_execution::optimize_profile::decision_config_from_optimize_result` → updates `StrategyExecutor`’s `DecisionConfig` without restart.

### Strategy parameters (API)

- `optimize_on_start`: run the CLI subprocess once before the executor loop; requires `optimize_command` + `optimize_result_json_path`.
- `optimize_interval_secs`: background interval to re-run the same command and re-apply the JSON (skips if a run is already in progress).
- `optimize_command`: argv vector, first element = program path; if `--optimize-result-json` is missing, the API appends it using `optimize_result_json_path`.
- `il_ledger_path`: optional JSONL file; lifecycle rows for `position_opened`, `rebalance`, `position_closed` are appended for offline IL / PnL reconstruction.

### Price convention for IL logs

- **`price_ab`**: **token B per token A** (same side as pool/oracle “price” used in execution logs). Use this consistently when joining ledger lines to simulator/backtest prices.
- Amounts are **raw on-chain units** (lamports/smallest units) when present; `None` means not yet wired from RPC.

### Offline IL pipeline (sketch)

1. Load JSONL from `il_ledger_path` (filter `event == "rebalance"`).
2. Pair `price_ab_before` / `price_ab_after` with `amount_*_before` / `amount_*_after` at each step.
3. Compare holding LP vs HODL of the same initial bundle using your IL definition (e.g. vs external USD oracle or vs `price_ab` path only).

---

## Ciągłość: logi ruchów bota, IL i decyzje (PL)

Ten podrozdział opisuje **jeden spójny łańcuch operacyjny**: co bot robi na łańcuchu → co trafia do logów → jak z tego liczyć IL i audytować ścieżkę → jak to się łączy z **podejmowaniem decyzji** (w tym z cykliczną optymalizacją). Chodzi o **ciągłość w czasie**: żadna decyzja nie „wisi w powietrzu” bez śladu w ledgerze, a zmiana strategii nie kasuje historii.

### 1. Oś czasu zdarzeń (nieprzerwana narracja)

- Każdy istotny ruch powinien dać się ułożyć w **jedną oś czasu** dla danej pozycji / strategii: `position_opened` → zero lub więcej `rebalance` → ewentualnie `position_closed`.
- Wiersze JSONL (gdy ustawiono `il_ledger_path`) mają `schema_version`, znacznik czasu (`timestamp`), `position`, `pool` oraz pola pod IL — dzięki temu można **odtworzyć stan** „tuż przed” i „tuż po” każdym kroku.
- **IL w CLMM jest ścieżkozależny**: ta sama para tokenów przy tym samym końcowym kursie może mieć inny wynik, jeśli inna była kolejność rebalansów i kosztów. Dlatego ciągłość = **zachowanie kolejności i kompletności** logów, nie tylko „snapshot końcowy”.

### 2. Co logujemy pod kątem IL

- **Ceny**: konwencja **`price_ab`** = *ile tokena B na 1 token A* (spójnie z logiką executora / pool); używaj tej samej definicji w skryptach analitycznych co w polach `price_ab_before` / `price_ab_after` (i `price_ab` przy otwarciu/zamknięciu).
- **Ilości**: surowe jednostki on-chain (`amount_*`), gdy są dostępne; jeśli pole jest puste, w danym buildzie traktuj je jako „jeszcze nie podpięte z RPC”, ale **reszta łańcucha czasu** nadal jest wartościowa (ticki, powód rebalance, `il_at_rebalance` w evencie lifecycle).
- **Powód / kontekst**: typ zdarzenia i przyczyna rebalance (np. okresowy, wyjście z zakresu, profil z optymalizacji) pozwalają powiązać **ruch** z **aktualną polityką** `DecisionConfig`.

### 3. Jak z logów liczyć IL (ciągłość między krokami)

1. Wczytaj JSONL posortowany po czasie.
2. Dla każdego `rebalance` weź parę **stan przed** / **stan po** (tokeny + `price_ab` jeśli jest).
3. Zdefiniuj benchmark (np. HODL tej samej paczki tokenów co na wejściu do LP, zaktualizowany według ustalonych reguł przy rebalance) i porównuj **krok po kroku** z wartością pozycji LP (w USD lub w jednostce referencyjnej).
4. Sumuj opłaty transakcyjne / slippage według tego, co macie w modelu (ledger może mieć tylko część — uzupełniaj z kosztów on-chain lub z osobnej tabeli kosztów).

Wynik: **ciągła krzywa** „wartość LP vs benchmark” i przyrost IL między kolejnymi ruchami — to jest podstawa pod raporty post-mortem i pod walidację, czy bot faktycznie realizuje założoną strategię.

### 4. Jak decyzje łączą się z tą ciągłością

- **Pętla live** (`StrategyExecutor`): w każdym cyklu decyzje (hold / rebalance / …) biorą się z **bieżącego** `DecisionConfig` (tryb strategii, szerokość zakresu, progi, itd.) oraz ze stanu pozycji i poola z monitora.
- **Cykliczna optymalizacja** (`backtest-optimize` → JSON → `set_decision_config`): co jakiś czas **odświeża politykę** (np. inny tryb lub parametry), ale **nie zastępuje ledgera** — nowy profil działa **od następnych** ewaluacji / rebalansów; historia w JSONL zostaje.
- **Ciągłość decyzyjna** oznacza więc:
  - *ex ante*: decyzja wynika z aktualnej polityki + stanu rynku;
  - *ex post*: ten sam moment można znaleźć w logu jako zdarzenie z cenami/ilościami i zrekonstruować IL na tej ścieżce;
  - *audyt*: można sprawdzić, czy po zmianie profilu z optymalizacji pojawiają się oczekiwane wzorce w logach (częstotliwość rebalansów, powody, szerokości).

### 5. Operacyjnie: co pilnować

- **Jedna ścieżka pliku** `il_ledger_path` per strategia (lub jasny podział), żeby nie rozbić osi czasu.
- **Backup / rotacja** plików JSONL tak jak logów aplikacji — to źródło prawdy pod sporami i analizą IL.
- Po wdrożeniu pełnego odczytu tokenów z RPC: uzupełniać pola `amount_*` w `RebalanceData`, żeby IL offline i online były **zbieżne**.

*(Powyższe uzupełnia anglojęzyczne sekcje „Cyclic optimization + IL ledger” i „Price convention”; mają być czytane razem.)*

## Stage 1 Operational Constraints

- Orca-only live runtime.
- Single-market deployment for initial operations.
- Small-capital limited-live mode before any scale-up.
- Any scale-up requires explicit post-mortem approval.

