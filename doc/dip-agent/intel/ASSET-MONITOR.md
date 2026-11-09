# Monitoring aktywów Jupiter — architektura

Cel: **ciągle** obserwować sensowne uniwersum na Jupiterze, wybierać kandydatów do dip-buy, dokładać newsy do **trwałego kompendium per asset**, a nie szukać od zera.

Powiązane:
- Źródła światowe: `NEWS-SOURCES.md` / `NEWS-ANALYSIS.md`
- Wybór: `ASSET-SELECTION.md`
- Dzienniki: `config/dip/context/assets/{ID}/`
- Usługa: `doc/dip-agent/ops-ALWAYS-ON.md` + `(legacy Node daemon — nie produkcja; Faza B+ = API Dex)`

---

## 1. Problem skali

Na Jupiterze są tysiące tokenów. **Nie monitorujemy „wszystkiego” minutowo.**

Inteligentny model = **lejek 3 warstw**:

```
Universe (szeroki, tani skan)
    → Candidates (średni, co N godzin)
        → Active theses (wąski, ciągły + pełny journal)
```

| Warstwa | Ile | Co robimy | Jak często |
|---------|-----|-----------|------------|
| **Universe** | setki → filtrowane do ~50–150 | snapshot cena/wolumen/płynność, anomaly flag | 1–6h |
| **Candidates** | max ~10–20 | scoring + krótki news pass | 6–12h lub po anomaly |
| **Active** | max **2–5** (watchlist Tier A+B aktywne) | pełny journal, intel, strefy, limity | ciągły / co 15–60 min |

Decision Agent handluje **tylko Active** (i Tier A z `watchlist.md`).  
Promotion Universe→Candidates→Active wymaga scoringu z `ASSET-SELECTION.md`.

---

## 2. Skąd brać uniwersum (Jupiter)

Źródła danych (kolejność preferencji):

1. **Nasze seed listy** — `config/universe.yaml` (crypto majors, wybrane stocks/commodities tokens).  
2. **Jupiter Spot Discover / screener** (docs Spot Discover) — organic / volume / liquidity filtry.  
3. **Tokenized stocks screener** — Equities / ETFs / Commodities (tylko gdy `stocks_allowed` / `commodities_allowed`).  
4. Price API dla mintów z listy.

Zakaz auto-add: memecoiny bez min. płynności, Shift 2x/3x, illiquid RFQ-only poza godzinami bez ręcznego OK.

---

## 3. Co mierzymy przy skanie (features)

Dla każdego assetu w Universe (gdy dostępne):

| Feature | Po co |
|---------|--------|
| `usdPrice` | poziom vs strefy / 7d/30d range |
| `% vs 7d high` (drawdown) | „przecena” |
| `volume_24h` / zmiana vol | zainteresowanie |
| `liquidity` (lub RFQ flag) | czy w ogóle da się wejść limitem |
| `priceChange24h` | momentum vs mean-reversion |
| `tier` / `asset_class` | crypto / equity_token / commodity_token |
| `anomaly` | dump ≥X%, vol spike, depeg vs underlying (dla tokenized) |

Snapshoty: `config/dip/context/monitor/snapshots/YYYY-MM-DD.jsonl` (append).  
Nie trzymać milionów świec — tylko agregaty + anomaly eventy.

---

## 4. News per asset (kompendium)

Każdy asset w Candidates/Active ma folder:

```
config/dip/context/assets/{ID}/
  meta.yaml      # mint, class, zones, status, last_scan
  thesis.md      # dlaczego w ogóle na radarze
  journal.md     # chronologiczny log (cena + decyzje)
  news.md        # KOMPENDIUM newsów — TYLKO append
```

**Zasada złota:** przed WebSearch/XSearch dla assetu **najpierw przeczytaj `news.md`**.  
Nowe findingi dopisuj na górę / z datą — nie nadpisuj historii.

Pipeline gdy asset wchodzi na Candidates lub jest Active:
1. Read `news.md` (jeśli istnieje).  
2. Query: ostatnie 24–72h (quick) lub 7d (promotion).  
3. Dedup po URL/tytule.  
4. Append wpisów w formacie NEWS-ANALYSIS.  
5. Zaktualizuj `meta.yaml` → `last_news_at`.

ID folderu: `SOL`, `USDC`, `TSLAx`, `GLDx` — stabilny symbol z `mints.yaml` / meta.

---

## 5. Pętla daemona (logika)

```
loop forever:
  1) heartbeat + load runtime
  2) world intel quick (P1) → news-digest
  3) universe price scan → snapshots + anomalies
  4) rescore → refresh candidates
  5) for each active|new candidate: news refresh if due
  6) if trade cycle due: Decision propose-only (default)
  7) sleep until next tick
```

Ticki konfigurowalne w `config/dip/runtime.yaml` → sekcja `monitor` / `daemon`.

---

## 6. Output dla Decision

Plik roboczy: `config/dip/context/monitor/candidates.md` (top N + score + link do asset journal).  
Decision **musi** czytać to + Active journals przed buy poza Tier A core.

---

## 7. Czego nie robimy

- Scraping całej Jupiter bez limitu rate  
- Auto-live trades z daemona bez `mode: live` + signer policy  
- Kasowanie `news.md`  
- Traktowanie tokenized stock news jak „prawdziwa akcja SEC filing” bez tagu issuer
