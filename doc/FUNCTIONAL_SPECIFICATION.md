# Functional specification (normative)

**Purpose:** one shared place where **how features are supposed to behave** is described in detail. Implementation should converge here; when code intentionally diverges, this document must be updated **or** the divergence must be recorded with rationale and a link to `doc/BUGS.md` / `doc/ENGINEERING_NOTES.md`.

**Audience:** operators, reviewers, and AI agents implementing or changing behavior.

**Not in scope (use other docs):**

| Topic | Where |
| ----- | ----- |
| Crate layout, pipelines, CLI names | [`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md) |
| End-to-end data → bot → UI narrative | [`PROJECT_END_TO_END.md`](PROJECT_END_TO_END.md) |
| Append-only change log per merge | [`ENGINEERING_NOTES.md`](ENGINEERING_NOTES.md) |
| Incident registry and fixes | [`BUGS.md`](BUGS.md) |
| API method map / service split | [`ORCA_API_SERVICE_CONTRACT.md`](ORCA_API_SERVICE_CONTRACT.md) |
| Run steps, env, operational checklists | runbooks in [`README.md`](README.md) *Runbooks and operations* |
| Decision / orchestration layer (vision, registry, audit; **implementation phases 0–6+**) | [`DECISION_LAYER.md`](DECISION_LAYER.md); [`IMPLEMENTATION_PLAN_DECISION_LAYER.md`](IMPLEMENTATION_PLAN_DECISION_LAYER.md) — **§8** below: normative *stub* produktu; kolejność wdrożeń i fazy w planie implementacji do czasu kodu |
| Wallet GL — plan faz (journal → read model → reconcile); szczegóły poza §5.1 | [`WALLET_GL.md`](WALLET_GL.md) |

---

## How to extend this document

1. Add a **top-level section** per subsystem (or per user-visible capability).
2. For each capability use a **fixed subsection pattern** (below) so diffs stay readable and grep works.
3. Every section ends with **`keywords:`** (comma-separated) for search — same convention as `ENGINEERING_NOTES.md`.
4. When behavior changes in code, either **update this file in the same change** or add a TODO block with owner/date — avoid silent drift.

### Subsection pattern (copy per feature)

```markdown
### <Feature name>

**Goal:** one sentence — what user/operator gets.

**Inputs:** APIs, env vars, strategy parameters, on-chain reads.

**Outputs:** lifecycle events, HTTP responses, side effects.

**Happy path:** numbered steps.

**Edge cases:** explicit list (empty wallet, OOR, multi-strategy, RPC stale, …).

**Invariants:** what must never happen (safety / accounting).

**Observability:** ledger event names, log fields, UI surfaces.

**Code pointers (non-exhaustive):** `path/to/file.rs` — keep short; prefer `keywords:` for discovery.

**keywords:** …
```

---

## Table of contents (stubs — fill in)

Sections below are **placeholders**. Replace stubs with normative text as you refine each area.

### 1. Position lifecycle (open, close, manual vs strategy)

**Goal:** *TODO*

**keywords:** positions, open, close, registry, lifecycle

---

### 2. Rebalance and rotation (close → wallet → open, session id)

**Goal:** Rotate an out-of-range (or otherwise rebalance-triggered) position into a new range while keeping **capital continuity**: the new position should be opened with a USD budget derived from what the close returned **to the wallet** (principal + fees, i.e. total returned value), optionally mixed/swapped to match deposit requirements, and correlated end-to-end via a `rebalance_session_id`.

**Inputs:**

- **On-chain reads**: position liquidity + pool state needed to compute close amounts and to quote open caps for a target USD.
- **Strategy decision**: planned tick range (`new_tick_lower/upper`) and whether a swap-mix step is allowed/required.
- **Session correlation**: `rebalance_session_id` must be stable across all rows of the rotation.

**Outputs:**

- **Lifecycle ledger rows** that let an operator reconstruct the chain: at minimum `bot_close_position` then `bot_open_position` (plus swap-mix rows when applicable), all sharing the same `rebalance_session_id`.
- **Position registry** transition: old position marked closed (kind = `rotation`/`strategy`) and the new position opened (and linked to the correct strategy if automation is enabled).

**Happy path:**

1. **Decide** that a rebalance/rotation is required and compute the **planned new tick range** (strategy-specific).
2. **Close** the old position and record:
   - `close_amount_{a,b}_raw` (token amounts returned from LP close; in practice close typically collects fees too),
   - `lp_collected_token_{a,b}_raw` (fees collected on close, when available),
   - context: `old_tick_*` and `planned_new_tick_*`.
3. **Derive the open budget in USD** (normative policy):
   - The close can return **only token A**, **only token B**, or **both** in any proportions.
   - Fees may be **zero** (e.g. position was open very briefly). This is expected and must not block continuity.
   - Define returned token amounts (raw):
     - `returned_a_raw = close_amount_a_raw + lp_collected_token_a_raw` (treat missing `lp_collected*` as 0)
     - `returned_b_raw = close_amount_b_raw + lp_collected_token_b_raw` (treat missing `lp_collected*` as 0)
   - Compute `returned_usd_from_close` by valuing `returned_{a,b}_raw` using the same synthetic USD price model used for deposit quoting (stable-aware; see section 5/6 for price conventions).
   - Set `target_usd = returned_usd_from_close` (minus a small margin, e.g. 0.5%, for rounding/dust).
4. **Swap-mix (optional)**: If current wallet balances do not cover the deposit quote split for `target_usd`, execute in-pool swaps (ExactIn rounds) until they do (within a small epsilon).
5. **Open** the new position in the planned tick range using caps derived from a **deposit-budget quote** for `target_usd`.
6. **Link to strategy**: If this open is part of a running strategy, ensure the new PDA is linked to that strategy and automation continues on the new PDA (or is explicitly disabled for this PDA).

**Edge cases:**

- **Insufficient operational native SOL**: close may succeed but open/swap-mix can fail; the system may attempt an operational top-up (unwrap WSOL / swap stable→WSOL then unwrap) before retrying open, per existing runbooks/guards.
- **Tick drift during delayed open**: if open is delayed (pending-open), the planned range may no longer include spot; recovery may replan/widen/recenter according to policy (documented under pending-open / recovery; see section 3 and runbooks).
- **Multi-strategy / shared wallet contention**: multiple strategies rotating positions share one wallet; the budget derivation must be per-session and must not assume other sessions won’t consume wallet liquidity in-between.
- **Wallet read inconsistency**: after a successful close, the wallet read-model can temporarily lag or be incomplete (RPC/indexing delays). This must be treated as a *read quality issue* first (retry/refresh + diagnostics), not as “we truly have less capital”.

**Invariants:**

- A single `rebalance_session_id` must not produce two successful opens (avoid orphan PDAs).
- The system must not intentionally “downsize” a reopen below `target_usd` due to stale/partial wallet reads — szczegółowa procedura: **§2.2** (retry odświeżeń → pending-open → alert po max attempts).
- Fees are included in the close-return budget by default; if/when we introduce a “principal-only reopen sizing” mode, it must be explicitly documented and observable (see section 6).

**Observability:**

- Ledger events (typical): `bot_close_position`, `bot_swap_mix_round`, `bot_swap_exact_in_*`, `bot_open_position`, `bot_pending_open_*`.
- `bot_open_position.details` should include: `open_target_usd`, `open_prev_end_value_usd` (USD value used as reopen budget), and `open_quote_*` fields to audit sizing and caps.

**Code pointers (non-exhaustive):**

- `crates/execution/src/strategy/rebalance.rs`
- `crates/protocols/src/orca/deposit_quote.rs`
- `crates/protocols/src/ledger/tx_lifecycle.rs`

**keywords:** rebalance, rotation, close_position, open_position, rebalance_session_id, target_usd, prev_end_value_usd, swap_mix, deposit_quote, lifecycle-ledger, pending-open

#### 2.1 Shared wallet contention (policy **3A**) + periodic reopen retries

**Problem:** Wiele strategii / wielu workerów dzieli jeden portfel. Środki zwrócone z close sesji A mogą chwilowo „znikać” z perspektywy A, jeśli sesja B zużyje saldo na swap/open — to nie musi być błąd odczytu, ale **konkurencja o ten sam wallet**.

**Policy 3A (no hard reservation):**

- Nie wprowadzamy na ten moment twardej rezerwacji tokenów per `rebalance_session_id`.
- Jeśli po close nie da się od razu otworzyć na `target_usd` (swap-mix / quote / brak sald): **nie downsizujemy cicho** — wpis trafia do **pending-open recovery** z telemetrią (`attempts`, `last_error`, `stuck_reason`, …).

**Trade-off vs twarde rezerwacje (świadoma decyzja produktowa):**

- W wielu systemach finansowych i w kolejkach zasobów typowym wzorcem jest **lock / rezerwacja** środków na czas transakcji, żeby równoległa sesja nie zużyła salda — **3A tego nie robi**.
- **Uzasadnienie (faza obecna):** prostszy runtime bez osobnego mechanizmu escrow on-chain ani „skrytek” rezerw SPL/SOL per `rebalance_session_id`; jeden wspólny portfel, **pending-open + ponawianie + alerty** oraz **§2.2** (odświeżenia `W`, brak cichego downsizingu) jako jawna mitigacja race i słabych odczytów.
- **Koszt:** realna **konkurencja o wallet** — inna strategia, worker lub operator może chwilowo „zabrać” tokeny z perspektywy danej sesji; reopen bywa **opóźniony**, niekoniecznie błędny. Jeśli kiedyś będzie wymagana **twarda** izolacja kapitału między sesjami, to **osobna** funkcja (rezerwacja / kolejka priorytetowa / sub-konta) — poza zakresem normy **3A**.

**Periodic retries (normative intent):**

- Pending-open **musi być ponawiany cyklicznie**, bo kapitał może „wrócić” do portfela później (inna sesja skończyła swap, RPC odświeżył salda, operator dołożył SOL, itd.).
- W obecnym modelu executor: przed każdą turą `evaluate_all` wykonywany jest pass `process_pending_open_recoveries` (czyli częstotliwość prób jest powiązana z **cadence pętli strategii**, a nie z osobnym timerem — dopóki nie wprowadzimy dedykowanego interwału).
- Limity operacyjne (implementacja): `CLMM_PENDING_OPEN_MAX_ATTEMPTS` (domyślnie 100), `CLMM_PENDING_OPEN_ALERT_ATTEMPTS` (domyślnie 10) — po przekroczeniu progu alert „Pending Open Stuck”; po max attempts kolejka porzuca item z logiem.
- Pass recovery **nie działa** w `dry_run` lub gdy `auto_execute` jest wyłączone (brak mutacji on-chain).

**keywords:** pending-open, wallet-contention, policy-3A, trade-off, no-reservation, periodic-retry, CLMM_PENDING_OPEN_MAX_ATTEMPTS, CLMM_PENDING_OPEN_ALERT_ATTEMPTS, evaluate_all

#### 2.2 Punkt 3 — **„Stary odczyt portfela”** vs **realny brak środków** (bez cichego downsizingu)

**TL;DR (operator):** Po close bot liczy **ile USD chce ponownie zdeponować (`T`)** i patrzy **ile USD „widzi” na portfelu (`W`)**. Jeśli **`W < T`**, to **nie znaczy od razu**, że pieniędzy brak — często to **zły/opóźniony odczyt** albo **inna strategia zużyła saldo**. Wtedy: **kilka razy odśwież salda** → jeśli dalej źle, **nie otwieraj mniejszej pozycji po cichu**, tylko **pending-open + ponawianie**; po wielu próbach **alert**. Mały brak od zaokrągleń przy swap-mix nadal OK.

**Cel:** po `close` mamy z ledgera (§6.1) **docelowy budżet** `T = returned_usd_from_close`. Portfel pokazuje **dostępne** `W` (w tym samym modelu cen + SOL-first). Jeśli `W < T`, nie wolno od razu uznać, że „nie ma pieniędzy” — może to być **RPC/cache**, **opóźnienie indeksu**, albo **konkurencja innej sesji** (§2.1).

**Definicje:**

- **`T`**: `returned_usd_from_close` z §6.1 (wartość zwrócona z close — w executorze: rawy zwrócone × aktualne `p_a/p_b` w momencie próby, §2.5).
- **`W`**: szacunek **spendable** notional portfela po close w USD (SPL + zasady SOL-first / operational SOL), **tymi samymi** `p_a/p_b` co przy depozycie.

**Normatywna procedura (po close, przed finalnym open / w swap-mix):**

1. **Porównanie:** jeśli `W >= T * (1 - ε_wallet)` → uznajemy portfel za wystarczający na start swap-mix / open (`ε_wallet` mały, np. **0.5%**, zgodny z istniejącym marginesem 0.995 / dust).
2. **Jeśli `W` wygląda na za małe:** wykonaj **`N` odświeżeń** sald (np. **`N = 3`**, krótki odstęp typu 250–500 ms między próbami — wartość do skonfigurowania w kodzie) i **ponownie** policz `W`.
3. **Po `N` próbach nadal `W < T`:** **nie** zmniejszaj `target_usd` do `W` cicho.
   - **Ścieżka domyślna:** zapisz / utrzymaj **pending-open** (§2.1) z jasnym `last_error` + `stuck_reason` (np. `wallet_below_target_after_refresh`, `wallet_contention_suspected`) i **ponawiaj** zgodnie z §2.1.
   - **Dopiero po przekroczeniu** `CLMM_PENDING_OPEN_MAX_ATTEMPTS`: przerwij automatycznie z alertem; operator decyduje (dopłata, ręczny open, dismiss).

**Kiedy można uznać „realny deficit” (bez dowodu z łańcucha):**

- Gdy po odświeżeniach `W` jest stabilnie poniżej progu **i** pending-open przez wiele cykli nie rośnie **oraz** operator potwierdza brak innych botów / ręcznych transferów — wtedy klasyfikacja **`wallet_deficit_persistent`** jest uzasadniona (heurystyka operacyjna, nie twarde twierdzenie on-chain).

**Ciasny przypadek (już istniejący w swap-mix):**

- Nadal dopuszczalny jest **mały** deficyt w USD względem quote po swapach w granicy **`swap_mix_deficit_usd_epsilon_for_target`** (zaokrąglenia / koszt tx) — to nie jest „downsizing całego `T`”, tylko tolerancja numeryczna.

**Observability (minimum):**

- Log / ledger: `T`, każde odświeżone `W`, liczba prób `N`, wynik (pending vs continue).
- Implementacja: wiersze diagnostyczne `bot_reopen_wallet_below_target` przy nieudanym porównaniu w danej próbie; komunikat błędu z prefiksem `wallet_below_target_after_refresh` gdy po **`N`** odczytach nadal `W < T*(1-ε)`. Regulacja: **`CLMM_REOPEN_WALLET_REFRESH_ATTEMPTS`** (domyślnie 3), **`CLMM_REOPEN_WALLET_REFRESH_GAP_MS`** (domyślnie 350), **`CLMM_REOPEN_WALLET_NOTIONAL_EPSILON`** (domyślnie 0.005 = 0.5%).

**keywords:** stale-wallet-read, wallet-notional, returned_usd_from_close, pending-open, no-silent-downsize, stuck_reason, swap_mix_deficit_usd_epsilon, CLMM_REOPEN_WALLET_REFRESH_ATTEMPTS

#### 2.3 Punkt 4 — **Cadence** ponawiania pending-open (jak często bot próbuje znowu)

**TL;DR:** Pending-open jest **próbowany na początku każdej tury `evaluate_all`** danego executora strategii. Opcjonalnie: **`CLMM_PENDING_OPEN_MIN_INTERVAL_SECS`** — jeśli od `last_attempt_at` minęło mniej niż X sekund, **pomija się** próbę **bez** zwiększania `attempts` (RPC saver). **Osobny worker** nadal tylko jeśli kiedyś okaże się potrzebny.

**Faza 1 (norma obecna — bez zmian architektury):**

- `process_pending_open_recoveries` uruchamia się **przed** pętlą po pozycjach w `evaluate_all` (implementacja: `crates/execution/src/strategy/executor.rs`).
- **Częstotliwość prób** dla danego hosta zależy więc od:
  - interwału / obciążenia pętli strategii (np. wiele strategii = wiele executorów może w praktyce **częściej** odpytywać kolejkę),
  - tego czy `auto_execute` jest włączone i czy jest skonfigurowany `CLMM_PENDING_OPEN_RECOVERY_PATH`.
- **Operator:** jeśli próby są zbyt częste (koszt RPC), pierwsza gałąź regulacji to **cadence samej strategii** / mniej równoległych executorów — zanim wprowadzimy osobny scheduler.

**Faza 2 (opcjonalna — włączana env):**

- **`CLMM_PENDING_OPEN_MIN_INTERVAL_SECS`** (1..=86400): zaimplementowane w `process_pending_open_recoveries` — jeśli od `last_attempt_at` minęło mniej niż X sekund, **pomiń** tę turę: **bez** `attempts += 1`, **bez** aktualizacji `last_attempt_at`, item zostaje w kolejce (`crates/execution/src/strategy/pending_open.rs`).
- **Osobny worker / timer** poza pętlą strategii: tylko jeśli wystąpi realny case „**nic** nie woła `evaluate_all`, a pending-open musi żyć” (dziś typowo strategia jest źródłem ticków).

**Powiązanie z punktem 3 (§2.2):**

- Krótkie retry odświeżeń sald (`N`×) dotyczy **pojedynczej próby** open/swap-mix; **cadence z §2.3** dotyczy **kolejnych** prób recovery w czasie.

**Operator — `CLMM_PENDING_OPEN_MIN_INTERVAL_SECS`:**

- **Cel:** ograniczyć **zbyt częste** wywołania `recover_open` / odczyty RPC dla **tego samego** wpisu pending-open, gdy `evaluate_all` (lub wiele executorów) odpytuje kolejkę bardzo gęsto — **bez** usuwania wpisu i **bez** „palenia” licznika `attempts` przy samej pauzie czasowej.
- **Kiedy włączyć:** widzisz w logach powtarzające się próby recovery co kilkadziesiąt sekund / minutę przy tym samym `closed_position_nft`, a przyczyna (np. brak SOL, tick) **nie zmienia się** między turami — wtedy ustaw np. **60–300** (sekundy) i obserwuj RPC oraz `attempts` w `pending-open-recovery.json`.
- **Jak liczy się odstęp:** od znacznika **`last_attempt_at`** zapisnego przy **ostatniej turze, która faktycznie weszła** w ścieżkę `attempts += 1` → `recover_open` (pominięcie przez min interval **nie** przesuwa `last_attempt_at` ani `attempts`).
- **Czego to nie zastępuje:** `CLMM_PENDING_OPEN_MAX_ATTEMPTS` / `CLMM_PENDING_OPEN_ALERT_ATTEMPTS` — to nadal limity **liczby** prób z efektem; min interval tylko **rozrzuca w czasie** próby on-chain.
- **Brak zmiennej lub wartość poza 1..86400:** zachowanie jak bez fazy 2 (każda tura `evaluate_all` może próbować recovery, o ile claim i reszta warunków pozwala).

**keywords:** pending-open, cadence, evaluate_all, process_pending_open_recoveries, CLMM_PENDING_OPEN_RECOVERY_PATH, CLMM_PENDING_OPEN_MIN_INTERVAL_SECS, rate-limit, optional-min-interval

#### 2.4 Punkt 5 — **Inna strategia** / wspólny portfel: **kto** otwiera i **skąd** bierze budżet

**TL;DR:** Budżet **`T`** (USD z close) bierzemy **z ledgera po `rebalance_session_id` + zamkniętym PDA** (§6.1) — **nie** z „pamięci strategii A vs B”. Kolejka **`pending-open`** jest **wspólna dla procesu** (plik z `CLMM_PENDING_OPEN_RECOVERY_PATH`); którykolwiek executor strategii, który pierwszy przejmie claim, spróbuje `recover_open` **swoim** `RebalanceExecutor` (ten sam portfel w typowym deployu). Jeśli kiedyś ma być **twarde** „tylko strategia X kończy reopen strategii Y” — potrzebne osobne pole / polityka (faza 2).

**Identyfikacja sesji (minimum, dziś w modelu danych):**

- **`rebalance_session_id`**: spina `bot_close_position` → ewentualne swapy → `bot_open_position` w lifecycle.
- **`closed_position_nft`**: adres zamkniętej pozycji (PDA), pod którym szukamy wiersza close w ledgerze.
- **`pool`**: adres puli.
- Rekord **`PendingOpenItem`** trzyma m.in. `intended_tick_lower/upper`, `reason`, `planned_at_utc`, `planned_price_ab` — **nie** przechowuje obecnie jawnego `strategy_id` (Rust: `crates/execution/src/strategy/pending_open.rs`).

**Norma kapitałowa (niezależnie od strategii „wykonującej” open):**

- **`T`** i `returned_*_raw` wyliczamy z **tego samego** `bot_close_position`, który należy do tej sesji i zamkniętego PDA (§6.1). Strategia, która akurat woła recovery, **nie wolno** nadpisywać `T` własnym „domyślnym sizingiem” — chyba że wprowadzimy explicite tryb / pole (faza 2).

**Współdzielenie kolejki pending-open (typowy mainnet single-wallet):**

- Wiele strategii → wiele pętli `evaluate_all` → wiele wywołań `process_pending_open_recoveries` na **tym samym** pliku kolejki.
- **Claim** (`sid:…` / `pool+closed`) zapobiega równoległemu podwójnemu przetwarzaniu tego samego itemu w jednym momencie; nadal obowiązuje **§2.1 / §2.3** (contention, cadence).
- **Wiele procesów OS (równoległe boty / porównywanie strategii):** claim jest **per proces** — **dwa** procesy z **tym samym** `CLMM_PENDING_OPEN_RECOVERY_PATH` **nie** widzą swoich claimów i mogą **skorumpować** plik JSON (read-modify-write). Norma operacyjna: **osobny plik kolejki (i sensownie osobne ścieżki ledgera / danych)** na instancję bota **albo** jeden koordynator; nie udawać współdzielonej kolejki między VM bez mechanizmu blokady pliku / bazy.

**„Inna strategia ma otworzyć po close” (produkt):**

- **Faza 1 (dziś):** otwarcie następuje przez **executora**, który **realnie** wykonuje recovery (zwykle ta sama maszyna bota z tym samym keypair). Zmiana „kto decyduje o tickach” to głównie **link strategii ↔ nowy PDA** po sukcesie (API / `position_addresses`), a nie osobny mechanizm w `PendingOpenItem`.
- **Faza 2 (jeśli wymagane):** dodać np. `origin_strategy_id` / `target_strategy_id` + reguły dispatch (kto może przejąć item) — **poza** obecnym kontraktem JSON; wymaga implementacji i migracji pliku.

**keywords:** cross-strategy, pending-open, rebalance_session_id, shared-wallet, strategy-executor, PendingOpenItem, handoff

#### 2.5 Punkt 6 — **Waloryzacja USD przy opóźnionym reopen** (pending-open: świeże ceny / przeliczanie)

**TL;DR:** **Źródło prawdy kapitału z close = `returned_{a,b}_raw`** (§6.1) — te wartości **nie zmieniają się** między kolejnymi próbami recovery. **Etykieta USD (`T`) i porównanie z portfelem (`W`)** oraz **quote open / swap-mix** używają **bieżących** `p_a/p_b` (i spójnego modelu cen jak w §2.2) **w momencie danej próby** — czyli **świeże przeliczenie**, nie zamrożony snapshot USD z chwili zamknięcia.

**Norma:**

1. **`returned_*_raw`**: tylko z ledgera sesji (`bot_close_position` + §6.1); **nie** przeliczamy ich ponownie przy retry (chyba że korekta błędu zapisu — osobna procedura poza happy path).
2. **`returned_usd_from_close` / `T` (notacja USD):** przy **każdej** próbie open / swap-mix / porównania `W` vs `T` wyliczamy na nowo jako wartość tych samych raw przy **aktualnych** cenach syntetycznych (`p_a`, `p_b`) używanych wtedy do depozytu i wallet-notional — zgodnie z §2.2 i modelem z sekcji 5/6.
3. **Wykonanie on-chain:** kapsyłki depozytu, ticki, stan puli — zawsze z **świeżego** odczytu w momencie próby (por. §2 „Tick drift during delayed open”).
4. **Opcjonalna telemetria (faza późniejsza):** jeśli operator potrzebuje audytu, można logować **dwa** USD: np. `T_usd_at_close_prices` (diagnostyka) oraz `T_usd_at_attempt` (decyzje) — normatywna ścieżka decyzyjna to **`T` przeliczone świeżo** jak w pkt 2.

**Invariants:**

- Nie wolno **zmniejszać** `target_usd` wyłącznie dlatego, że „stary label USD” z close spadł przy nowej cenie — zmiana rynku zmienia **etykietę**, nie **inwentarz tokenów** z zamknięcia.
- Nadal obowiązuje §2.2: przy `W < T` po odświeżeniach — pending-open, nie cichy downsizing.

**keywords:** pending-open, USD-valuation, fresh-prices, repricing, returned_raw, T-vs-W, swap-mix, deposit-quote

---

### 3. Reopen preflight and guardrails (`no_close_unless_reopen_feasible`, deposit quote)

**Goal:** Prevent “close leaving the operator with no position” by checking that a reopen/open is **feasible** before closing, using a deposit budget quote and clear diagnostic rows when feasibility fails.

**Inputs:**

- Planned new range (`tick_lower/upper`) for the next position.
- Current pool state (`tick_current`, `sqrt_price`, `tick_spacing`).
- Wallet balances available for the open (including SOL-first considerations on WSOL leg).
- USD target budget derived from expected post-close spendable (see below).

**Outputs:**

- If preflight passes: rotation continues to close.
- If preflight fails: rotation is **blocked before close** when the guardrail is enabled, and a diagnostic lifecycle row is emitted.

**Definition: “reopen feasible” (normative):**

Reopen is considered feasible iff `quote_deposit_budget_in_range(...)` succeeds for the planned range with:

- spot tick inside the range: `tick_lower <= tick_current < tick_upper`
- `target_usd` is finite and > 0
- synthetic USD prices are finite and > 0 for both legs

**Preflight USD budget (normative):**

Preflight happens **before close**, so SPL balances may be zero while all value sits in the position. Therefore:

- `target_usd` for preflight must be derived from an estimate of **post-close spendable USD**, not from pre-close SPL balances alone.
- Use: `target_usd = min(prev_end_value_usd, 0.995 * (wallet_notional_before_close + prev_end_value_usd))`
  - `prev_end_value_usd` is the USD value of what close is expected to return to the wallet (principal + fees, valued at the same synthetic prices).
  - `wallet_notional_before_close` is the current wallet notional in USD (SPL + SOL-first effective balance where applicable).
  - The 0.995 factor is a safety margin for rounding/dust.

**Happy path:**

1. Compute `prev_end_value_usd` for the old position at current pool state.
2. Compute `wallet_notional_before_close` (SOL-first when pool has WSOL leg).
3. Compute preflight `target_usd` using the policy above.
4. Run `quote_deposit_budget_in_range` for the planned range.
5. If quote fails and auto-widen is enabled, widen ticks around current spot up to configured max steps; rerun quote each step.
6. If a quote succeeds, lock in `planned_tick_lower/upper` and proceed to close.

**Failure behavior (guardrail ON):**

- Do **not** close.
- Emit `bot_reopen_preflight_failed` with details including: ticks, `wa/wb`, `wallet_notional`, `prev_end_value_usd`, `target_usd`, prices and mode, and a clear note.

**Edge cases:**

- **Empty wallet SPL but value in LP**: preflight must still compute a positive `target_usd` from `prev_end_value_usd` (otherwise it will deadlock rotations).
- **Spot tick outside planned range**: quote will reject; widening/replanning is the allowed remediation when enabled.
- **Multi-session wallet contention**: preflight is best-effort; between preflight and open other sessions can consume wallet liquidity, so post-close swap-mix/open must still have robust failure handling and pending-open recovery.

**Invariants:**

- With guardrail ON, the system must not close a position when preflight quote is impossible for the planned range under the defined budget policy.
- Preflight must never produce `target_usd = 0` when `prev_end_value_usd > 0` (this is a bug class; see `BUGS.md`).

**Observability:**

- Ledger events: `bot_reopen_widen_ticks`, `bot_reopen_preflight_failed` (diagnostic rows).
- Required diagnostic fields: `wallet_notional`, `prev_end_value_usd`, `target_usd`, `pa/pb`, `price_mode`, ticks.

**Code pointers (non-exhaustive):**

- `crates/execution/src/strategy/rebalance.rs`
- `crates/protocols/src/orca/deposit_quote.rs`

**keywords:** reopen_preflight, no_close_unless_reopen_feasible, target_usd, prev_end_value_usd, wallet_notional, widen_ticks, quote_deposit_budget_in_range, bot_reopen_preflight_failed, bot_reopen_widen_ticks

---

### 4. Strategies (executor loop, link position ↔ strategy, dry-run, auto_execute)

**Goal:** *TODO*

**keywords:** strategy_service, StrategyMode, executor, dry_run, auto_execute

---

### 5. Wallet and funding (API signer, SOL-first, effective balances)

**Goal:** Operator i executor widzą **spójne salda** portfela API (natywny SOL, SPL w tym projekcje WSOL) z **read-model cache + publicznego RPC**, z ochroną przed **regresją odczytu** (np. pusta lista SPL przy chwilowo złej odpowiedzi endpointu). Szczegóły zmian i znane regresje: [`ENGINEERING_NOTES.md`](ENGINEERING_NOTES.md) (`keywords:` `effective-balances`, `wallet_effective`), [`BUGS.md`](BUGS.md).

**Normatywnie — źródło prawdy sald w UI (faza bieżąca):**

- Salda w dashboardzie (**w tym ekran otwierania pozycji / swap-before-open**) są wyprowadzane z **`GET /wallets/effective-balances`** (i powiązanych ścieżek odświeżania), **nie** z sumy wierszy **Wallet GL journal** (`§5.1`).
- On-chain przez RPC pozostaje **„bankiem”** do uzgodnienia; **jawne odświeżenie** operatora (`force=true` tam, gdzie przewidziano w API) może **nadpisać** read-model zgodnie z zasadami monotoniczności opisanymi w kodzie i `BUGS.md`.

**keywords:** wallets, effective-balances, wsol, native_sol, operational_sol, wallet_effective_cache

#### 5.1 Wallet GL — append-only journal (shadow, fazy B–E)

**Cel:** **Osobny** rejestr zdarzeń portfelowych zainicjowanych przez API — plik append-only **`data/wallet-ledger-events.jsonl`** (nadpisanie ścieżki: env **`CLMM_WALLET_LEDGER_PATH`**), przygotowanie pod szerszy model **GL / delty na konta** i reconcile z on-chain, **bez** zastąpienia źródła prawdy sald z **`§5`** dopóki nie nastąpi świadoma zmiana produktowa i rozszerzenie normy.

**Plan implementacji i założenia docelowe (jedno miejsce):** [`WALLET_GL.md`](WALLET_GL.md) — m.in. wizja księgowa vs **faza A** (journal), checklisty faz **B–E**.

**Normatywnie — faza A (obowiązuje w kodzie dziś):**

- Dla zaimplementowanych `kind` obowiązuje śledzenie **`pending` → `confirmed` / `failed`** z **`correlation_id`** wiążącym serię, o ile dana ścieżka jest w kodzie podpięta pod `wallet_ledger`.
- Rejestr służy **audytowi** i podłożu pod przyszłe fazy z `WALLET_GL.md`; **nie** blokuje wykonania transakcji ani **nie** zastępuje wyświetlania sald z **`§5`**.
- Podgląd operatora: **`GET /api/v1/wallets/ledger-events`**, UI **`/wallet/ledger`**.

**Poza zakresem fazy A (wdrożenie wg checklisty w `WALLET_GL.md`, bez zmiany normy `§5` bez decyzji):** księgowanie **każdej** transakcji on-chain spoza kontrolowanych ścieżek API; **saldo wyłącznie z sumy journalu**; pełny **chart of accounts** — do dopisania w kodzie i tu, gdy produkt uzna to za normę.

**keywords:** wallet_gl, wallet_ledger, WALLET_GL.md, journal, correlation_id, shadow-ledger, ledger-events

#### 5.2 Konto logiczne sesji (`rebalance_session_id`) — norma docelowa (nie zastępuje §5 ani 3A)

**Cel:** przy starcie cyklu życia pozycji (ręczny open / bot reopen) operator i bot mają **jawny inwentarz tokenów przypisany do sesji**, a nie wyłącznie saldo całego portfela — zgodnie z [`WALLET_GL.md` §2.2](WALLET_GL.md#22-konto-logiczne-per-cykl-życia-pozycji-rebalance_session_id--norma-docelowa).

**Normatywnie (docelowo, po fazach C–D GL):**

- **Identyfikator:** `rebalance_session_id` (rotacja bot) lub `cost_session_id` (ręczny swap+open) — spina lifecycle i wpisy wallet journal.
- **Źródło prawdy inwentarza sesji (tokeny):** lifecycle `bot_close_position` → `returned_*_raw` (**§6.1**); swapy i collect dopisują ruchy; read model GL `SESSION:{id}` per mint (shadow → produkt).
- **Decyzja reopen:** `T` z §6.1; porównanie z **sesją** (preferowane) lub z `W` (**§2.2**) dopóki read model sesji nie jest produkcyjny.
- **Fee w cyklu:** zebrane fee zwiększają inwentarz tej samej sesji (nie „giną” w globalnym portfelu bez śladu).

**Stan dziś:** sesja i `T` są w **lifecycle**; salda UI i executor przy open nadal używają **`§5` (`effective-balances`)** i **policy 3A** (**§2.1** — brak twardej rezerwacji SPL). Konto logiczne SESSION w GL — **do wdrożenia** (patrz checklista w `WALLET_GL.md` §2.2).

**keywords:** rebalance_session_id, cost_session_id, session-account, WALLET_GL, policy-3A, returned_raw, session-notional

### 6. Fees and PnL presentation (collect vs close, principal vs fees in UI)

**Goal:** Provide consistent accounting semantics for “what returned from a close” and how fees are represented in ledger/UI, without blocking execution on perfect fee attribution.

**Normative (for reopen sizing):**

- Default reopen sizing uses **total returned value** from a close: principal + fees, valued in USD with the synthetic price model used for deposit quotes.
- Fees may be **0** (short-lived positions); reopen sizing must still proceed using the returned principal token amounts.

#### 6.1 Punkt 2 — Źródło prawdy: **USD z close** (tokeny A/B + fees)

**Cel:** jednoznacznie powiedzieć, *z jakich pól ledgera* liczymy „ile wróciło na portfel”, żeby **`target_usd` / `open_prev_end_value_usd`** były audytowalne i spójne z sekcją 2.

**Źródło danych (kolejność dopasowania):**

1. Wiersz lifecycle **`bot_close_position`** dla tej samej **`position_pubkey`** (zamykana pozycja) i — jeśli jest — tego samego **`rebalance_session_id`** co bieżąca rotacja.
2. Z `details` (lub równoważnych polach top-level, jeśli tak zapisujemy close) odczytujemy:
   - `close_amount_a_raw`, `close_amount_b_raw` — tokeny zwrócone z **principal** zamknięcia (notional LP po stronie tokenów; mogą być `0` na jednej nodze).
   - `lp_collected_token_a_raw`, `lp_collected_token_b_raw` — **fees zebrane przy close** (mogą być `0` / brak w polach).

**Wzór na „ile tokenów wróciło” (raw):**

- `returned_a_raw = close_amount_a_raw + coalesce(lp_collected_token_a_raw, 0)`
- `returned_b_raw = close_amount_b_raw + coalesce(lp_collected_token_b_raw, 0)`

**USD (dla reopen / swap-mix / open details):**

- `returned_usd_from_close = ui(returned_a_raw)*p_a + ui(returned_b_raw)*p_b` przy **tych samych** syntetycznych cenach `p_a/p_b`, które są używane do `quote_deposit_budget_in_range` w tej samej operacji (spójność tick/decimals).

**Przypadek: fees „zero” albo pomijalnie małe**

- Jeśli `lp_collected_*` wynosi `0` (albo brak w JSON), **traktujemy fee jako 0** dla powyższego wzoru i liczymy wyłącznie z `close_amount_*`. To jest poprawne dla bardzo krótkich pozycji.

**Przypadek degradacji danych: `lp_collected_*` = `0/0`, ale realnie były fees**

- Zdarza się historycznie / przy niepełnym zapisie close (patrz `doc/BUGS.md` w okolicy fees na close).
- **Normatywnie:** reopen **nie może** zostać zablokowany wyłącznie dlatego, że nie da się idealnie rozdzielić fee od principal w ledgerze, o ile `close_amount_*` jest wiarygodne.
- **Fallback (kolejność, best-effort):**
  1. Jeśli na tym samym `signature` co `bot_close_position` mamy `fee_payer_token_deltas` dla mintów **A/B puli** — można użyć ich do **rekonstrukcji** faktycznego przyrostu tokenów na portfelu fee payera (tylko nogi puli, bez mieszania z innymi mintami).
  2. Jeśli nadal niepewne: użyj wyłącznie `close_amount_*` (jak wyżej) i dopisz diagnostykę **`reopen_budget_fee_attribution_degraded`** (lub równoważny wpis) z `signature` + linkiem do Solscan.
  3. UI może pokazywać osobno „fee unknown / degraded”, ale **executor** nadal kontynuuje reopen wg `close_amount_*`.

**Notes / future (optional):**

- A “principal-only sizing” mode is allowed in the future but must:
  - be explicitly configurable,
  - be observable in ledger (fields showing principal-only vs fees-only breakdown),
  - not silently change existing operator expectations.

**keywords:** fees, fee_owed, lineage, position_stream_lineage, Fees-zebrane, close_amount, lp_collected_token, returned_usd, fee_payer_token_deltas, reopen_budget_fee_attribution_degraded

---

### 7. Backtests and optimize apply (policy, busy lock, agent envelope)

**Norm:** Zachowanie endpointu `POST /api/v1/strategies/{id}/apply-optimize-result`, polityki `optimize_apply_policy`, lock `optimization_busy` oraz semantyki `AgentDecision` opisuje [`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md) (tabele i przepływ). **Słownik warstw „agent / AI”** (co jest LLM, co kontraktem JSON, co regułami w execution): [`AI_AGENT_LAYER.md`](AI_AGENT_LAYER.md).

**keywords:** backtest, backtest-optimize, optimize_apply_policy, AgentDecision

---

### 8. Decision layer (orchestrator LP) — draft

**Status:** **Draft** — pełna pętla orkiestratora (ranking wariantów, shadow) nadal w rozwoju; **faza gate:** subcommand `orchestrator-gate` + schemat `gate_health` w polu `decision` (§8 poniżej, [`doc/examples/orchestrator-run-v1.example.json`](examples/orchestrator-run-v1.example.json)). Architektura i rejestr narzędzi: [`DECISION_LAYER.md`](DECISION_LAYER.md).

**Goal:** Warstwa ponad istniejącymi narzędziami zbiera sygnały (dane, metryki ciągu pozycji, jakość danych), uruchamia **te same** ścieżki symulacji/backtestu dla wariantów (w tym shadow / what-if), wydaje **rekomendacje lub decyzje pomocnicze** z pełnym audytem, a w późniejszej fazie może być spięta z wykonaniem (tx, apply optimize). Pierwsza faza produktowa: **analiza i informowanie operatora** — bez samowolnego wykonywania transakcji.

**Mapowanie szczegółowego celu operatora (wolumen, wiele par, jeden ciąg PnL, kapitał):** [`DECISION_LAYER.md`](DECISION_LAYER.md) §1a — tam wskazane, co jest już rozpisane, a co jest szkicem do doprecyzowania przy implementacji. **Rejestr zdolności (narzędzia, cele, NO-GO):** [`DECISION_LAYER.md`](DECISION_LAYER.md) §1b. **Plan implementacji (fazy, kryteria sukcesu, kolejność PR):** [`IMPLEMENTATION_PLAN_DECISION_LAYER.md`](IMPLEMENTATION_PLAN_DECISION_LAYER.md).

**Inputs (docelowo, nie wszystkie dziś spięte):** lokalne snapshoty/swapy i ich jakość; wyniki `backtest` / `backtest-optimize`; metryki ciągu (m.in. IL, fees, koszty) z API/stream; polityki operatora; opcjonalnie envelope [`AgentDecision`](AI_AGENT_LAYER.md) przy apply siatki.

**Outputs:** rekomendacje i/lub strukturalny log decyzji (co, dlaczego, na jakich danych, wersja/kompletność danych); kanał audytu: [`GET`/`POST /api/v1/data/agent/decisions`](AI_AGENT_LAYER.md) zapisuje wiersze do `data/agent/agent_decisions.jsonl` (pole `decision` jest ogólne JSON — dla bramki jakości z CLI używany jest schemat poniżej).

**Happy path (faza 1 — informowanie):**

1. Orkiestrator (proces/moduł) odczytuje stan danych i **bramki jakości** (np. NO-GO przy zbyt słabym decode / starych snapshotach).
2. Przy akceptowalnej jakości uruchamia **wielokrotnie ten sam silnik symulacji** z różnymi parametrami (zakres, rozmiar, ewentualnie pool) na zsynchronizowanym oknie danych — porównanie wariantów **nie** opiera się na ukrytych regułkach rynkowych poza silnikiem (patrz [`DECISION_LAYER.md`](DECISION_LAYER.md) §7).
3. Zapisuje wynik i uzasadnienie w logu decyzji; UI/alerty pokazują operatorowi ranking i **założenia**.

**Edge cases:** nierówna jakość danych między parami (ranking **wstrzymany** lub oznaczony jako niepewny); devnet **nie** jest źródłem prawdy ekonomicznej vs mainnet dla porównań „gdzie zarobić więcej”; wiele legów kapitału na tej samej parze wymaga **jawnej agregacji** w metrykach (patrz [`DECISION_LAYER.md`](DECISION_LAYER.md) §8).

**Invariants (projektowe):**

- Źródło prawdy o rynku dla porównań ekonomicznych: **dane mainnetu** (replay / read-only + lokalne pliki), nie zastąpienie devnetem.
- Shadow / what-if: **wspólna metodologia metryk** z pozycją realną; ten sam silnik backtestu/symulacji co w researchu.
- Faza 1: **brak** automatycznego wysyłania transakcji bez osobnego Go/No-Go i sekcji normatywnej.

**Observability:** każda decyzja musi być **powtarzalna z logu** (wejścia + wersje danych + parametry runów). Dla **fazy 1 (gate)** wiersz JSONL zawiera m.in. `ts_utc`, `source`, opcjonalnie `chain_id`, oraz pole **`decision`** z typem runu opisanym poniżej.

**`decision` — orchestrator run v1 (`kind: "gate_health"`):** jeden przebieg bramki jakości (ta sama logika co `data-health-check` na curated poolach). Pola w `decision`:

| Pole | Typ / wartości | Znaczenie |
| ---- | -------------- | --------- |
| `schema_version` | `1` | Wersja kontraktu tego obiektu |
| `run_id` | string (np. prefiks `gate-` + UUID) | Identyfikator przebiegu |
| `kind` | `"gate_health"` | Rodzaj runu |
| `tools_invoked` | tablica stringów | Narzędzia Rust/CLI użyte w przebiegu (np. `health_check_curated_all_collect`) |
| `data_quality` | obiekt JSON | Skrót wyniku health: `alerts`, `alert_count`, `health_report_path` (ścieżka do `data/reports/health_alerts_*.json` gdy są alerty, inaczej `null`), `summary_ts_utc` |
| `outcome` | `"ok"` \| `"no_go"` | `no_go` gdy `alerts` niepuste |
| `no_go_reason` | string, opcjonalnie | Obecny przy `outcome === "no_go"` |
| `inputs` | obiekt JSON | Progi przebiegu, np. `max_age_minutes`, `min_decode_ok_pct` |
| `inputs_ref` | obiekt JSON | **Faza 0 — audyt plików:** `schema_version` (1), `role` (`curated_dataset_file_stats`), `curated_pool_list_source` (stat `STARTUP.md`: `path`, `mtime_unix_secs`, `size_bytes`), `pool_data_files` — posortowane leksykograficznie po (`protocol`, `pool`); każdy element: `swaps_raw`, `decoded_swaps`, `snapshots_jsonl` z tymi samymi polami statystyk (brak pliku → `mtime_unix_secs` / `size_bytes` = `null`). **Bez** skrótu treści pliku (hash zawartości — opcjonalnie później). |

**`decision` — orchestrator run v1 (`kind: "api_backtests_full"`):** przebieg „API-first” nad macierzą `POST /api/v1/backtests/full` (CLI `orchestrator-backtests-full`). Domyślnie przed FULL: `POST /api/v1/backtests/data-readiness` z `pool_ids` / `snapshot_variants` skopiowanymi z pliku żądania (pomijanie: `--skip-data-readiness`; `--fail-on-data-readiness` domyślnie włączone). Pola (skrót): `schema_version`, `run_id`, `kind`, `tools_invoked` (`POST`/`GET` jak wyżej, w tym readiness), `data_quality` (`job_id`, `job_status`, `metric_rows`, `window_count`, `stderr_preview`, opcjonalnie `gate` jeśli nie `--skip-gate`, `data_readiness` — pełna odpowiedź readiness gdy krok wykonany), `outcome` (`ok` / `no_go` wg statusu joba i flag `--fail-on-job-partial`), `no_go_reason`, `inputs` (= ciało żądania `BacktestFullRequest`), `inputs_ref` (`request_json_path`, `api_base`), opcjonalnie `job` (pełna odpowiedź — tylko z `--decision-include-full-job`). Przykładowe ciało żądania: [`doc/examples/backtest-full-request.min.json`](examples/backtest-full-request.min.json).

**Przykładowy wiersz JSONL:** [`doc/examples/orchestrator-run-v1.example.json`](examples/orchestrator-run-v1.example.json).

**Code pointers:** `crates/cli/src/orchestrator_gate.rs` (`OrchestratorRunV1`, `run_gate_and_log`, `append_agent_decision_row`); subcommand `orchestrator-gate` w `crates/cli/src/main.rs`; `crates/cli/src/orchestrator_api_full.rs` + subcommand `orchestrator-backtests-full`; zbiór danych health: `crates/cli/src/swap_sync.rs` (`health_check_curated_all_collect`).

**keywords:** decision-layer, orchestrator, shadow, counterfactual, simulation, backtest, data-quality, NO-GO, advisory-phase, DECISION_LAYER, orchestrator-gate, gate_health, agent_decisions, inputs_ref, curated_dataset_file_stats, api_backtests_full, orchestrator-backtests-full, backtests/data-readiness, data_readiness

**Audyt stanu implementacji (fakty z repo, bez zgadywania):** [`DECISION_LAYER.md`](DECISION_LAYER.md) §11.

---

## Appendix A — Implementation gap checklist (**A** + audit **C**)

**Audit scope (2026-05-11):** `crates/execution/src/strategy/rebalance.rs` — wszystkie miejsca ustawiające `prev_end_value_usd` / `target_usd` dla reopen i preflight.

**How to use this table:** przy implementacji zmieniaj status `TODO` → `OK` i dopisz commit/PR albo krótką notkę w `ENGINEERING_NOTES.md`.

| # | Wymaganie (spec / sekcje 2, 2.1, 3, 6) | Miejsce w kodzie (orientacyjnie) | Co robi kod dziś | Status |
|---|----------------------------------------|-----------------------------------|-------------------|--------|
| A1 | Budżet reopen w USD = **wartość zwrócona z close** — norma: **sekcja 6.1** (`returned_*_raw` = `close_amount_* + lp_collected_*`) | Po `close_position` przekazywane są **`returned_*_raw` = pre-close `read_close_amounts_best_effort` + `lp_collected_*` z wyniku close** (te same wartości co w wierszu ledgera). | **OK** (faza 1) — principal nadal z odczytu tuż przed close (jak w `details`), fee z quote close; opcjonalnie później: twardy re-read z pliku ledgera po append. |
| A2 | Preflight przed close: budżet USD spójny z powyższym | `no_close_unless_reopen_feasible` | `prev_end_value_usd` z **on-chain `get_position`**: `calculate_token_amounts` (principal w √P) **+ `fees_owed_{a,b}`** w tych samych cenach co quote; fallback na `amount_*_before_calc` przy błędzie odczytu pozycji. | **OK** |
| A3 | Swap-mix po close: `target_usd` vs portfel | `ensure_swap_mix_for_rebalance_open` + `open_new_range_with_wallet_mix` | Wejściowe rawy = **returned** z A1; `target_usd` = `target_usd_for_swap_mix_and_open` (bez clamp do portfela przy `prev_end > 0`). | **OK** (powiązane z A1/A4) |
| A4 | **Nie** ciche obniżenie reopenu przy złym odczycie portfela / contention (3A) | `target_usd_for_reopen_sizing` + `wallet_notional_refresh_until_reopen_target_met` przed swap-mix | Brak clampu do `W`; przed swap-mix: **`W` vs `T*(1-ε)`** z **`CLMM_REOPEN_WALLET_REFRESH_ATTEMPTS`** (domyślnie 3) i **`CLMM_REOPEN_WALLET_REFRESH_GAP_MS`** (domyślnie 350); **`CLMM_REOPEN_WALLET_NOTIONAL_EPSILON`** (domyślnie 0.005). Nadal fail → pending-open. | **OK** |
| A5 | Pending-open recovery: budżet z close | `close_amounts_from_lifecycle_row` + `close_amounts_from_lifecycle_best_effort` | Sumuje **`close_amount_*_raw` + `lp_collected_token_*_raw`** (top-level wiersza lub `details`). | **OK** |
| A6 | Preflight `target_usd` przy pustych ATA + wartość w LP | `target_usd_for_close_reopen_preflight` + testy ~L3953 | Naprawione w kodzie (BUG-20260510-01); preflight nie powinien już wpaść w `target_usd=0` przy `prev_end>0`. | **OK** (wersja z poprawką na hoście) |
| A7 | Pending-open: **cadence** retry w czasie | `process_pending_open_recoveries` na początku `evaluate_all` + opcjonalny **`CLMM_PENDING_OPEN_MIN_INTERVAL_SECS`** | Faza 1 bez zmian; faza 2: min odstęp między **próbami on-chain** per item (bez bumpu `attempts` przy skip). | **OK** |
| A8 | **Cross-strategy:** budżet `T` z ledgera sesji, kolejka wspólna, brak `strategy_id` w `PendingOpenItem` | `pending_open.rs`, `executor.rs` (`upsert` pending item ~L1388) | Zgodne z **§2.4 faza 1**; brak jawnego handoff strategii w JSON (faza 2). | **OK** (faza 1) / **TODO** (faza 2, jeśli wymagane) |

### Appendix A.1 — Co **nie** jest zaimplementowane (lub tylko norma bez kodu)

**Rdzeń reopen + pending-open (A1–A7):** w typowym deployu uznajemy za **zaimplementowany** w executorze / `rebalance.rs` / `pending_open.rs` — zgodnie z wierszami **OK** w tabeli (w tym opcjonalne env: `CLMM_REOPEN_WALLET_*`, `CLMM_PENDING_OPEN_MIN_INTERVAL_SECS`).

**Nadal poza zakresem lub opcjonalna przyszłość:**

| Temat | Uwaga |
| ----- | ----- |
| **A8 faza 2** | Jawny handoff strategii w `PendingOpenItem` (`origin_strategy_id` / itp.) + migracja JSON — **TODO**, tylko jeśli produkt wymaga twardego „kto kończy czyją sesję”. |
| **A1 — węższy audyt** | Opcjonalny **re-read** `returned_*` wyłącznie z pliku lifecycle zaraz po append `bot_close_position` (zamiast wyłącznie pre-close read + fee z tx) — **nie** zrobiony; faza 1 wystarcza do spójności z zapisem. |
| **§2.3 osobny worker** | Timer/worker recovery **poza** `evaluate_all` — **nie** zaimplementowany; tick nadal z pętli strategii. |
| **§2.1 rezerwacja (nie-3A)** | Twarda rezerwacja tokenów per sesja / escrow — **świadomie poza** modelem 3A. |
| **§6 tryb principal-only** | Osobny tryb sizingu reopen tylko z principal — **nie** zaimplementowany; tylko zapowiedź w §6. |
| **`wallet_deficit_persistent`** | Heurystyka operatorska z §2.2 — **nie** wymaga osobnego automatu w kodzie, dopóki nie zdefiniujecie UI/alertów pod ten label. |
| **Sekcje speca 1 / 4 / 5 / 7 / 8** | **Draft** dokumentacyjny — nie są pełną implementacją Rust; §8 (decision layer) opiera się o [`DECISION_LAYER.md`](DECISION_LAYER.md) do czasu kodu. |
| **Równoległe boty (A/B strategii)** | Porównanie strategii ≠ wspólny plik `pending-open` bez izolacji: **unikalny `CLMM_PENDING_OPEN_RECOVERY_PATH`** (i typowo osobny katalog `data/` lub `CLMM_REPO_ROOT`) na proces; ten sam portfel między procesami = nadal **§2.1** (konkurencja). **A8 faza 2** nie zastępuje izolacji pliku — tylko routing „czyja sesja” wewnątrz **jednej** kolejki bezpiecznej dla jednego writera. |

**Czy „zakończyliśmy implementację tej funkcjonalności”?** — **Tak** dla **rdzenia** opisanego w §2–§2.5 + §6.1 + Appendix A (A1–A7) w wykonaniu **faza 1** (wspólny portfel, 3A, pending-open, preflight, brak cichego downsizingu, refresh `W`, min interval). **Nie** w sensie absolutnym: **A8 faza 2** i pozycje z tabeli powyżej pozostają **opcjonalne / poza zakresem**, dopóki nie zdecydujecie inaczej.

**keywords:** gap-analysis, rebalance, prev_end_value_usd, target_usd, preflight, pending-open, audit, implementation-checklist, implementation-status, A8-phase-2, out-of-scope

---

## Document status

| Field | Value |
| ----- | ----- |
| Status | **Draft + appendix A (gap audit)** — dalsze sekcje 1/4/5/7 do uzupełnienia; **§8** stub decision layer (szczegóły w `DECISION_LAYER.md`) |
| Created | 2026-05-11 |
| Maintainer | team / operator (update when ownership changes) |

**keywords:** functional-spec, normative, documentation, operator, behavior, single-source-of-truth
