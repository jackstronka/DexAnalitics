# Dip / Jupiter — matryca capability (API → funkcja → status)

**Cel v1:** pełny cykl tradingu **spot** przez API w granicach allowlisty — nie „cały Jupiter”.  
**Źródło endpointów:** [`jupiter-API.md`](jupiter-API.md) · produkty: [`jupiter-ALLOWED.md`](jupiter-ALLOWED.md) · fazy: [`BRIDGE.md`](BRIDGE.md).  
**Docs zewnętrzne:** [Trigger overview](https://developers.jup.ag/docs/trigger), [deposit](https://developers.jup.ag/docs/trigger/deposit), [manage orders](https://developers.jup.ag/docs/trigger/manage-orders).

Legenda statusu:

| Symbol | Znaczenie |
|--------|-----------|
| ✅ | Zaimplementowane w Dex (read path) |
| ⬜ | Zaplanowane (propose-only → live wg fazy) |
| 🚫 | Explicit deny v1 (osobny GO żeby włączyć) |

Tryby:

- **read** — bez podpisu / bez broadcast  
- **propose** — gotowe body + checklista podpisu; `requires_user_confirm`  
- **live** — `runtime.mode: live` + signer + Faza E GO  

---

## 1. Price

| ID | Funkcja | Endpoint | Tryb | Faza | Status |
|----|---------|----------|------|------|--------|
| P1 | Cena USD watchlisty | Price `GET` v3 (Dex: `GET /prices/jupiter`) | read | A/B | ✅ |

---

## 2. Trigger V2 — auth / vault / deposit

| ID | Funkcja | Endpoint | Tryb | Faza | Status |
|----|---------|----------|------|------|--------|
| T1 | Auth challenge | `POST /auth/challenge` | read/setup | B | ✅ |
| T2 | Auth verify → JWT | `POST /auth/verify` | read/setup | B | ✅ |
| T3 | Auth refresh | `POST /auth/refresh` | read/setup | B | ✅ |
| T4 | Status vault | `GET /vault` | read | E prep | ⬜ |
| T5 | Rejestracja vault | `GET /vault/register` | live (raz) | E | ⬜ |
| T6 | Craft deposit (fund order) | `POST /deposit/craft` | propose → live | E | ⬜ |

Każdy create limitu wymaga vault + signed deposit (`depositRequestId` + `depositSignedTx`).

---

## 3. Trigger V2 — price orders (Limit / Buy the Dip / TP / SL)

| ID | Funkcja | Endpoint | Tryb | Faza | Status |
|----|---------|----------|------|------|--------|
| T7 | Create **Buy the Dip** (`below`) | `POST /orders/price` | propose → live | C body / E exec | ⬜ |
| T8 | Create **Take Profit** (`above`) | `POST /orders/price` | propose → live | C / E | ⬜ |
| T9 | Create **Stop Loss** | `POST /orders/price` | propose → live | E (flag) | ⬜ |
| T10 | Create **OCO / OTOCO** | `POST /orders/price` | — | — | 🚫 |
| T11 | **Trailing** (`trailingBps`) | `POST /orders/price` | — | — | 🚫 |
| T12 | Historia / open orders | `GET /orders/history` | read | B | ✅ |
| T13 | Filtr open Buy-the-Dip | lokalnie na T12 | read | B | ✅ |
| T14 | Patch trigger / slippage | `PATCH /orders/price/{id}` | propose → live | E | ⬜ |
| T15 | Cancel start | `POST /orders/price/cancel/{id}` | propose → live | E | ⬜ |
| T16 | Confirm cancel + withdraw | `POST /orders/price/confirm-cancel/{id}` | live + sign | E | ⬜ |

### Powierzchnia Dex (obecna)

| Surface | Pokrywa |
|---------|---------|
| `JupiterTriggerClient` | T1–T3, T12 (+ T13 helper) |
| `GET /api/v1/dip/snapshot` | P1 (via Price) + strefy + T13 |
| CLI `dip-snapshot` | j.w. |

**Create/cancel/patch/deposit: NO-GO do Fazy E.**

---

## 4. Trigger V2 — DCA (opcjonalnie)

Włączane tylko gdy `runtime.jupiter.dca_allowed: true`.

| ID | Funkcja | Endpoint | Tryb | Faza | Status |
|----|---------|----------|------|------|--------|
| T17 | DCA create | `POST /orders/dca` | propose → live | E | ⬜ |
| T18 | DCA history | `GET /orders/history/dca` | read | E prep | ⬜ |
| T19 | DCA cancel + confirm | cancel + confirm-cancel | live | E | ⬜ |

---

## 5. Swap V2 — market (awaryjny exit / mały rebalance)

**Nie** do łapania dipu (dip = Trigger).

| ID | Funkcja | Endpoint | Tryb | Faza | Status |
|----|---------|----------|------|------|--------|
| S1 | Quote + unsigned tx | `GET /swap/v2/order` | propose → live | E | ⬜ |
| S2 | Execute signed | `POST /swap/v2/execute` | live | E | ⬜ |
| S3 | Custom build | `GET /swap/v2/build` | opcjonalnie | E | ⬜ |

---

## 6. Decision / Portfolio (warstwa Dex, nie Jupiter HTTP)

| ID | Funkcja | Artefakt | Faza | Status |
|----|---------|----------|------|--------|
| D1 | Propose `buy_dip\|sell\|hold\|cancel_order\|adjust_lp\|skip` → JSONL | `POST /dip/propose` + CLI `dip-propose` → `agent_decisions.jsonl` (`kind=dip_propose`) | C | ✅ |
| D2 | UI open limits + alert LP OOR (+ bot settings + agent surfaces) | dashboard — szczegóły: [`IMPLEMENTATION_PLAN_DIP_AGENT_UI.md`](../IMPLEMENTATION_PLAN_DIP_AGENT_UI.md); **U1** `/dip` snapshot ✅ | D | 🟡 U1 |
| D2b | UI **drabinka 3Commas-like** (preview table + Summary; N× Buy-the-Dip) | `web` lib + `/dip` — model A w planie UI §1b | D (U1b) | ⬜ |
| D2c | UI native Jupiter DCA (time/band) | osobna karta; T17–T19 | E opcjonalnie (U8) | ⬜ |
| D3 | Strefy SOL A/B/C | `config/dip/zones.json` + `clmm_lp_data::dip` | B | ✅ |

---

## 7. Explicit deny (nie wołać)

| Temat | Status |
|-------|--------|
| Jupiter Perps / YOLO / Predict / Gacha | 🚫 |
| Trigger V1 `/trigger/v1` | 🚫 |
| Recurring V1 `/recurring/v1` | 🚫 |
| Transfer na „losowy” vault poza `/deposit/craft` | 🚫 |
| Scraping HTML jup.ag zamiast API | 🚫 (wyjątek: awaria + manual) |
| Shared keypair Orca + Jupiter bez policy | 🚫 |
| Auto-live create w tym samym procesie co Orca rebalance | 🚫 |

---

## 8. Mapowanie Decision → sekwencja API

| `action` | Sekwencja (live) | Propose-only |
|----------|------------------|--------------|
| `buy_dip` | P1 → T1–T3 → T4–T6 → T7 → T12 | body T6/T7 + checklista |
| `sell` (limit TP) | j.w. z T8 (minty odwrotne, `above`) | body T8 |
| `sell` (market) | S1 → sign → S2 | body S1 |
| `hold` | P1 + T12 | tylko read |
| `cancel_order` | T15 → sign withdraw → T16 | body T15 |
| `adjust_lp` | **nie Trigger** — Orca | TBD / blocked |
| `skip` | — | — |

Szczegóły pól create: [`jupiter-API.md`](jupiter-API.md) §3.2.

---

## 9. Minimalny cykl Analysis (przed Decision)

1. **P1** — `usdPrice` SOL (+ aktywne theses)  
2. **T12 / T13** — open Trigger orders (trigger, size, expiry)  
3. Opcjonalnie **S1** bez execute — sanity płynności  
4. Kontekst markdown (`config/dip/context/…`) — news / thesis  

---

## 10. Kryteria „pełnego tradingu przez API” (Definition of Done E)

- [ ] T4–T6 + T7–T9 + T14–T16 w `JupiterTriggerClient` (hermetic httpmock)  
- [ ] Propose path zwraca body bez broadcast (`mode: propose-only`)  
- [ ] Live path za flagą + osobny signer  
- [ ] Pending Withdraw po cancel dopięty zanim nowe `buy_dip`  
- [ ] S1–S2 tylko dla policy `sell` market / pilny exit  
- [ ] T10/T11/T17 nadal 🚫 lub za jawną flagą runtime  
- [ ] Brak rozszerzania `AgentDecision` (`deny_unknown_fields`) — dip = D1  

---

## 11. Changelog

| Data | Zmiana |
|------|--------|
| 2026-11-09 | v1 matryca: P/T/S/D IDs, status A+B vs C–E, deny list |
