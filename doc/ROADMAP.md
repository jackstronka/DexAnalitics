# Roadmap (produkt / strategie)

**keywords:** roadmap, position, strategy, shadow, dry_run, counterfactual, history, assignment

Ten plik zbiera **kierunki produktowe** omówione poza krótkimi wpisami w `ENGINEERING_NOTES.md`. Szczegółowe roadmapy domenowe: np. [`ROADMAP_JUPITER_MULTI_VENUE_LP.md`](ROADMAP_JUPITER_MULTI_VENUE_LP.md).

**Plan implementacji (fazy, pliki, API):** [`IMPLEMENTATION_PLAN_BOLLINGER_CANDLE_STRATEGIES.md`](IMPLEMENTATION_PLAN_BOLLINGER_CANDLE_STRATEGIES.md) — strategie Bollinger i „ostatnia świeca”.

---

## Strategie CLMM: pasma Bollingera oraz kotwica na ostatniej świecy

**Cel:** Rozszerzyć `backtest` / `backtest-optimize` (siatka strategii), **API + dashboard** oraz **decyzje bota** o dwie nowe semantyki rebalance’u oparte na cenie, z jawnym rozdzieleniem **częstotliwości pomiaru** vs **częstotliwości rebalance’u** tam, gdzie to ma sens.

### 1) Zakres jako pasma Bollingera (BB)

- **Idea:** Środek pasma LP ≈ średnia krocząca (SMA) z cen zamknięcia; górna/dolna granica ≈ środek ± \(K\) odchyleń standardowych na oknie \(N\) świec (w tej samej jednostce czasu co symulacja / feed).
- **Rebalance:** Co ustalony interwał czasu (niezależny od \(N\), chyba że użytkownik celowo ustawi tak samo) — **przeliczenie BB** i **przesunięcie zakresu CLMM** tak, by odpowiadał aktualnym górnemu/dolnemu pasmu (zwykle z zachowaniem tej samej logiki „recenter” co w `periodic` / `threshold` po stronie symulacji).
- **Parametry produktowe (szkic):** okno \(N\), mnożnik \(K\), interwał ewaluacji/rebalance’u (np. godziny lub kroki ścieżki cenowej), ewentualnie minimalny odstęp między tx (`min_rebalance_interval`).

### 2) Kotwica na ostatniej świecy (Last closed candle)

- **Idea:** „Ostatnia cena” = **close** (lub uzgodniona reguła, np. HL/2) **ostatniej zamkniętej świecy** na wybranym interwale: **15m / 30m / 1h** (lista rozszerzalna).
- **Rebalance:** Osobny parametr — co ile czasu bot **może** zrobić tx (np. co 1h), podczas gdy **świeca referencyjna** jest np. 15m — wtedy co godzinę bierzemy **ostatnią zamkniętą** 15m świecę względem zegara i ustawiamy nowe pasmo wokół tej kotwicy (szerokość jak w `range_width_pct` lub osobny preset).
- **Semantyka:** jawne pola: `reference_candle_interval` vs `rebalance_interval` (oba w sekundach lub w jednostce zgodnej z resztą stacku), walidacja „rebalance ≥ candle” opcjonalna (często nie — użytkownik może świadomie rzadziej rebalance’ować).

### Integracja

| Warstwa | Wymaganie |
| -------- | ---------- |
| **Symulacja / CLI** | Nowe warianty w `StratConfig` + logika w `backtest_engine::run_single` (lub odpowiednik), etykiety w `parse_strategy_label`, wpis w [`BACKTEST_OPTIMIZE_STRATEGIES.md`](BACKTEST_OPTIMIZE_STRATEGIES.md). |
| **API** | `StrategyType` + pola w `StrategyParameters` (OpenAPI). |
| **Web** | `StrategyType` w `web/src/lib/api.ts`, formularz (`StrategyCreate` / `StrategyEdit`, `strategyFormShared`). |
| **Execution** | `StrategyMode` + `DecisionEngine` + ewentualnie źródło OHLC zgodne z założeniem „dane za darmo” (snapshots → agregacja świec; patrz plan). |

Szczegóły faz i ryzyk danych: [`IMPLEMENTATION_PLAN_BOLLINGER_CANDLE_STRATEGIES.md`](IMPLEMENTATION_PLAN_BOLLINGER_CANDLE_STRATEGIES.md).

---

## Wiele strategii na jednej pozycji: jedna „live”, reszta shadow (symulacja)

**Idea:** Do **tej samej pozycji** (ten sam kapitał / ten sam NFT LP) można powiązać **wiele strategii**. Jedna strategia wykonuje transakcje **na serio** (real); pozostałe działają jak **„co by było, gdyby to była strategia aktywna”** — bez wysyłania tx, tylko liczenie decyzji i hipotetycznych skutków w tych samych warunkach rynkowych.

**Przykład:** 10 strategii przypiętych do pozycji → **1 real** + **9 shadow** (fikcyjne przebiegi), odświeżane np. co **5–10 minut** (nie musi być tick-by-tick; przyrost danych i tak bywa okresowy).

**Wartość:**

- Porównanie w czasie, który wariant zachowania lepiej pasuje do rynku (wg uzgodnionych metryk: fee, IL, liczba rebalansów, drawdown itd.).
- Możliwość **ręcznej zmiany** strategii „live” na taką, która lepiej wygląda w shadow — docelowo opcjonalna **automatyzacja** tego przełączenia (na koniec, jako osobny etap).

**Relacja do obecnego `dry_run` w kodzie:** dzisiejsza flaga jest **globalna per strategia** (cały executor bez tx albo z tx). Ten roadmap opisuje **osobną warstwę**: wiele strategii na jednej pozycji z jawną rolą **live vs shadow**, a nie tylko jeden globalny dry run.

---

## Wymaganie: historia przy zmianie przypisania strategii

**Jeśli strategie przypisane do pozycji będą się zmieniały** (np. zmiana „live”, dodanie/usunięcie shadow, migracja na inną strategię), **trzeba zachować historię poprzedniego przypisania** do:

- dalszych **obliczeń porównawczych** (ciągłość serii shadow vs live),
- audytu („kiedy i z jakiej strategii przeszliśmy na inną”),
- ewentualnych **backtestów / raportów** z okresu, gdy obowiązywała poprzednia konfiguracja.

**Implikacja projektowa:** model danych powinien przewidywać **wersjonowanie lub append-only log** powiązań pozycja ↔ strategia ↔ rola (live/shadow) ↔ zakres czasowy, zamiast nadpisywać jedno pole bez śladu.

---

## Fazy (propozycja)

| Faza | Zakres |
| ---- | ------ |
| **MVP** | Jedna pozycja, **2** strategie (1 live + 1 shadow), wspólny zegar odświeżania, podstawowy raport różnic. |
| **Rozszerzenie** | N strategii shadow, metryki i UI porównawcze. |
| **Operacja** | Bezpieczna zmiana strategii live + **zachowana historia** (powyżej). |
| **Opcjonalnie** | Reguły automatycznego przełączenia „live” po progach (ostrożnie: ryzyko nadmiaru tx / błędnej heurystyki). |

---

*Dopisywać tu kolejne pozycje roadmapy produktowej lub linkować do osobnych plików `doc/ROADMAP_*.md`.*
