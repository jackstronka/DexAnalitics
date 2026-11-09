# Plan UI: Jupiter / Dip + bot settings + agent

**Status:** plan (docs only) — 2026-11-09  
**Branch kanoniczny (A+B):** `feat/dip-agent-phase-ab`  
**Powiązane:** [`dip-agent/BRIDGE.md`](dip-agent/BRIDGE.md) (Faza D), [`dip-agent/API-CAPABILITY-MATRIX.md`](dip-agent/API-CAPABILITY-MATRIX.md), [`AI_AGENT_LAYER.md`](AI_AGENT_LAYER.md), [`DECISION_LAYER.md`](DECISION_LAYER.md), istniejący dashboard `web/`.

## Cel

Doprecyzować w jednym miejscu, **co ma pojawić się w UI**, żeby:

1. Obsłużyć **Jupiter Trigger / Dip** (odczyt → propose → live wg faz),  
2. Dać operatorowi **narzędzie do bota LP + settings**,  
3. Ujednolicić **obsługę agentów** (Position Agent już jest; Dip / Decision / audyt — braki).

To **nie** jest implementacja ekranów — checklista produktowa pod Fazy C–E + osobne PR UI.

## Zasady UI

- **Reuse** `web/` (React dashboard); **nie** osobna aplikacja Node z `jupiter-dip-agent`.  
- Domyślnie **propose-only**: UI pokazuje propozycję + body + „wymaga potwierdzenia”; live write dopiero po Faza E + flaga.  
- Dip **nie** mutuje `AgentDecision` optimize (`deny_unknown_fields`).  
- Lokalny API: preferuj `http://127.0.0.1:8081` (workspace).  
- Markdown `config/dip/context/` zostaje fallbackiem do czasu stabilnego Trigger history (BRIDGE Faza D).

---

## 1. Sekcja Jupiter / Dip w UI

### 1.1 Powierzchnia nawigacji (propozycja)

| Element | Ścieżka (propozycja) | Priorytet |
|---------|----------------------|-----------|
| Strona **Dip / Jupiter** | `/dip` lub `/jupiter` | P0 Faza D |
| Widget na Dashboard | karta „Open limits + strefa SOL” | P1 |
| Alert OOR LP | banner na `PositionDetail` + opcjonalnie Dashboard | P0 (LP-HYGIENE) |
| Link z Settings | „Dip runtime / zones” → podstrona lub docs-only do C | P2 |

### 1.2 Funkcje UI ↔ matryca API

| UI feature | Capability ID | API / dane | Faza UI | Tryb |
|------------|---------------|------------|---------|------|
| Cena SOL + **strefa A/B/C** | P1, D3 | `GET /dip/snapshot` | D | read |
| Lista **open Buy-the-Dip** | T12, T13 | `/dip/snapshot` (+ JWT) | D | read |
| Historia orderów (tabela) | T12 | snapshot / przyszły endpoint | D+ | read |
| Status vault (saldo locked) | T4 | Trigger vault | E prep | read |
| Podgląd propose `buy_dip` | T6, T7, D1 | JSONL + body | C→D | propose |
| Podgląd propose TP / SL | T8, T9 | body | C→D | propose |
| Confirm create (live) | T6→T7 | Faza E | E | live + signer |
| Patch trigger / expiry | T14 | | E | propose→live |
| Cancel + withdraw status | T15, T16 | Pending Withdraw | E | propose→live |
| DCA panel | T17–T19 | tylko jeśli `dca_allowed` | E opcjonalnie | — |
| Market exit (Swap) | S1, S2 | rzadko / policy | E | propose→live |
| OCO / trailing | T10, T11 | | — | **nie w UI v1** |
| Deny: Perps / YOLO | — | | — | **nie w UI** |

### 1.3 Stany ekranu Dip (v1)

1. **Unauth / no JWT** — strefa + cena; orders = puste + CTA „ustaw `CLMM_JUPITER_TRIGGER_JWT` / Settings”.  
2. **Read OK** — open dips + expiry + trigger USD.  
3. **Propose pending** — karta z `requires_user_confirm` z JSONL (Faza C).  
4. **Live armed** — przyciski create/cancel widoczne tylko gdy `mode: live` + policy (E).  
5. **Error** — 401/403 JWT, Price fail → hold messaging (ops-ALWAYS-ON).

### 1.4 Poza zakresem v1 UI Jupiter

- Scraping jup.ag HTML  
- Perps / Predict / Card  
- Współdzielony przycisk „rebalance Orca + create Jupiter” w jednym kliku bez osobnych mode

---

## 2. Narzędzie bota + settings w UI

### 2.1 Stan obecny (LP)

| Powierzchnia | Co robi dziś | Luka |
|--------------|--------------|------|
| `Strategies` | lista, start/stop | brak spójnego „bot cockpit” |
| Strategy Create/Edit/Detail | parametry strategii / DecisionEngine | OK jako edycja policy LP |
| `BotActivity` | ledger JSONL aktywności | read-only audyt |
| `Settings` | localStorage: API key, RPC, dry-run, metrics | **nie** bound do runtime bota ani `config/dip` |
| CLI / env | faktyczny start `orca-bot-run`, JWT, API keys | operator poza UI |

### 2.2 Plan: Bot Control (LP)

Cel: jedna strona lub rozbudowa Settings + Strategies = **„Bot / Execution”**.

| Funkcja UI | Opis | Priorytet |
|------------|------|-----------|
| **BC1** Status bota / strategii | running / stopped, ostatni tick, błędy | P0 |
| **BC2** Start / Stop | jak dziś Strategies — czytelniej na cockpit | P0 |
| **BC3** Dry-run vs live | przełącznik + ostrzeżenie; źródło prawdy = API/config, nie tylko localStorage | P0 |
| **BC4** Podgląd `StrategyMode` / progów DecisionEngine | read + link do Edit | P1 |
| **BC5** Ledger skrót | embed lub link do BotActivity | P1 |
| **BC6** Env health | API port, RPC reachability, `JUPITER_API_KEY` present (bez ujawniania sekretu) | P1 |
| **BC7** Signer policy | który signer LP (bez private key w UI) | P2 / E |

### 2.3 Plan: Settings — Dip + wspólne

Rozszerzyć `Settings` (lub `/settings/dip`) o **jawne sekcje**:

| Sekcja | Pola (UI) | Źródło prawdy | Priorytet |
|--------|-----------|---------------|-----------|
| **S-API** | Base URL API (default 8081) | env / local | P0 (dziś częściowo) |
| **S-RPC** | Solana RPC URL | config | P0 |
| **S-JUP** | Jupiter API key set? (boolean) | env server-side | P0 |
| **S-JWT** | Trigger JWT set? / refresh hint | `CLMM_JUPITER_TRIGGER_JWT` | P0 Faza D |
| **S-DIP-MODE** | `propose-only` \| `paper` \| `live` | `config/dip/runtime.yaml` (read via API TBD) | P0 C/D |
| **S-ZONES** | podgląd pasm A/B/C (read-only lub edycja z zapisem) | `config/dip/zones.json` | P1 |
| **S-DCA** | `dca_allowed` | runtime.yaml | P2 |
| **S-METRICS** | pnl mode | local (już jest) | ✅ |

**NO-GO w Settings UI:** wklejanie private key / seed; commit sekretów do `context/`.

### 2.4 API potrzebne pod settings (backend, osobne taski)

| Endpoint (propozycja) | Cel |
|-----------------------|-----|
| `GET /api/v1/dip/runtime` | mode, flags (bez secretów) |
| `GET /api/v1/dip/snapshot` | już jest (B) |
| `GET /api/v1/system/health-secrets` lub rozszerzenie health | „key present” booleans |
| Opcjonalnie `PATCH /dip/runtime` | tylko gdy product zdecyduje o write config z UI |

Do czasu tych endpointów: Settings pokazuje docs path + „edytuj yaml / env”.

---

## 3. Obsługa agenta w UI

Mapowanie z [`AI_AGENT_LAYER.md`](AI_AGENT_LAYER.md):

| Warstwa | Znaczenie | UI dziś | Plan UI |
|---------|-----------|---------|---------|
| **A. AgentDecision / apply-optimize** | envelope siatki | pośrednio strategie / backtest | **AH1** panel „Optimize apply / decision envelope” na StrategyDetail |
| **B. Position Agent** | czat / skan / supervisor przy pozycji | ✅ zakładka Agent na `PositionDetail` | **AH2** dopracowanie UX (status worker, LLM mode badge, link do JSONL) |
| **C. DecisionEngine** | reguły live LP | ukryte w parametrach strategii | **AH3** read-only „ostatnia decyzja hold/rebalance” na Position/Strategy |
| **D. Dip Agent** | Buy the Dip propose/live | ❌ | **AH4** strona Dip + kolejka propose (Faza C/D) |
| Audyt | `agent_decisions.jsonl` | API istnieje | **AH5** strona `/agent/decisions` lub tab w Logs — filtry source/kind |
| Orkiestrator / chart agent | wizja | ❌ / TODO_CHART | **poza** tym planem v1 (link tylko) |

### 3.1 Funkcje UI agenta (checklist)

| ID | Funkcja | Gdzie | Faza |
|----|---------|-------|------|
| AH1 | Podgląd / submit `AgentDecision` + busy state | StrategyDetail / Backtests | osobny PR |
| AH2 | Position Agent: start, scan, chat, supervisor (utrzymać) | PositionDetail | ✅ + polish |
| AH3 | Ostatnia decyzja DecisionEngine (hold/rebalance + reason) | PositionDetail | P1 |
| AH4 | Dip proposes: lista `buy_dip\|hold\|…`, confirm/reject | `/dip` | C→D |
| AH5 | Browser `agent_decisions.jsonl` | `/agent/decisions` lub Logs | P1 |
| AH6 | Badge `mode` (propose-only / live) globalnie gdy Dip włączony | Layout / Dip | D |
| AH7 | Alert LP OOR → sugestia `adjust_lp` | PositionDetail | D |

**Nie łączyć** w jednym przycisku: Position Agent chat „zrób rebalance” + Jupiter create bez osobnych confirmów.

---

## 4. Kolejność wdrożenia UI (zalecana)

| Krok | Zakres | Zależność |
|------|--------|-----------|
| **U0** | Ten dokument + linki w BRIDGE/README/matrix | — |
| **U1** | `/dip` read-only: strefa + open orders (`/dip/snapshot`) | Faza B ✅ |
| **U2** | Settings: S-API, S-JUP, S-JWT, S-DIP-MODE (read) | B + runtime API lub yaml hint |
| **U3** | AH5 decisions browser + AH7 OOR alert | API decisions ✅ |
| **U4** | AH4 propose queue | Faza C |
| **U5** | Bot Control BC1–BC3 hardening (dry-run bound to server) | LP API |
| **U6** | Live buttons create/cancel (T6–T16) | Faza E GO |
| **U7** | AH1 optimize envelope UX | niezależnie |

---

## 5. Kryteria akceptacji (produkt)

- Operator widzi **open Jupiter dips + strefę SOL** bez CLI.  
- Settings mówi jasno: **API up**, **Jupiter key?**, **JWT?**, **dip mode**.  
- Position Agent zostaje na pozycji; Dip ma **własną** powierzchnię.  
- Audyt decyzji (LP + Dip) da się **przejrzeć w UI**.  
- Żaden przycisk live Trigger bez `mode: live` + confirm.  
- Matryca [`API-CAPABILITY-MATRIX.md`](dip-agent/API-CAPABILITY-MATRIX.md) pozostaje źródłem ID funkcji backendu.

---

## Testy i kryteria regresji

**Obszar testów (R1):** Web · Decision layer / agent · API / kontrakt  
**Pozycja w mapie rozwoju:** Faza D dip-agent (+ Settings/Bot UX); nowa pozycja względem G9 — UI Dip.

| Co chronimy | Typ testu | Fixture / dane | Stan |
|-------------|-----------|----------------|------|
| Client typów `/dip/snapshot` z OpenAPI | kontrakt (`api.gen` / `api.contract`) | wygenerowane OpenAPI | 🟡 po U1 |
| Render strefy / empty orders bez JWT | unit web (`*.test.ts`) | mock JSON snapshot | ❌ |
| Propose card nie woła create bez confirm | unit web | mock decision row | ❌ (C/D) |
| Settings nie serializuje secretów do git paths | manual / unit | — | ❌ |
| OOR alert nie false-positive gdy in-range | unit / integr. | fixture pozycji | ❌ |
| Position Agent tab nadal start/scan | manual / istniejące | PositionDetail | ✅ bazowo |

**Test exit gate:** po U1 — co najmniej 1 test web na mapowanie `DipSnapshotResponse` + `npm run gen:api` po zmianach OpenAPI; `npx tsc --noEmit` w `web/`.

**Bugi powiązane:** brak (plan).

**Golden delta:** nie dotyczy (brak ekonomii/goldens).

**Poza automatem:** JWT wallet challenge; live create/cancel na mainnet; wizualny QA alertów OOR.

Po wdrożeniu testów: wpis w [`TESTS.md`](TESTS.md).

---

## Changelog

| Data | Zmiana |
|------|--------|
| 2026-11-09 | v1 plan: Jupiter UI features, Bot Control + Settings, Agent Hub checklist, kolejność U0–U7 |
