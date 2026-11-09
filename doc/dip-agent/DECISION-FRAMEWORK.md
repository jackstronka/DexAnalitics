# Decision Framework — co agent bierze pod uwagę ZA KAŻDYM RAZEM

Ten dokument jest **obowiązkowy** przed każdą decyzją `buy_dip | sell | hold | cancel_order | adjust_lp | skip`.  
Nie wolno skrócić do „cena spadła → kup”.

---

## 0. Preflight (zawsze pierwsze)

1. Odczytaj: `config/dip/context/WORLD.md`, `config/dip/context/portfolio-state.md`, `config/dip/context/watchlist.md`, `config/dip/runtime.yaml` (lub lokalną kopię `data/dip/context/`).  
2. Sprawdź `mode`: `propose-only` | `paper` | `live`.  
3. Potwierdź: cash buffer ≥ `min_cash_buffer_usd`, gas SOL wystarczy.  
4. Jeśli dane portfela są nieaktualne / TODO → **najpierw odśwież stan**, dopiero potem trade decision.

**Output preflight:** `ready: true/false` + braki.

---

## 1. Analiza — stały pakiet (checklist)

Agent **musi** wypełnić każdą sekcję (nawet jeśli wynik = „n/d / nieistotne”).

### A. Stan pozycji (portfolio)

| Pytanie | Po co |
|---------|--------|
| Co jest w LP (range, in/out, skład SOL vs USDC)? | Nie dokładać w złą stronę |
| Jakie open limity (trigger, size, expiry, vault locked)? | Unikać duplikatów / konfliktu |
| Ile wolnego USDC vs zablokowanego? | Size i buffer |
| Jaka średnia / cost basis aktywnych pozycji? | Nie „doganiać średniej” bez tezy |
| Czy zbliżamy się do out-of-range LP? | Hygiene przed nowym riskiem |

### B. Cena i struktura instrumentu (per asset z watchlisty Tier A; Tier B tylko jeśli rozważany)

| Pytanie | Po co |
|---------|--------|
| Cena teraz vs strefy A/B/C z strategii / watchlisty | Entry tylko w zdefiniowanych strefach |
| Odległość od open limitów | Hold vs adjust |
| 24h / 7d range (high-low) | Kontekst „przecena vs noise” |
| Czy jesteśmy po spike headline (1–6h)? | Unikać chase / fakeout |

### C. Mikrostruktura crypto (gdy asset = SOL/BTC/ETH lub silnie skorelowany)

| Sygnał | Jak czytać (v1) |
|--------|------------------|
| Funding | Ekstremalnie + przy dumpie = ostrożniej z long add; nie wymagane do buy w strefie B |
| OI zmiana | OI↑ + cena↑ na bounce = kruche longi; OI↓ na dumpie = deleveraging |
| Taker buy/sell | Agresywny sell przy wejściu w strefę B = OK na limit; nie market w pysk |
| Crowded retail L/S | Wysoki L/S kont ≠ automatyczny short squeeze |

Jeśli brak live danych derivatives → oznacz `derivatives: unavailable` i **nie blokuj** limit dip w Tier A strefie B/C, ale **zablokuj** agresywny market add.

### D. Flows i sentiment (BTC/SOL)

| Sygnał | Jak czytać (v1) |
|--------|------------------|
| Spot BTC ETF daily flow | Silne outflowy = gorsze tło risk-on; nie kasuje dip-buy w C, obcina size |
| Fear & Greed | Fear sprzyja dip-buy wg stref; Extreme Fear ≠ all-in |
| Dominujący narrative 24h | Hack / wybory / Fed — klasyfikuj: *priced-in* vs *surprise* |

### D2. World news / intel (obowiązkowe)

Źródła: `doc/dip-agent/intel/NEWS-SOURCES.md`  
Metoda: `doc/dip-agent/intel/NEWS-ANALYSIS.md`  
Stan: `config/dip/context/news-digest.md` (+ briefing Intel)

| Pytanie | Reguła |
|---------|--------|
| Czy zrobiono szybki skan P1 od ostatniej decyzji? | Jeśli nie → uruchom Intel (quick) przed Decision |
| Severity | `critical/high` → najpierw WORLD + digest, potem Decision |
| Implication | Mapuj na `favor_dip_limits` / `widen_caution` / `flash_risk` / itd. |
| Opinion vs fact | Founder/CEO narrative ≠ fakt do market buy |

### E. Katalizatory kalendarza

| Typ | Reguła |
|-----|--------|
| Wynik wyborów / FOMC / duże PoR | 0–6h po: preferuj **hold / existing limits**, nie nowe market |
| Surprise vs consensus | Surprise bullish → nie FOMO market; Surprise bearish → wolno **schodki limit** w strefach |
| Breaking z digestu (`flash_risk`) | Jak wyżej + bez nowych buys poza już otwartymi limitami |
| Brak katalizatora | Normalny reżim stref |

### F. Multi-asset / rotacja

| Pytanie | Reguła |
|---------|--------|
| Ile aktywnych dip theses poza cash? | Max **2** |
| Czy nowy asset jest na watchliście? | Nie → skip / zaproponuj dodanie do watchlisty (nie kupuj) |
| Tier B (stocks/commodities tokens) | Tylko mały size + `requires_user_confirm: true` w v1 |
| Czy kupno X wymaga sprzedaży Y? | Domyślnie **nie** ruszaj core SOL dla FOMO innej klasy |

### G. Venue / egzekucja

| Pytanie | Reguła |
|---------|--------|
| Preferowany entry | Jupiter **Buy the Dip** |
| Market swap | Tylko exit pilny albo explicit strategy flag |
| LP adjust | Osobna decyzja `adjust_lp`, nie mylić z buy_dip |
| Deny list | Perps / YOLO / Shift lewar / HL stock perps |

---

## 2. Zasady decyzyjne (policy)

### Kupno (buy_dip)

**Wolno**, gdy WSZYSTKIE:
1. Asset na watchliście (Tier A, lub Tier B z confirm).  
2. Cena w strefie B lub C (A tylko mały starter jeśli cash pozwala).  
3. Po add: cash ≥ buffer.  
4. Size ≤ `max_single_entry_usd` i nie psuje `max_total_deployed_pct`.  
5. Brak głupiego duplikatu (już wiszą limity w tym samym poziomie ±1%).  
6. Mode pozwala na proponowanie / egzekucję.

**Forma:** 2–3 schodki limit, nie jeden all-in.  
**Zakaz:** all-in, chase po bounce ≥ oporu bez limitu niżej.

### Hold

Gdy limity już pokrywają strefę **lub** cena poza strefą **lub** preflight/katalizator każe czekać.

### Sell / Take profit

Gdy:
- osiągnięty TP1/TP2 ze strategii, **lub**
- LP przy górze range / out-of-range w stronę quote, **lub**
- invalidacja tezy (np. breakdown poniżej C bez planu dokupu), **lub**
- potrzeba przywrócenia cash buffer.

Preferuj Limit Take Profit; Market tylko przy ryzyku venue / awarii.

### Cancel / adjust order

Gdy: zmiana tezy, złe triggery, expiry blisko, konflikt schodków, potrzeba zwolnić vault USDC.

### Adjust LP

Gdy: out-of-range, range zbyt szeroki/wąski vs nowa teza, user zmienia horyzont.  
Nie dokładaj do LP „żeby uśrednić” powyżej cost bez nowej decyzji size.

### Skip

Gdy brak edge / brak danych krytycznych / deny list / limit theses.

---

## 3. Size i ryzyko (twarde)

- Max 1 entry: `runtime.limits.max_single_entry_usd`  
- Min cash: `runtime.limits.min_cash_buffer_usd`  
- Panic zone C: size ≤ 50% typowego B (chyba że user nadpisze)  
- Tier B: ≤ max z watchlisty i zawsze confirm  
- Nigdy nie używaj Card/Spend balance jako trading capital  

---

## 4. Wymagany output analizy (szablon)

Każdy cykl kończy się tak:

```md
## Analysis
### Preflight
- ready: ...
- mode: ...
- cash_buffer_ok: ...

### Portfolio
- lp: ...
- open_limits: ...
- free_usdc: ...

### Market (asset=...)
- price: ...
- zone: A|B|C|none
- 24h/7d: ...
- headline_window: yes/no
- flows/sentiment: ...
- derivatives: ... | unavailable

### Catalysts / News
- intel_scan: quick|full|flash|stale
- severity: none|low|medium|high|critical
- action_implication: ...
- bullets: (0–5) ...
- digest_ref: config/dip/context/news-digest.md

### Rotation
- active_theses: ...
- watchlist_ok: ...

## Decision
- action: ...
- venue: ...
- trigger_price / range: ...
- size_usd: ...
- rationale: ...
- risks: ...
- mode: ...
```

Bez wypełnionego **Analysis** nie wolno wydać **Decision**.

---

## 5. Częstotliwość

| Trigger cyklu | Co robić |
|---------------|----------|
| Ręczny prompt usera | Pełny framework |
| Cron / loop (gdy będzie) | Pełny framework; bez spamu tx — max 1 egzekucja live / cykl o ile limit pozwala |
| Fill / cancel event | Context update + krótki re-run Decision |
| Duży headline | WORLD update + Decision z regułą 0–6h |

---

## 6. Czego agent NIE optymalizuje (v1)

- Predykcja wyniku wyborów / „polityczny alpha” ponad strefy ceny  
- Scalping 1–2%  
- Latanie między 10 klasami aktywów  
- Maksymalizacja fee LP kosztem tezy kierunkowej  

**Cel v1:** dyscyplina dip-buy + ochrona cash + czysta egzekucja Jupiter Limit.

---

## 7. NO-GO (z mostu — `doc/dip-agent/BRIDGE.md`)

- Port całego `orca-bot-run` / CLMM executor do Node daemona dip-agenta  
- Jeden proces = live Orca rebalance **i** live Jupiter Trigger create  
- Współdzielony keypair bez osobnej policy / dry-run  
- Traktowanie backtest fee CLMM jako PnL limitu Jupiter  
- Auto-promocja memecoinów z Discover bez scoringu  
- Nadpisywanie `config/dip/context/assets/*/news.md` (append-only)  
- Market buy tylko na headline (CEO/portal) bez verify  

Gdy LP out-of-range (Orca 89–110): Decision = `adjust_lp` + odesłanie do [`LP-HYGIENE.md`](LP-HYGIENE.md) / [`ORCA_RUNBOOK.md`](../ORCA_RUNBOOK.md) — nie zgaduj rebalance w TS.
