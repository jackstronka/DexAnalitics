# Analiza newsów — jak agent czyta świat

Źródła: `doc/dip-agent/intel/NEWS-SOURCES.md`  
Wynik okresowy (świat): `config/dip/context/news-digest.md`  
Wynik per asset: `config/dip/context/assets/{ID}/news.md` (kompendium — append only)  
Wpływ na WORLD: `config/dip/context/WORLD.md` (sekcja Regime)  
Monitor lejek: `doc/dip-agent/intel/ASSET-MONITOR.md`

---

## Po co

Trading bez śledzenia informacji = late do ryzyka (hack CEX, outage L1, ETF, regulacja).  
Cel: **klasyfikować** news → **wpływ na setup** → dopiero Decision (strefy nadal rządzą wejściem).

---

## Częstotliwość

| Tryb | Kiedy | Zakres źródeł |
|------|-------|----------------|
| **Szybki skan** | Każdy cykl Decision | P1 z NEWS-SOURCES (media + CEX + SOL/ETH status + security alerts) |
| **Pełny digest** | Co 6–12h lub na życzenie usera | P1 + P2 + makro + founders Tier relevant |
| **Flash** | Breaking (hack, halt, ETF surprise, war/policy bomb) | Natychmiast: primary + 2 niezależne potwierdzenia → update WORLD + Decision |

Narzędzia: XSearch / WebSearch / oficjalne announcement pages — **nie** tylko jeden portal.

---

## Per-asset news (kompendium)

Gdy news dotyczy konkretnego ID z `config/dip/context/assets/`:
1. **Najpierw** otwórz `config/dip/context/assets/{ID}/news.md`.  
2. Szukaj tylko **delty** (nowsze niż `last_news_at` w meta).  
3. Dopisz wpisy — nie nadpisuj.  
4. Zaktualizuj `meta.yaml` → `last_news_at`.  
5. Jeśli brak folderu a score każe Candidates → utwórz szablon wg `config/dip/context/assets/README.md`.

Ogólny świat (nie-asset) → nadal `news-digest.md`.

---

## Pipeline analizy (obowiązkowy)

Dla każdego istotnego newsa wypełnij:

### 1. Capture
- Tytuł / claim  
- Źródło + URL / post  
- Czas (UTC)  
- Typ źródła: `portal | cex_official | founder | regulator | security_firm | opinion`

### 2. Verify
- Czy jest **primary** (oficjalny komunikat, on-chain, regulator)?  
- Czy ≥1 niezależne potwierdzenie przy claimach rynkowych?  
- Status: `confirmed | likely | rumor | debunked`

### 3. Classify
| Tag | Przykład |
|-----|----------|
| `cex_solvency` | Hack, wypłaty, PoR |
| `l1_outage` | Solana/ETH halt |
| `regulation` | SEC, Clarity, midterms |
| `etf_flows` | IBIT out/in |
| `macro` | Fed, DXY, Nasdaq |
| `stablecoin` | Freeze USDT/USDC |
| `listing` | Binance/Coinbase list |
| `exploit_defi` | Protocol hack |
| `narrative` | Saylor/Hayes opinia |

### 4. Bias check
- Founder/CEO → zakładaj **promocję ekosystemu**  
- Portal → sprawdź sensational headline vs body  
- Opinion → nie używaj jako „fact” do market order

### 5. Market impact hypothesis (1–4h / 1–3d)
- Kierunek: `risk-off | risk-on | asset-specific | noise`  
- Dotknięte assety z **naszej watchlisty** (SOL primary)  
- Czy już **priced-in**? (cena już ruszyła / consensus oddsy)

### 6. Action implication (dla Decision)
| Implikacja | Reguła |
|------------|--------|
| `no_trade_change` | Tylko zanotuj |
| `widen_caution` | Bez market; limity OK |
| `favor_dip_limits` | Trzymaj/schodki B/C |
| `raise_cash` | Proponuj TP / nie dokładai |
| `lp_hygiene` | Sprawdź out-of-range / venue risk |
| `flash_risk` | Skip nowych buys 0–6h poza już wiszącymi limitami |

**Zakaz:** sam headline ≠ `buy market`.

---

## Mapowanie na Decision Framework

W każdym cyklu Decision sekcja **Catalysts / News** musi zawierać:
- 0–5 bulletów z ostatniego skanu / link do `news-digest.md`  
- Najwyższy severity: `none | low | medium | high | critical`  
- `action_implication` z tabeli wyżej  

Jeśli `critical` (hack dużego CEX, L1 down, bank run stable):  
→ najpierw update `WORLD.md` + `news-digest.md`, potem Decision.

---

## Szablon wpisu w `config/dip/context/news-digest.md`

```md
### YYYY-MM-DD HH:mm UTC — [TAG]
- claim: ...
- source: ... (portal|cex|founder|...)
- status: confirmed|likely|rumor
- bias: ...
- impact: risk-off|... | assets: SOL/...
- priced_in: yes/no/unknown
- implication: no_trade_change|...
- links: ...
```

Max ~15 najnowszych wpisów w pliku; starsze → archiwum `config/dip/context/archive/news-digest-YYYY-MM.md` (gdy urośnie).

---

## Scoring wagi źródła (v1)

| Waga | Przykład |
|------|----------|
| 1.0 | Regulator, oficjalny CEX announcement, issuer freeze, status L1 |
| 0.8 | CoinDesk/The Block/Reuters + on-chain firm (TRM/Elliptic) |
| 0.5 | Founder opinion, Cointelegraph headline (wymaga confirm) |
| 0.2 | CT influencer / whale alert bez kontekstu |

Do Decision używaj claimów ze średnią wagą ≥ 0.8 **lub** dwóch źródeł ≥ 0.5 zgodnych.

---

## Rola Intel Agent

Spec: `agents/intel-agent.md`  
Output: zaktualizowany digest + krótki briefing dla Decision.
