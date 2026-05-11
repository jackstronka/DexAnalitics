# Impermanent Loss (IL) w USD — definicje, fees, łańcuch pozycji, mapa kodu

**Cel dokumentu:** jedno miejsce powrotu przy projektowaniu „najdokładniejszego” IL dla scenariusza **ręczny open → bot (zamknięcia / reopeny) → ręczny close**, z walutą referencyjną **USD** i jasnym rozdzieleniem **IL z fees LP** vs **IL bez fees LP** (oraz: fees sieci vs fees puli).

**Słowa kluczowe (grep / AI):** `IL`, `impermanent loss`, `HODL`, `baseline`, `stream-pnl`, `position_stream_pnl`, `lineage`, `calculate_il_concentrated`, `segment`, `fees_usd`, `realized_cashflow`, `hodl_value_usd`, `BUG-20260419-01`, `LVR`, `Uniswap v3`, `arxiv`

**Powiązane:** [`STARTUP.md`](../STARTUP.md) (sekcja IL across rebalances), [`doc/BUGS.md`](BUGS.md) (stream PnL / mint columns), [`doc/ORCA_RUNBOOK.md`](ORCA_RUNBOOK.md) (`--il-ledger-path`), [`doc/PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md).

---

## 1. Intuicja produktowa

Chcesz zmierzyć **koszt koncentracji / LP vs trzymanie tokenów** na całym życiu „linii pozycji” (wiele PDA po rotacjach), nie tylko na jednym tick-range.

- **Punkt startu:** pierwszy depozyt (ręczny open) — wektor tokenów \((a_0, b_0)\) w puli o mintach znanych z baseline.
- **Środek:** bot zamyka i otwiera nowe PDA — ekonomicznie to **ciąg**, o ile umiemy powiązać PDAs (lineage / IL ledger edges).
- **Koniec:** ostatnia pozycja zamknięta ręcznie — wektor „principal” lub wypłata z close w snapshotach.

**USD** oznacza: te same ilości tokenów × **cena USD mintu** w momencie porównania (mark-to-market). Dokładność absolutna zależy od źródła cen (w repo: głównie darmowe feedy typu Jupiter / heurystyki WSOL — patrz `position_valuation`, `price_fetch`).

---

## 2. Definicje formalne (dwie wersje względem fees LP)

Oznaczenia:

- \(P_t\): wektor cen USD mintów puli w chwili \(t\) (albo „teraz” przy raporcie live).
- **Principal LP:** wartość **samej płynności** (amounty z bonding curve / readera pozycji), **bez** składnika „nagromadzone fees LP w pozycji”, jeśli ten składnik jest raportowany osobno.
- **Fees LP:** skumulowane **protocol/LP fees** przypisane do pozycji (claimable / zebrane — definicja musi być spójna z tym, co wliczasz do `current`).
- **Fees sieci (SOL):** opłaty transakcyjne w SOL → USD (osobny wątek od IL puli).

### 2.1. Wariant A — „czysty” IL (klasyczny, **bez** fees LP w obu nogach)

\[
IL^{clean}_t = V^{LP,principal}_t - V^{HODL}_t
\]

gdzie

\[
V^{HODL}_t = a_0 \cdot P^{(A)}_t + b_0 \cdot P^{(B)}_t
\]

czyli: **te same** \(a_0, b_0\) co przy pierwszym openie łańcucha, wycenione po **dzisiejszych** (lub końcowych) cenach USD.

**Interpretacja:** „Ile traciłbym na samym kształcie inventory LP vs gdybym nie handlował inventory przez krzywą puli” — **bez** kompensacji fees LP.

### 2.2. Wariant B — IL „z fees LP” (często używany do pytania „czy LP się opłacał”)

Zdefiniuj **wartość strategii LP z fees** (principal + fees LP zatrzymane w pozycji lub już zcollectowane — musisz wybrać jedną konwencję):

\[
V^{LP+fees}_t = V^{LP,principal}_t + F^{LP}_t
\]

(np. \(F^{LP}\) = niezebrane + zrealizowane fees w tokenach × \(P_t\)).

Wtedy:

\[
IL^{gross}_t = V^{LP+fees}_t - V^{HODL}_t = IL^{clean}_t + F^{LP}_t
\]

**Uwaga nazewnicza:** część ludzi nazywa to już nie „IL”, tylko **„LP vs HODL”** lub **„economic gap vs benchmark”**, bo fees LP nie są „impermanent” w tym samym sensie co krzywa — to **realny cashflow**. W UI warto **nigdy nie mieszać** liczb z wariantu A i B bez etykiety.

### 2.3. Co z fees **sieci** i **realized cashflow** z ledgera?

To **nie wchodzi** do klasycznego IL vs HODL, ale wchodzi do **Net PnL strategii**:

\[
NetPnL \approx V^{end} + Cashflow^{realized} - V^{start} - Fees^{network}
\]

(dokładna postać w API: patrz `PositionStreamPnLResponse.net_pnl_usd` w kodzie).

---

## 3. Jak to dziś jest zrobione w repozytorium (mapa ścieżek)

### 3.1. Dashboard / API — **łańcuch PDA** (`stream-pnl`, lineage)

**Plik:** `crates/api/src/services/position_stream_pnl.rs`

- Przy **lineage** `old → new`: **baseline** = snapshot startowy **pierwszego** PDA (prefer `raw_json.kind = baseline_open`), **current** = snapshot **ostatniego** PDA (prefer `end_close`).
- **HODL USD:**

  `hodl_value_usd = baseline_amount_a_ui * price_a_usd_now + baseline_amount_b_ui * price_b_usd_now`

  (minty z baseline snapshot, fallback do mintów z current).

- **IL USD (implementacja = wariant A / „principal only”):**

  `il_usd = current_value_usd - hodl_value_usd`

  gdzie `current_value_usd` pochodzi z `value_usd` snapshotu końcowego — w praktyce to **principal** z wyceny pozycji (zob. `compute_position_usd_valuation`: `value_usd` z amountów; `fees_usd` jest osobnym polem w wycenie on-chain).

- **Fees LP / collect:** trafiają głównie do **`realized_cashflow_usd`** (suma `fee_payer_token_deltas` × ceny), a potem do **`net_pnl_usd`**, a **nie** są dodawane do `il_usd` w tej ścieżce.

Czyli: **obecny stream IL w API jest bliski wariantowi A** (o ile snapshoty `value_usd` nie mieszają principal z fees — patrz implementacja wyceny).

**Znane regresje / jakość danych:** `doc/BUGS.md` → **BUG-20260419-01** (brak kolumn mint w SELECT psował HODL/IL), oraz wpis o **chain session scoping** dla fee/cashflow vs anchor lineage.

### 3.2. Domena — `calculate_il_concentrated`

**Plik:** `crates/domain/src/metrics/impermanent_loss.rs`

- Liczy **ułamek** \((V_{LP} - V_{held}) / V_{held}\) na modelu CL z **znormalizowaną płynnością** (`1e18`), dla zadanego entry price, current price i widełek ceny.
- To jest **model ilościowy w jednostkach „wartości w token B”** (skala się skraca w ilorazie), **nie** USD z portfela — USD dokładasz warstwą wyżej przez ceny mintów.

### 3.3. Backtest / symulacja — IL **segmentowe**

**Pliki:** `crates/simulation/src/position_tracker.rs`, `STARTUP.md` („IL across rebalances”).

- Po każdym rebalance: nowy segment z **nowym** `segment_entry_price` i `segment_capital`.
- IL% liczone w segmencie; wartość pozycji składa się z kapitału segmentu + IL segmentu + fees − koszty.

To jest **świadomie inna semantyka** niż „jeden HODL od pierwszego manual open”: segmenty służą do **porównywania strategii rebalance vs szeroki zakres** na tej samej ścieżce cen, a nie do replikacji pojedynczego łańcucha „jedna baza HODL na zawsze”.

### 3.4. Monitor procesu — `PnLTracker`

**Plik:** `crates/execution/src/monitor/pnl_tracker.rs`

- Używa `calculate_il_concentrated` do `il_pct`, ale **`il_usd = entry_value_usd * il_pct.abs()`** jest ** podejrzane metrologicznie** (wartość bezwzględna gubi znak; skalowanie `entry_value` × `%` z modelu znormalizowanego nie musi odpowiadać USD z rzeczywistych amountów).
- `net_pnl_usd` w komentarzu sugeruje relację z IL, ale w kodzie jest `value_change + fees` **bez** jawnego odjęcia IL — **nie traktuj tego trackera jako źródła prawdy** dla łańcucha PDAs; za **benchmark IL vs HODL w USD** preferuj **stream PnL** (sekcja 3.1).

---

## 4. Checklist „czy liczę to, co myślę?”

| Pytanie | Wariant A (czysty IL) | Wariant B (IL + fees LP) |
|--------|------------------------|---------------------------|
| Czy `current` zawiera tylko principal? | Tak — wymagane | Można świadomie dodać fees |
| Czy HODL używa \(a_0,b_0\) z **pierwszego** opena łańcucha? | Tak | Tak |
| Czy ceny USD są te same dla LP i HODL w danym momencie? | Zalecane | Zalecane |
| Czy fees **sieci** są w IL? | Nie | Nie |
| Czy **collect** zmienia interpretację? | Nie zmienia \(V^{HODL}\); zmienia cashflow / wallet | Zależnie od tego, czy fees są w `current` |

---

## 5. Rekomendacja robocza dla Twojego celu (manual → bot → manual)

1. **Źródło prawdy dla „IL vs HODL od pierwszego depozytu”:** endpoint / logika oparta o **`compute_position_stream_pnl_for_stream_members`** + poprawnie zbudowane **lineage** + snapshoty `baseline_open` / `end_close` z poprawnymi **mintami i amountami UI**.
2. **Równolegle raportuj:** `net_pnl_usd` (ekonomia z cashflow) oraz opcjonalnie **`IL_B = il_usd + fees_lp_component_usd`** jeśli zdefiniujesz `fees_lp_component_usd` jednoznacznie (niezebrane + zcollectowane, albo tylko jedno).
3. **Backtest segmentowy** traktuj jako **osobną metrykę** (świetna do optymalizacji zakresów), nie jako zamiennik punktu 1 bez dodatkowej matematyki scalającej segmenty do jednego HODL.

---

## 6. Szybkie linki do kodu

| Temat | Lokalizacja |
|-------|-------------|
| IL USD łańcuch (LP − HODL) | `crates/api/src/services/position_stream_pnl.rs` (`il_usd`, `hodl_value_usd`) |
| Model IL% (CLMM) | `crates/domain/src/metrics/impermanent_loss.rs` |
| Segment IL w symulacji | `crates/simulation/src/position_tracker.rs` |
| Wycena principal/fees USD | `crates/api/src/services/position_valuation.rs` |
| Opis pól API | `crates/api/src/models.rs` (`PositionStreamPnLResponse`) |

---

## 7. Literatura i podejścia zewnętrzne (skrót)

Sekcja do **porównań** z implementacją w repo (sekcje 2–3): co mierzą inni, jakie są typowe konwencje nazewnicze i kiedy metryka przestaje nazywać się „IL” w sensie akademickim.

### 7.1. Constant product (np. Uniswap v2) — wzór na ułamek

Dla 50/50 AMM często podaje się \(r = P_{now}/P_{wejście}\) i:

\[
IL(r) = \frac{2\sqrt{r}}{1+r} - 1
\]

Materiały edukacyjne / przewodniki (intuicja + przykłady):

- [Binance Academy — Impermanent Loss Explained](https://academy.binance.com/en/articles/impermanent-loss-explained)
- [OpenLiquid — Impermanent Loss Explained](https://openliquid.io/blog/impermanent-loss-explained/)
- [Covalent — How to Calculate Impermanent Loss (with Examples)](https://www.covalenthq.com/docs/unified-api/guides/how-to-calculate-impermanent-loss-with-examples/)

**Mapowanie na repo:** `calculate_il_constant_product` w `clmm_lp_domain` — to **nie** jest automatycznie to samo co łańcuch PDA w USD (`stream-pnl`).

### 7.2. LP vs HODL w tej samej walucie (mark-to-market)

Powszechna definicja „ludzka”: wartość portfela w LP (w USD lub stable) **minus** wartość **tych samych** tokenów z wejścia przy **tych samych** cenach.

- [Quant Matter — What Is Impermanent Loss?](https://quantmatter.com/what-is-impermanent-loss/)
- [STON.fi — Impermanent loss (guide)](https://guide.ston.fi/EN/providing-liquidity/impermanent-loss)

**Mapowanie na repo:** idea zbliżona do `il_usd = current_value_usd - hodl_value_usd` w `position_stream_pnl` (HODL z koszyka startu łańcucha × bieżące ceny USD mintów).

### 7.3. Concentrated liquidity (Uniswap v3 / CLMM)

Inna krzywa niż v2; typowo większa wrażliwość na ruch ceny w wąskim zakresie i inne zachowanie poza tickami.

- [arXiv:2111.09192 — Impermanent Loss in Uniswap v3](https://arxiv.org/abs/2111.09192)
- [Medium — Impermanent Loss Calculation for Uniswap V3 (Leo Lau)](https://medium.com/@leo-lau/impermanent-loss-calculation-for-uniswap-v3-c753dcfae16d)

**Mapowanie na repo:** `calculate_il_concentrated` (model %), oraz osobno symulacja segmentowa (`position_tracker`).

### 7.4. Rebalance i „ścieżka” — segmenty vs jedna baza HODL

| Podejście zewnętrzne | Idea | Związek z repo |
|----------------------|------|----------------|
| **Segmentowe / per okno** | Po zmianie zakresu nowy punkt odniesienia dla IL lub suma efektów w kawałkach. | Jak **backtest** (`segment_entry_price`, `segment_capital`). |
| **Jedna baza od pierwszego depozytu** | Jedna para \((a_0,b_0)\) do końca łańcucha. | Jak **`stream-pnl` + lineage** (baseline pierwszego PDA). |
| **Realizacja przy rebalance** | Zmiana zakresu / swap w trakcie strategii **realizuje** ruchy inventory (koszt ścieżki). | Blogi / analiza strategii v3, np. [Amberdata — mitigating IL across Uniswap v3](https://blog.amberdata.io/strategies-for-mitigating-impermanent-loss-across-uniswap-v3). |

### 7.5. Fees LP — zwykle osobno, potem „net”

Typowa konwencja edukacyjna: **IL od dywergencji cen/inventory**, **fees od wolumenu**; sens LP = porównanie obu (np. fees ≥ IL → net dodatni możliwy). Patrz m.in. [STON.fi](https://guide.ston.fi/EN/providing-liquidity/impermanent-loss), [Quant Matter](https://quantmatter.com/what-is-impermanent-loss/).

**Mapowanie na repo:** zgodne z rozdziałem **2.1 vs 2.2** oraz z tym, że `realized_cashflow_usd` / `net_pnl_usd` są **obok** `il_usd` w `PositionStreamPnLResponse`, a nie wewnątrz jednej liczby bez etykiety.

### 7.6. LVR (Loss-Versus-Rebalancing) — inna oś niż klasyczne IL

Literatura opisuje m.in. stratę LP względem **rebalancingu po cenie rynkowej** i koszty związane z arbitrażem przy „starych” cenach w CFMM — to **nie zastępuje** wzorca „LP vs HODL z depozytu”, ale uzupełnia analizę kosztów pasywnego LP.

- [Milionovich / Zhang — LVR (PDF)](https://anthonyleezhang.github.io/pdfs/lvr.pdf)
- [arXiv:2208.06046](https://arxiv.org/abs/2208.06046) (pokrewne prace o stratach LP w CFMM)

**Mapowanie na repo:** na razie **brak** dedykowanej metryki LVR w dashboardzie; IL vs HODL w USD pozostaje benchmarkiem z sekcji 3.1.

---

## 8. Historia zmian tego dokumentu

| Data | Zmiana |
|------|--------|
| 2026-05-11 | Utworzono: definicje A/B, mapa implementacji, rozróżnienie stream vs segment vs PnLTracker. |
| 2026-05-11 | Dodano §7: literatura zewnętrzna, linki, tabela segment vs jedna baza HODL, LVR. |
