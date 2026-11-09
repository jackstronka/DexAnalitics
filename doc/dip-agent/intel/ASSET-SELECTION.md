# Wybór potencjalnie zyskownych aktywów

Cel: z Universe wybrać **Candidates**, potem **Active** — pod dip-buy / rotację, nie pod FOMO.

Input: snapshoty monitora + journals + world digest.  
Output: `config/dip/context/monitor/candidates.md` + ewentualna promocja do `config/dip/context/watchlist.md` (Tier B) **z potwierdzeniem usera w v1**.

---

## 1. Filtry twarde (must pass)

Asset odpada, jeśli którekolwiek:

1. Brak sensownej płynności (próg w `runtime.monitor.min_liquidity_usd`) lub chroniczne RFQ poza godzinami bez ręcznego OK.  
2. Na deny liście (Shift lev, perps, meme bez wpisu).  
3. Brak mint w `config/mints.yaml` **i** nie da się bezpiecznie dodać (unknown / scam score).  
4. Już mamy ≥ `max_active_assets` Active i asset nie jest anomaly-critical.  
5. News `critical` negatywny na sam protokół (rug / freeze całości) → quarantine, nie buy.

---

## 2. Score (0–100) — v1

| Składnik | Max pkt | Jak liczyć (prosto) |
|----------|---------|---------------------|
| **Dip depth** | 25 | Drawdown od 7d high: 5–8% → 10; 8–15% → 18; 15–25% → 25; >25% → 15 (pułapka noży) |
| **Liquidity quality** | 15 | Wysoka liq / stabilny AMM → 15; cienko → 5; RFQ only → 0–5 |
| **Volume confirmation** | 10 | Vol↑ przy dumpie (kapitulacja) → 10; martwy rynek → 2 |
| **Narrative / news** | 20 | Brak FUD lub FUD już priced + primary OK → 15–20; niejasny hack → 0–5; czysty positive catalyst niepogoniony → 10–15 |
| **Fit strategii** | 15 | Crypto Tier A / SOL beta → 15; tokenized equity/commodity z research → 8–12; random alt → 0–5 |
| **Portfolio fit** | 15 | Nie koreluje 1:1 z już max SOL → bonus dywersyfikacji 10–15; trzeci ten sam cluster → 0 |

**Próg Candidates:** score ≥ **55**  
**Próg propozycji Active / size:** score ≥ **70** + journal news świeży (<48h) lub świadomy brak newsy (`news_status: clear`)

---

## 3. Preferencje klas (rotacja)

| Klasa | Kiedy faworyzować |
|-------|-------------------|
| **SOL / majors crypto** | Domyślny core; zawsze w Active jeśli w pozycji/LP |
| **BTC (SOL mint)** | Gdy silny macro/ETF dip, SOL już „pełny” |
| **Commodity tokens (GLD/SLV)** | Risk-off / USD słaby + dip vs underlying |
| **Equity tokens** | Single-name dip + niesprzeczne news; mały size |
| **Reszta alts** | Tylko anomaly + jasna teza w `thesis.md` |

Max **2 Active theses poza cash** (zgodnie z DECISION-FRAMEWORK). SOL LP liczy się jako 1 teza SOL.

---

## 4. Promocja / degradacja

| Event | Akcja |
|-------|--------|
| Score ≥70 + dip strefa | Propozycja: dodaj limity / watchlist Tier B (user confirm v1) |
| Score <40 przez 7d | Degraduj Candidates → Universe only; journal zostaje |
| Critical bad news | `status: quarantine` w meta; zakaz buy |
| Fill + otwarta pozycja | Zostaje Active aż exit / user |

---

## 5. Output format (`candidates.md`)

```md
# Candidates — YYYY-MM-DD HH:mm UTC

| Rank | ID | Class | Score | Dip7d | Zone | News | Next action |
|------|----|-------|-------|-------|------|------|-------------|
| 1 | SOL | crypto | 78 | -9% | none | clear | hold limits |
| 2 | ... | ... | ... | ... | ... | ... | ... |

## Notes
- ...
```

---

## 6. Relacja do handlu

Selection **nie kupuje**.  
Kupuje tylko Decision wg stref + DECISION-FRAMEWORK + Jupiter API allowlist.  
Selection dostarcza **kogo w ogóle analizować głębiej**.
