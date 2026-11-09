# Jupiter API — allowlista dla Decision / Portfolio Agent

Źródło prawdy: [developers.jup.ag](https://developers.jup.ag/)  
Portal kluczy: [developers.jup.ag/portal](https://developers.jup.ag/portal)

Agent **nie zgaduje** endpointów. Używa tylko tabel poniżej.  
UI jup.ag jest fallbackiem gdy API niedostępne lub mode = wyłącznie manual.

---

## Auth (wspólne)

| Element | Zasada |
|---------|--------|
| `x-api-key` | Wymagany na requestach API; klucz **tylko** w env / secret store, nigdy w `context/` ani git |
| Trigger V2 JWT | Challenge–response: podpis wallet → `Authorization: Bearer <jwt>` |
| Co JWT może | Tworzyć / edytować / cancel orderów; **nie** wyprowadza funduszy bez osobnej podpisanej tx withdraw |
| Co agent może w `propose-only` | Czytać ceny; **proponować** body create/cancel; **nie** podpisywać deposit/withdraw |
| Live | Wymaga `runtime.mode: live` + osobny signer (nie seed w repo) |

Env (przykład, nie commitować wartości):

```bash
JUPITER_API_KEY=
SOLANA_RPC_URL=
# opcjonalnie ścieżka do signera / policy engine — nie private key w plain text
```

---

## Role × API

| Rola | Read | Write (tylko live + signer) |
|------|------|-------------------------------|
| **Decision** | Price; (opcjonalnie) order history jeśli Portfolio dostarczy | — (tylko decyzja tekstowa) |
| **Portfolio** | Price, vault status, order history | Swap execute, Trigger deposit+create, update, cancel+withdraw |
| **Context** | — | — (tylko pliki `context/`) |

---

## 1) Price API — READ (Decision + Portfolio)

**Preferuj V3** (V2 deprecated).

| Method | Base / path | Użycie agenta |
|--------|-------------|----------------|
| `GET` | `https://api.jup.ag/price/v3` (wg aktualnych docs Price) | `usdPrice` tokenów z watchlisty przed Analysis |

**Implementacja read (Faza A):** reuse Dex `JupiterProvider` + `GET /prices/jupiter`; minty: `config/dip/mints.yaml`.  
**Trigger client:** Faza B+ w Rust/API (historyczny szkic Node: `Documents/jupiter-dip-agent/src/trigger/`).

**Zasady:**
- Przed Decision dla SOL / Tier B: odśwież cenę USD.  
- Nie używaj Price jako jedynego sygnału buy — tylko input do stref A/B/C.  
- Mapowanie mintów: **`config/mints.yaml`**.

### Mints startowe

| Symbol | Mint |
|--------|------|
| SOL (wSOL) | `So11111111111111111111111111111111111111112` |
| USDC | `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v` |

Inne (BTC wrapped, GLD token, TSLAx…) — tylko po wpisie do watchlisty + mints config.

Docs: https://developers.jup.ag/docs/price

---

## 2) Swap API V2 — spot natychmiastowy (Portfolio; Decision rzadko)

Base: `https://api.jup.ag/swap/v2`

| Method | Path | Allow | Użycie |
|--------|------|-------|--------|
| `GET` | `/order` | **TAK** | Quote + unsigned tx (Ultra-style routing) |
| `POST` | `/execute` | **TAK** (live) | Landing podpisanej tx z `/order` |
| `GET` | `/build` | **Opcjonalnie** | Tylko gdy trzeba custom instructions; nie default |

**Kiedy wolno (policy):**
- `sell` / exit pilny  
- mały rebalance cash↔SOL poza limit flow  
- **NIE** jako domyślne łapanie dipu (dip = Trigger)

**Buy the Dip mapping:** nie tu — patrz Trigger.

Docs: https://developers.jup.ag/docs/api-reference/swap/order

---

## 3) Trigger API V2 — Limit / Buy the Dip / TP / DCA (główne)

Base: `https://api.jup.ag/trigger/v2`  
Wszystkie: `x-api-key` + (po auth) JWT.

### 3.1 Auth / vault / deposit

Base auth (aktualne docs):

| Method | Path | Allow | Kto |
|--------|------|-------|-----|
| `POST` | `/auth/challenge` | **TAK** | Portfolio — `npm run trigger:challenge` |
| `POST` | `/auth/verify` | **TAK** | Portfolio — po podpisie wallet (`authMode: access_refresh`) |
| `POST` | `/auth/refresh` | **TAK** | Portfolio — odśwież access token |
| `GET` | `/vault` | **TAK** | Portfolio (read) |
| `GET` | `/vault/register` | **TAK** (raz) | Portfolio live |
| `POST` | `/deposit/craft` | **TAK** | Portfolio → unsigned tx + `requestId` (jeszcze nie w CLI write) |

**Kod (Faza B+):** Trigger client w Dex crates/API — JWT tylko w env / lokalnym gitignore (nie w `config/dip/context/`).

**Deposit craft — Buy the Dip (USDC → SOL):**
- `inputMint` = USDC  
- `outputMint` = SOL  
- `orderType` = `price`  
- `orderSubType` = `single`  
- `amount` ≥ równowartość **10 USD**

### 3.2 Price orders (limit)

| Method | Path | Allow | Użycie |
|--------|------|-------|--------|
| `POST` | `/orders/price` | **TAK** | Create: Buy the Dip / TP / SL single; OCO/OTOCO gdy strategia włączy |
| `GET` | `/orders/history` | **TAK** | Open / past — **obowiązkowe** przed Decision |
| `PATCH` | `/orders/price/{id}` | **TAK** | Zmiana trigger / slippage |
| `POST` | `/orders/price/cancel/{id}` | **TAK** | Start cancel |
| `POST` | `/orders/price/confirm-cancel/{id}` | **TAK** | Po podpisaniu withdraw |

### Buy the Dip — pola create (single)

| Field | Wartość typowa |
|-------|----------------|
| `orderType` | `single` |
| `triggerMint` | mint kupowanego assetu (np. SOL) |
| `triggerCondition` | `below` |
| `triggerPriceUsd` | strefa B/C (np. 90.5, 89) |
| `inputMint` | USDC |
| `outputMint` | SOL |
| `expiresAt` | now + expiry z `runtime.yaml` (ms) |
| `slippageBps` | ostrożnie; buy-below często auto/RTSE w produkcie — trzymaj się docs |

### Take Profit (sprzedaż SOL → USDC)

- `inputMint` = SOL, `outputMint` = USDC  
- `triggerMint` = SOL  
- `triggerCondition` = `above`  
- `triggerPriceUsd` = TP1/TP2  

### Explicit Trigger deny (v1)

| Feature | Status |
|---------|--------|
| `orderType: otoco` | **OFF** dopóki strategia nie włączy |
| `trailingBps` | **OFF** (v1) |
| DCA `price_conditional` | **ON** tylko gdy Decision wybierze DCA i `dca_allowed: true` |
| Trigger V1 `/trigger/v1` | **OFF** — nie używać w nowym kodzie |
| Recurring V1 `/recurring/v1` | **OFF** — unmaintained |

### 3.3 DCA (opcjonalnie)

| Method | Path | Allow |
|--------|------|-------|
| `POST` | `/orders/dca` | **TAK** jeśli `runtime.jupiter.dca_allowed` |
| `GET` | `/orders/history/dca` | **TAK** |
| `POST` | `/orders/dca/cancel/{id}` + confirm | **TAK** |

Docs:  
- Overview https://developers.jup.ag/docs/trigger  
- Flow https://developers.jup.ag/docs/trigger/lifecycle  
- Deposit https://developers.jup.ag/docs/trigger/deposit  
- Create https://developers.jup.ag/docs/trigger/create-order  

---

## 4) Poza allowlistą (nie wołać)

- Jupiter Perps / YOLO HTTP APIs (jeśli odkryte)  
- Jakiekolwiek „transfer to random vault address” poza flow `/deposit/craft`  
- Scraping HTML jup.ag zamiast API (wyjątek: awaria API + tryb manual)  
- Orca LP program calls — osobny doc później; **nie** mieszać z Trigger bez `adjust_lp` decision  

---

## 5) Mapowanie Decision → wywołania Portfolio

| Decision `action` | Portfolio API sequence |
|-------------------|-------------------------|
| `buy_dip` | Price → (live) auth/vault → `deposit/craft` → sign → `POST /orders/price` (`below`) → update `portfolio-state` + `trade-log` |
| `sell` (limit TP) | j.w. z `above` i odwrotnymi mintami |
| `sell` (market) | `GET /swap/v2/order` → sign → `POST /execute` |
| `hold` | `GET /orders/history` + Price (read-only) |
| `cancel_order` | cancel + confirm-cancel → Withdraw pending |
| `adjust_lp` | **nie Trigger** — Orca path (TBD); oznacz blocked jeśli brak implementacji |
| `skip` | brak write |

W `propose-only`: Portfolio zwraca **gotowe JSON body + checklistę podpisu**, bez broadcast.

---

## 6) Minimalny cykl Analysis (dane z API)

Przed Decision Portfolio (lub Decision z toolami) zbiera:

1. `usdPrice` SOL (i innych aktywnych theses)  
2. Listę open Trigger orders (triggry, size, expiry)  
3. (Opcjonalnie) quote Swap bez taker — sanity płynności  

Derivatives / ETF **nie** są w Jupiter API — brać z zewnętrznych źródeł wg DECISION-FRAMEWORK lub oznaczyć `unavailable`.

---

## 7) Błędy / retry

| Sytuacja | Zachowanie agenta |
|----------|-------------------|
| 401/403 API key / JWT | Stop write; poproś o re-auth; nie loop |
| 400 order &lt; 10 USD | Podnieś size lub skip |
| Partial fill | Update portfolio-state; nie zakładaj full |
| Pending Withdraw po cancel | Dopnij confirm/withdraw zanim nowe buy |
| Docs ≠ ten plik | Aktualizuj ten plik; zaufaj developers.jup.ag |

---

## 8) Changelog pliku

| Data | Zmiana |
|------|--------|
| 2026-11-04 | v1 allowlista: Price V3, Swap V2, Trigger V2 (price+dca), deny V1/perps |
| 2026-11-04 | auth paths `/auth/challenge|verify|refresh`; kod `src/trigger` read+propose |
