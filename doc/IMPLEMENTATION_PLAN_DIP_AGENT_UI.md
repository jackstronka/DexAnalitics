# Plan UI: Jupiter / Dip + bot settings + agent

**Status:** plan (docs only) — 2026-11-09  
**Branch kanoniczny (A+B):** `feat/dip-agent-phase-ab`  
**Powiązane:** [`dip-agent/BRIDGE.md`](dip-agent/BRIDGE.md) (Faza D), [`dip-agent/API-CAPABILITY-MATRIX.md`](dip-agent/API-CAPABILITY-MATRIX.md), [`AI_AGENT_LAYER.md`](AI_AGENT_LAYER.md), [`DECISION_LAYER.md`](DECISION_LAYER.md), istniejący dashboard `web/`.

## Cel

Doprecyzować w jednym miejscu, **co ma pojawić się w UI**, żeby:

1. Obsłużyć **Jupiter Trigger / Dip** (odczyt → propose → live wg faz),  
2. Dać operatorowi **narzędzie do bota LP + settings**,  
3. Ujednolicić **obsługę agentów** (Position Agent już jest; Dip / Decision / audyt — braki),  
4. Przenieść **intuicję UX z 3Commas DCA** (formularz + **tabela drabinki** + Summary box) na Solana/Jupiter — bez kopiowania CEX custodial modelu.

To **nie** jest implementacja ekranów — checklista produktowa pod Fazy C–E + osobne PR UI.

## Zasady UI

- **Reuse** `web/` (React dashboard); **nie** osobna aplikacja Node z `jupiter-dip-agent`.  
- Domyślnie **propose-only**: UI pokazuje propozycję + body + „wymaga potwierdzenia”; live write dopiero po Faza E + flaga.  
- Dip **nie** mutuje `AgentDecision` optimize (`deny_unknown_fields`).  
- Lokalny API: preferuj `http://127.0.0.1:8081` (workspace).  
- Markdown `config/dip/context/` zostaje fallbackiem do czasu stabilnego Trigger history (BRIDGE Faza D).  
- **Inspiracja 3Commas, nie klon:** te same pojęcia (base / safety / TP / required capital), inna egzekucja (wallet + Jupiter Trigger vault, nie API key CEX).

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

## 1b. Inspiracja UX: 3Commas DCA (wplecione w plan)

### 1b.1 Co robił 3Commas (CEX)

3Commas to platforma automatyzacji na **giełdach scentralizowanych** przez **API keys** (Binance, Bybit, OKX, …).  
**Nie** jest native DEX / Solana wallet botem — strony marketingowe „Uniswap/Jupiter/Solana bot” oznaczają zwykle handel tokenami **UNI/JUP/SOL na CEX**, nie routing Jupiter DEX.

Źródła (help center — UX referencyjny):

| Temat | URL |
|-------|-----|
| DCA strategy & setup | https://help.3commas.io/en/articles/16281102-how-the-dca-bot-works-strategy-setup-guide |
| DCA form / interface (tabela drabinki) | https://help.3commas.io/es/articles/3108940-bot-dca-interfaz-y-configuracion-principal |
| Summary box (max drop / avg / required) | https://help.3commas.io/en/articles/16666593-understanding-the-dca-bot-summary-box |
| Supported exchanges (CEX list) | https://3commas.io/exchanges |

### 1b.2 Elementy UI 3Commas, które **przenosimy** do Dex `/dip`

| Wzorzec 3Commas | Znaczenie dla operatora | Nasz odpowiednik UI |
|-----------------|-------------------------|---------------------|
| Formularz Base + Safety orders | Base size, SO size, deviation %, step scale, volume scale, max SO | Form **Dip ladder** (sekcja 1b.4) |
| **Tabela podglądu drabinki** (live przy zmianie ustawień) | Deviation, size, est. price, avg price, TP target, total volume | Komponent **`DipLadderPreviewTable`** (P0 UX) |
| **Summary box** | Max price drop covered · avg entry deviation · Required vs Available | **`DipLadderSummary`** |
| TP % (avg vs base) + multi-TP (do 4) | Wyjście z pozycji | Propose T8 (+ multi-TP później) |
| SL / trailing / move BE | Ochrona | T9 / trailing 🚫 v1 (matryca); SL opcjonalnie flagą |
| Presets strategii | Szybki start | Presety w `config/dip/` + UI dropdown |
| Active deals list | Co jest otwarte | Open Trigger orders (T12/T13) |
| Classic vs advanced layout | Form ± chart | v1: form + tabela; chart P2 |

### 1b.3 Kolumny tabeli drabinki (wymagane w UI)

Wzór 3Commas (BO + SO1…SOn + wiersze TP):

| Kolumna | Opis |
|---------|------|
| Level | `BO` / `SO1`…`SOn` / `TP1`… |
| Deviation % | Odległość od entry / poprzedniego SO |
| Order size (quote) | USDC (lub quote) na ten poziom |
| Order size (base) | Est. SOL przy cenie poziomu |
| Est. trigger / fill price | USD |
| Avg entry after fill | Średnia ważona po wypełnieniu tego poziomu |
| TP price (after this level) | Cel TP przy avg (lub base — toggle jak 3C) |
| Cumulative quote | Suma kapitału do tego poziomu włącznie |

Liczone **lokalnie w `web/src/lib/`** (hermetic unit tests) z aktualnej ceny P1 — bez wołania Jupiter create przy suwaniu sliderów.

### 1b.4 Pola formularza ladder (mapowanie 3Commas → Dex)

| Pole 3Commas | Pole Dex UI | Backend / uwagi |
|--------------|-------------|-----------------|
| Base order size | `base_order_usd` | Pierwszy Buy-the-Dip / natychmiastowy entry policy |
| Safety order size | `safety_order_usd` | Kolejne `POST /orders/price` `below` |
| Price deviation % | `deviation_pct` | Odległość SO1 od entry |
| Deviation step scale | `deviation_step_scale` | Jak 3C: kumulacja kroków |
| Volume / size scale | `volume_scale` | Mnożnik size kolejnych SO |
| Max safety orders | `max_safety_orders` | Cap N |
| Take profit % | `take_profit_pct` | T8 `above`; baza: **avg** (default) \| **base** |
| Multi TP | `tp_targets[]` | v1: 1 TP; v1.1: do 4 |
| Stop loss % | `stop_loss_pct` | T9; default OFF |
| Trailing | — | 🚫 v1 (T11) |
| Exchange account | Wallet + JWT Trigger | Nie CEX API key |
| Pair | `inputMint`/`outputMint` (USDC→SOL…) | `config/dip/mints.yaml` |

### 1b.5 Summary box (obowiązkowy sanity-check przed Create / Propose)

Jak [Summary box 3Commas](https://help.3commas.io/en/articles/16666593-understanding-the-dca-bot-summary-box):

| Metryka | Pytanie operatora |
|---------|-------------------|
| **Max price drop covered** | Jak głęboki dump drabinka jeszcze średiuje? |
| **Max averaging entry deviation** | Gdzie ląduje średni entry po pełnym fillu drabinki? |
| **Required** | Suma BO+SO (kapitał zarezerwowany) |
| **Available** | Vault / wallet USDC (gdy T4/balances dostępne; inaczej „unknown”) |

Reguła UX: **Propose/Create zablokowane** gdy `Required > Available` (gdy Available znane) albo gdy którykolwiek poziom &lt; min Jupiter (~10 USD / round).

### 1b.6 Jupiter API a „bot jak 3Commas” — czy da się?

**Tak, ale dwoma różnymi ścieżkami — nie mylić nazw „DCA”.**

| Model | Jak w 3Commas? | Jak na Jupiterze? | Capability | W Dex |
|-------|----------------|-------------------|------------|-------|
| **A. Safety-order ladder (klasyczny 3C DCA bot)** | Base + SO poniżej + TP od avg | **Wiele** Trigger **price** orders: Buy-the-Dip `below` na poziomach drabinki + TP `above` | T6–T9, T12–T16 | **Główny UX** `/dip` ladder (orkiestracja Dex) |
| **B. Native Jupiter DCA** | Częściowo jak „kupuj co X czasu” | `POST /orders/dca`: `time_based` lub `price_conditional` (pasmo min/max USD), równe rundy, min ~10 USD/rundę | T17–T19 | Osobna karta „Jupiter DCA”; `dca_allowed` |
| **C. Market exit** | Close deal market | Swap V2 S1–S2 | S1–S2 | Awaryjnie |

**Wniosek:** API Jupitera **pozwala** zbudować bota klasy 3Commas na Solana DEX:

- egzekucja limitu / dipu / TP przez Trigger V2 (vault + JWT + podpis deposit),  
- drabinka 3C = **nasza warstwa** (config + UI preview + N× create price order), nie jeden magic endpoint CEX,  
- native `/orders/dca` = uzupełnienie (time / band), **nie** zamiennik tabeli SO.

Ograniczenia vs 3Commas: min size USD, vault lock, brak gwarancji output jak CLOB, cancel = dwustopniowy withdraw, trailing/OCO OFF w v1, brak futures/margin.

Szczegóły endpointów: [`dip-agent/jupiter-API.md`](dip-agent/jupiter-API.md), ID: [`API-CAPABILITY-MATRIX.md`](dip-agent/API-CAPABILITY-MATRIX.md).

### 1b.7 Docs 3Commas w procesie produktowym

Przy projektowaniu ekranu Dip / ladder:

1. Przejrzyj artykuły z §1b.1 (setup + interface + summary).  
2. Zachowaj **tabelę + summary** jako first-class (nie chować za „advanced”).  
3. Każde pole formularza musi mieć tooltip z odpowiednikiem 3C **oraz** ograniczeniem Jupiter (min USD, vault).  
4. Nie kopiuj futures / multipair CEX 1:1 w v1 (multipair = później, osobne boty per mint jak 3C „osobny bot = inne ustawienia”).

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
| **S-DCA** | `dca_allowed` (native Jupiter DCA, model B) | runtime.yaml | P2 |
| **S-LADDER** | default deviation / volume scale / max SO / TP% (presety 3C-like) | `config/dip/runtime.yaml` + zones | P0 UX |
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

## 3b. Reuse inventory — co już jest w Dex (nie budować od zera)

Zasada: **doklejamy** Trigger / Dip / drabinkę do istniejących powierzchni, zamiast drugiego stacku.

| Building block (już w repo) | Ścieżki / API | Użycie pod Jupiter / Dip |
|-----------------------------|---------------|---------------------------|
| **Jupiter Price** | `crates/data/src/providers/jupiter.rs`; `GET /api/v1/prices/jupiter`; `web` `getJupiterPrices` | P1, preview drabinki (U1b), strefy |
| **Trigger read (Faza B)** | `jupiter_trigger.rs`; `GET /dip/snapshot`; CLI `dip-snapshot` | U1 open limits; T12/T13 |
| **Zones** | `config/dip/zones.json`; `clmm_lp_data::dip` | D3, widget strefy |
| **Audyt decyzji** | `GET`/`POST /api/v1/data/agent/decisions` → `agent_decisions.jsonl` | D1 / AH4 / AH5 — Faza C dopina `kind` dip, nie nowy magazyn |
| **Position Agent UI** | `PositionDetail` tab Agent; `/positions/{addr}/agent/*` | Wzorzec UX (status, chat, confirm) — **nie** logika create Trigger |
| **Strategies start/stop** | `Strategies.tsx`; API strategy run | Wzorzec Bot Control BC1–BC2 (LP); dip bot ≠ ten sam executor |
| **BotActivity ledgers** | `BotActivity.tsx`; bot JSONL API | Wzorzec audytu aktywności; osobny ledger Trigger później |
| **Settings shell** | `Settings.tsx` (localStorage) | Rozszerzyć o S-JUP / S-JWT / S-DIP-MODE (U2) — nie nowa app |
| **Tx build / submit** | `txBuildUnsigned`, `txSubmitSigned` (`web` + API `/tx/…`) | Wzorzec podpisu pod deposit/cancel (E); **policy signer Jupiter ≠ Orca** bez jawnej flagi |
| **OpenAPI → TS** | `crates/api/openapi.json`; `npm run gen:api` → `api.gen.ts` | Każdy nowy endpoint dip |
| **Kapitał UI** | Session / Chain capital panels | Inspiracja „Available” w Summary; vault Trigger (T4) to **osobne** saldo |
| **LP position / OOR** | `PositionDetail`, [`LP-HYGIENE.md`](dip-agent/LP-HYGIENE.md) | AH7 alert → `adjust_lp` |
| **Health / port** | `/api/v1/health`; workspace API **8081** | S-API; diagnostyka |
| **Config dip** | `config/dip/*` (runtime, mints, universe, context seed) | S-LADDER defaults; propose refs |

### Świadomie NIE reuse 1:1

| Istniejące | Dlaczego nie mapować na ślepo |
|------------|-------------------------------|
| `AgentDecision` / apply-optimize | `deny_unknown_fields` — tylko optimize LP; dip = osobny wiersz JSONL |
| Orca `swap-before-open` / Whirlpool ExactIn | To nie Jupiter Swap V2 (S1–S2) |
| `DecisionEngine` live | Policy LP rebalance; drabinka Trigger to osobny executor / Portfolio path |
| Birdeye / płatne feedy | Preferuj Price Jupiter + on-chain (workspace free-data rule) |

### Reuse → kroki planu

| Krok | Preferowany reuse |
|------|-------------------|
| U1 | `/dip/snapshot` + Layout nav jak inne strony |
| U1b | Cena z `/prices/jupiter` + czysta matematyka w `web/src/lib/` |
| U3 / AH5 | Istniejące `…/data/agent/decisions` |
| U4 / C | POST decisions + refs do `config/dip/context/` |
| U6 / E | Rozszerzyć `JupiterTriggerClient` write; podpis wzorowany na tx pipeline, osobny mode/signer |
| U5 | Strategies + Settings patterns |

---

## 4. Kolejność wdrożenia UI (zalecana)

| Krok | Zakres | Zależność |
|------|--------|-----------|
| **U0** | Ten dokument + linki + sekcja 3Commas (§1b) | — |
| **U1** | `/dip` read-only: strefa + open orders (`/dip/snapshot`) | Faza B ✅ · **DONE 2026-11-09** |
| **U1b** | **`DipLadderPreviewTable` + `DipLadderSummary`** (lokalna matematyka drabinki, bez create) | tylko web lib + cena P1 |
| **U2** | Settings: S-API, S-JUP, S-JWT, S-DIP-MODE, **S-LADDER** defaults (read) | B + runtime/yaml |
| **U3** | AH5 decisions browser + AH7 OOR alert | API decisions ✅ |
| **U4** | AH4 propose queue — body N× Buy-the-Dip z drabinki + TP | Faza C |
| **U5** | Bot Control BC1–BC3 hardening (dry-run bound to server) | LP API |
| **U6** | Live: materializacja drabinki (T6–T9, T14–T16) + opcjonalnie native DCA T17 | Faza E GO |
| **U7** | AH1 optimize envelope UX | niezależnie |
| **U8** | Native Jupiter DCA card (model B) osobno od ladder | `dca_allowed` |

---

## 5. Kryteria akceptacji (produkt)

- Operator widzi **open Jupiter dips + strefę SOL** bez CLI.  
- Przy konfiguracji drabinki widzi **tabelę poziomów + Summary** (jak 3Commas) zanim propose/live.  
- Settings mówi jasno: **API up**, **Jupiter key?**, **JWT?**, **dip mode**, **ladder defaults**.  
- Position Agent zostaje na pozycji; Dip ma **własną** powierzchnię.  
- Audyt decyzji (LP + Dip) da się **przejrzeć w UI**.  
- Żaden przycisk live Trigger bez `mode: live` + confirm.  
- Operator rozumie różnicę **ladder (model A)** vs **native Jupiter DCA (model B)**.  
- Matryca [`API-CAPABILITY-MATRIX.md`](dip-agent/API-CAPABILITY-MATRIX.md) pozostaje źródłem ID funkcji backendu.

---

## Testy i kryteria regresji

**Obszar testów (R1):** Web · Decision layer / agent · API / kontrakt  
**Pozycja w mapie rozwoju:** Faza D dip-agent (+ Settings/Bot UX); nowa pozycja względem G9 — UI Dip.

| Co chronimy | Typ testu | Fixture / dane | Stan |
|-------------|-----------|----------------|------|
| Client typów `/dip/snapshot` z OpenAPI | kontrakt (`api.gen` / `api.contract`) | wygenerowane OpenAPI | ✅ U1 (`api.gen.test.ts`) |
| Render strefy / empty orders bez JWT | unit web (`dipSnapshotDisplay.test.ts`) | mock `orders_source` | ✅ U1 |
| **Ladder preview:** deviation / avg / required zgodne z formułami (scale=1 i scale≠1) | unit web `dipLadder*.ts` | syntetyczne ceny | ❌ U1b |
| Summary: Required &gt; Available → propose disabled | unit web | mock balances | ❌ U1b |
| Propose card nie woła create bez confirm | unit web | mock decision row | ❌ (C/D) |
| Settings nie serializuje secretów do git paths | manual / unit | — | ❌ |
| OOR alert nie false-positive gdy in-range | unit / integr. | fixture pozycji | ❌ |
| Position Agent tab nadal start/scan | manual / istniejące | PositionDetail | ✅ bazowo |

**Test exit gate:** po U1 — typ `DipSnapshotResponse` + `tsc`; po U1b — zielone unit testy drabinki (Required, max drop, avg deviation) w CI `web`.

**Bugi powiązane:** brak (plan).

**Golden delta:** nie dotyczy (brak ekonomii/goldens).

**Poza automatem:** JWT wallet challenge; live create/cancel na mainnet; wizualny QA alertów OOR.

Po wdrożeniu testów: wpis w [`TESTS.md`](TESTS.md).

---

## Changelog

| Data | Zmiana |
|------|--------|
| 2026-11-09 | v1.2: §3b Reuse inventory (Price, Trigger read, decisions API, tx pipeline, Strategies/Settings, Position Agent UX) |
| 2026-11-09 | v1.1: inspiracja 3Commas DCA (tabela drabinki, Summary, pola); model A ladder vs B native Jupiter DCA; U1b/U8; linki help.3commas.io |
| 2026-11-09 | v1 plan: Jupiter UI features, Bot Control + Settings, Agent Hub checklist, kolejność U0–U7 |
