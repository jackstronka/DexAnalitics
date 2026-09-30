# Portfel łańcucha sesji — norma produktowa

**Status:** norma (do akceptacji operatora); **bez implementacji** do jawnego **GO**  
**Data:** 2026-05-26  
**keywords:** chain_session_portfolio, logical_position_id, chain_session_id, portfel łańcucha, SESSION GL, rebalance_session_id, lifecycle, lineage, operator close, capital continuity

**Powiązane:** [`WALLET_GL.md` §2.2](WALLET_GL.md), [`WALLET_SESSION_GL_INTEGRATION_ANALYSIS.md`](WALLET_SESSION_GL_INTEGRATION_ANALYSIS.md), [`WALLET_SESSION_CAPITAL_EXECUTOR_PLAN.md`](WALLET_SESSION_CAPITAL_EXECUTOR_PLAN.md), [`FUNCTIONAL_SPECIFICATION.md`](FUNCTIONAL_SPECIFICATION.md) §2, §6.1, [`IMPERMANENT_LOSS_USD_AND_FEES.md`](IMPERMANENT_LOSS_USD_AND_FEES.md), [`IMPLEMENTATION_PLAN_CHAIN_SESSION_PORTFOLIO.md`](IMPLEMENTATION_PLAN_CHAIN_SESSION_PORTFOLIO.md)

---

## 1. Cel

Operator prowadzi **jedną strategię LP** jako **ciąg rotacji** (wiele PDA, jedna linia biznesowa od pierwszego open do ręcznego lub finalnego close). Potrzebuje **jednego logicznego portfela**, który:

1. Przyjmuje kapitał z **close** i **collect** (principal + LP fees).
2. Finansuje **open** kolejnej pozycji i **opłaty transakcyjne** (SOL/λ) w ramach tej samej strategii.
3. Ma **jawną wycenę startu** (np. target ~$10, realny open ~$9.95 w cenach z momentu open, znane ilości tokenów).
4. Po zakończeniu daje **pełną historię** zdarzeń i **saldo vs start** — bez mieszania z resztą portfela on-chain operatora.

Ta norma **uzupełnia** (nie zastępuje od razu) istniejące:

- **`SESSION:{rebalance_session_id}`** — jeden cykl rebalance (close → open),
- **lineage / stream-lineage** — wynik ekonomiczny i tabela rotacji PDA,
- **portfel on-chain** — fizyczne ATA (policy 3A).

---

## 2. Definicje

| Termin | Znaczenie |
| ------ | --------- |
| **Portfel łańcucha** | Logiczne konto księgowe strategii od **pierwszego open** do **zamknięcia cyklu** (ręcznego lub ostatniego close bez reopen). |
| **`chain_session_id`** | Stabilny UUID (propozycja nazwy pola) przypisany przy starcie cyklu; **ten sam** na wszystkich rotacjach tej strategii. |
| **`rebalance_session_id`** | UUID **jednego** rebalance (close → swap → open). Zostaje jako pod-zdarzenie w historii; **nie** definiuje portfela całego łańcucha. |
| **Kapitał wdrożony (start)** | Tokeny + wycena USD w momencie **pierwszego open** cyklu (`open_amount_*_raw`, `open_quote_estimated_value_usd` lub równoważnik). |
| **Saldo portfela łańcucha** | Suma skorygowanych delt tokenów na koncie logicznym (per mint), **bez** NAV pozycji LP między close a open. |
| **NAV LP (w puli)** | Wartość bieżącej pozycji on-chain — **osobna linia** w UI; nie mylić z „portfel sesji teraz”. |
| **Zamknięcie cyklu** | Operator kończy strategię (ręczny close bez reopen **lub** jawna akcja „zakończ cykl”); portfel łańcucha pozostaje w PG do audytu (retention). |

**Propozycja kodu konta GL:** `CHAIN:{chain_session_id}` lub rozszerzenie `SESSION:` z polem `scope=chain` — decyzja w planie implementacji (preferowane: **osobny `account_type=chain`** żeby nie psuć istniejącego SESSION per rebalance).

---

## 3. Norma produktowa (przepływ operatora)

### 3.1 Start cyklu

1. Operator otwiera pierwszą pozycję (ręcznie lub bot) z targetem kapitału (np. **$10**).
2. System zapisuje:
   - **`chain_session_id`** (nowy UUID, jeśli brak),
   - **`rebalance_session_id`** pierwszego rebalance (jak dziś),
   - ilości tokenów open (`open_amount_a_raw`, `open_amount_b_raw`),
   - wycenę open (np. **$9.95** — realny depozyt, nie tylko target),
   - timestamp, signature, PDA, pool mints.
3. **Posting:** portfel łańcucha **−** depozyt (tokeny idą do LP), zapis **start snapshot** (kapitał wdrożony).

### 3.2 Praca w puli

- NAV LP zmienia się z rynkiem — to **nie** jest saldo portfela łańcucha.
- **Collect** (gdy strategia zbiera fee): **+** zebrane tokeny fee → portfel łańcucha.

### 3.3 Rotacja (poza range, strategia)

1. **Close** zgodnie ze strategią: principal (+ ewentualnie fee na wierszu close) **+** do portfela łańcucha.
2. **Opłata tx** close: **−** SOL (λ) z portfela łańcucha (po przeliczeniu na USD w raportach).
3. Ewentualny **swap** przed open: delty **wewnątrz** portfela łańcucha (mint A ↔ mint B), **credit przed debit open** — bez „phantom” ujemnych mintów.
4. **Open** nowej PDA: depozyt **tylko do wysokości dostępnego salda portfela łańcucha** (per mint); brak cichego dofinansowania z globalnego portfela bez jawnego zdarzenia „dopłata zewnętrzna” (patrz §5.3).
5. Nowy `rebalance_session_id`, **ten sam** `chain_session_id`.

### 3.4 Koniec cyklu

- Ręczny **close** (bez reopen) lub jawne zamknięcie cyklu.
- Ostatni zwrot z LP → portfel łańcucha.
- UI/API: **pełna historia** (lifecycle + posting GL + lineage PDA) oraz **wynik vs start** (§4).

---

## 4. Metryki obowiązkowe (UI / API)

Operator musi widzieć **jeden panel** powiązany z `chain_session_id` (nie tylko ostatni `rebalance_session_id`):

| Metryka | Definicja | Źródło |
| ------- | --------- | ------ |
| **Start cyklu — USD** | Kapitał wdrożony przy pierwszym open | snapshot + ceny z eventu open |
| **Start cyklu — tokeny** | Ilości A/B (i minty) | pierwszy open lifecycle |
| **Portfel łańcucha teraz — tokeny** | Saldo GL/PSLR per mint | suma Δ na `CHAIN:{id}` |
| **Portfel łańcucha teraz — USD** | Suma tokenów × ceny (domyślnie: ceny z **startu** lub jawny wybór „ceny live”) | jawna metoda wyceny |
| **W puli teraz — USD** | NAV bieżącej głowy łańcucha | RPC / snapshots (jak dziś) |
| **Opłaty tx (suma)** | Σ λ → USD w ramach cyklu | lifecycle + posting |
| **LP fees (suma)** | Σ zebranych fee w cyklu | lifecycle / lineage |
| **Wynik vs start** | `(portfel + NAV_LP + ewent. wycofany jawny)` − start − tx | patrz §4.1 |

### 4.1 Relacja z „Wynikiem ekonomicznym łańcucha” (lineage)

- **Lineage `net_pnl_usd`** pozostaje metryką **analityczną** (baseline pierwszego PDA, cashflow model lifecycle, end NAV) — patrz [`IMPERMANENT_LOSS_USD_AND_FEES.md`](IMPERMANENT_LOSS_USD_AND_FEES.md).
- **Portfel łańcucha** jest **księgowym** modelem tokenów: „ile strategia ma w swoim sub-portfelu + co siedzi w LP”.
- Po wdrożeniu: UI **nie** powinno sugerować, że `SESSION:{rebalance_session_id}` „Portfel sesji teraz” = cały cykl od $10.
- Docelowo: **reconcile** między portfelem łańcucha a lineage (tolerancja USD, jawne rozbieżności).

---

## 5. Reguły księgowania (norma)

Bazują na istniejącej funkcji `session_mint_deltas_from_lifecycle_json` ([`WALLET_SESSION_GL_INTEGRATION_ANALYSIS.md` §6](WALLET_SESSION_GL_INTEGRATION_ANALYSIS.md#6-reguła-liczenia-δ-na-session-jedna-funkcja-wielu-konsumentów)), z **tą samą semantyką Δ**, ale konto = **`CHAIN:{chain_session_id}`**.

| Zdarzenie lifecycle | Δ portfel łańcucha |
| ------------------- | ------------------ |
| **Pierwszy / kolejny open** | **−** `open_amount_a_raw`, **−** `open_amount_b_raw` (minty puli) — **tylko jeśli** saldo ≥ depozyt; inaczej §5.2 |
| **close** | **+** `close_amount_*_raw`, **+** `lp_collected_*` |
| **collect** | **+** `lp_collected_*` |
| **swap** (w sesji rebalance) | delty z `fee_payer_token_deltas` (swap **przed** open musi być w GL) |
| **tx fee** | **−** SOL (native / WSOL) — jawny posting z λ wiersza lifecycle |
| **transfer / convert global** | **poza** portfelem łańcucha v1 (konto WALLET global) |
| **Dopłata zewnętrzna** | jawne zdarzenie `chain_external_funding` (przyszłość) — **+** do portfela łańcucha |

**Idempotencja:** `event_id = lifecycle:{signature}` (jak dziś).

### 5.1 Zasada ciągłości kapitału

Po **close** i przed **open** suma tokenów w portfelu łańcucha (wyceniona w USD przy cenach z close/open) powinna odpowiadać **T** z [`FUNCTIONAL_SPECIFICATION.md`](FUNCTIONAL_SPECIFICATION.md) §6.1 (`returned_*_raw`), z tolerancją zaokrągleń i opłat tx.

**Open kolejnej pozycji:** target depozytu **≤** saldo portfela łańcucha (per mint po swap-mix). Bot **nie** downsizuje poniżej T bez jawnego powodu w lifecycle; **nie** debetuje mintów, których nie ma w saldzie (koniec „phantom USDC”).

### 5.2 Niewystarczające saldo

Jeśli po close brakuje nogi (np. close zwrócił głównie SOL, open wymaga USDC):

1. **Swap wewnątrz cyklu** — delty swapu **najpierw** creditują portfel łańcucha, potem open debetuje.
2. Jeśli swap-mix nadal wymaga tokenów spoza portfela łańcucha → **błąd / pending-open**, **nie** cichy debet ujemny w GL (regresja BUG-20260520-02 / phantom USDC).
3. Opcjonalna **dopłata zewnętrzna** (faza późniejsza) — osobny typ zdarzenia, widoczny w historii.

### 5.3 On-chain vs logiczny portfel

- Fizycznie tokeny nadal na **tym samym signerze** (policy 3A) — [`FUNCTIONAL_SPECIFICATION.md`](FUNCTIONAL_SPECIFICATION.md) §2.1.
- **Portfel łańcucha** = **sub-ledger analityczny + docelowo kapsy executora** (`min(RPC, CHAIN)`), nie osobny keypair.
- Izolacja **logiczna** (5a); twarda rezerwacja on-chain (5b) — poza zakresem v1 tej normy.

---

## 6. Historia i audyt

Na koniec cyklu operator ma dostęp do:

1. **Timeline lifecycle** — wszystkie wiersze z `chain_session_id` (open/close/collect/swap), pogrupowane po `rebalance_session_id`.
2. **Tabela rotacji PDA** — lineage (jak „Historia pozycji” dziś).
3. **Ledger posting** — `wallet_gl_posting` dla `CHAIN:{id}` (mint, delta, signature, kind).
4. **Snapshot start / koniec** — USD + tokeny.
5. **Eksport** — JSON/CSV (faza UI późniejsza; API v1 wystarczy).

**Retention:** zamknięcie cyklu **nie usuwa** konta portfela łańcucha (jak SESSION dziś).

---

## 7. Przypisanie `chain_session_id`

| Moment | Reguła |
| ------ | ------ |
| Pierwszy open strategii | Generuj `chain_session_id`; zapisz w lifecycle `details` + PSLR + opcjonalnie strategia/registry |
| Rotacja (bot) | **Kopiuj** `chain_session_id` z poprzedniego wiersza łańcucha (lineage parent / strategy link) |
| Ręczny open w tej samej strategii | Operator podaje lub UI proponuje istniejący `chain_session_id` |
| Nowa strategia / nowy eksperyment | **Nowy** `chain_session_id` |
| Operator open bez strategii | UUID z `cost_session_id` **może** służyć jako `chain_session_id` v1 (decyzja w planie) |

**Kotwica lineage:** pierwszy PDA łańcucha lub `chain_anchor_pubkey` z materializacji chain-history — spójność z [`POSITION_CHAIN_HISTORY_PLAN.md`](POSITION_CHAIN_HISTORY_PLAN.md).

---

## 8. Antywzorce (obecne rozjazdy — do usunięcia)

| Antywzorzec | Dlaczego złe | Norma |
| ----------- | ------------ | ----- |
| UI „Portfel sesji teraz” = cały cykl od $10 | Myli **ostatni rebalance** z całą strategią | Osobny panel **portfel łańcucha** |
| Open debetuje SESSION o pełne `open_amount_*`, gdy mint przyszedł z globalnego portfela | Ujemne USDC w GL (~−$2.08) przy ~$0 w sesji | Swap credit → open debit; cap per saldo |
| 17× `rebalance_session_id` bez wspólnego ID | Rozbity portfel księgowy | Jeden `chain_session_id` |
| `delta_vs_pre_open` = −wdrożony kapitał po open | Wygląda jak strata całego cyklu | Osobne metryki: „w puli”, „w portfelu”, „vs start” |
| Lineage cashflow vs SESSION saldo | Dwa modele bez reconcile | Jawny reconcile + dokumentacja różnic |

---

## 9. Kryteria akceptacji normy (Definition of Done produktu)

1. Jeden `chain_session_id` obejmuje wszystkie rotacje strategii do zamknięcia cyklu.
2. Saldo portfela łańcucha = suma close + collect + fee − open − tx ± swap **bez ujemnych phantom mintów** przy typowym rebalance SOL/USDC.
3. UI pokazuje: **start**, **portfel łańcucha**, **NAV LP**, **historia**, **wynik vs start**.
4. Executor (z flagą) planuje open/reopen w granicach **salda portfela łańcucha**.
5. Ręczny close końcowy zostawia audytowalną historię w PG.
6. Test regresji na sesji z ujemnym USDC (np. `1f33b923…` / `9vhKYHA…`) — po backfillu saldo zgodne z oczekiwanym ~resztką SOL, USDC ≥ 0.

---

## 10. Otwarte decyzje (do GO implementacji)

| # | Pytanie | Propozycja |
| - | ------- | ---------- |
| D1 | Nazwa pola: `chain_session_id` vs `logical_position_id` | `chain_session_id` w lifecycle; alias w API |
| D2 | Osobne konto `CHAIN:` vs ten sam `SESSION:` z `scope` | **`account_type=chain`** (czytelniejsze migracje) |
| D3 | Ceny USD w panelu: freeze z startu vs live | Start freeze domyślnie; toggle „ceny live” |
| D4 | Czy `cost_session_id` pierwszego open = `chain_session_id` | Tak dla v1 ręcznego flow |
| D5 | Reconcile z lineage net PnL | Faza 2; tolerancja $ i lista przyczyn |

---

## 11. Powiązanie z istniejącą dokumentacją

| Dokument | Relacja |
| -------- | ------- |
| [`WALLET_GL.md`](WALLET_GL.md) §2.2 | Poprzednia norma **per rebalance**; portfel łańcucha **nadbudowa** |
| [`WALLET_SESSION_CAPITAL_EXECUTOR_PLAN.md`](WALLET_SESSION_CAPITAL_EXECUTOR_PLAN.md) | Kapsy `min(RPC, SESSION)` → docelowo `min(RPC, CHAIN)` |
| [`UI_REQUIREMENTS_PHASE1.md`](UI_REQUIREMENTS_PHASE1.md) | Wzmianka `logical_position_id` — **realizacja przez ten dokument** |
| [`IMPERMANENT_LOSS_USD_AND_FEES.md`](IMPERMANENT_LOSS_USD_AND_FEES.md) | Lineage / IL — warstwa równoległa |
| [`CHAIN_ECONOMIC_NET_REFACTOR_PLAN.md`](CHAIN_ECONOMIC_NET_REFACTOR_PLAN.md) | Net PnL nagłówka — reconcile z portfelem łańcucha |

**Plan wdrożenia:** [`IMPLEMENTATION_PLAN_CHAIN_SESSION_PORTFOLIO.md`](IMPLEMENTATION_PLAN_CHAIN_SESSION_PORTFOLIO.md)
