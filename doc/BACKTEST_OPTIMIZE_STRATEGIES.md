# Strategie backtestu i `backtest-optimize`

Ten dokument opisuje **wszystkie warianty** `StratConfig` używane w siatce `backtest-optimize` (wspólny silnik: `crates/cli/src/backtest_engine.rs`, funkcja `run_single`).

## Cel rankingu (`--objective`) — dlaczego „BEST” ma szerokie pasmo i małe fee?

Siatka zawsze skanuje **wiele** szerokości (`--min-range-pct` … `--max-range-pct`), ale **kolejność rankingu** zależy od celu:

| Cel | Co maksymalizuje | Typowy efekt na zakres |
|-----|------------------|------------------------|
| **`vs_hodl`** (domyślny) | Nadwyżka LP vs benchmark HODL | Często **najszersze** pasmo z siatki (wysoki TIR, mała „dźwignia” vs HODL) |
| **`fees`** | Suma zebranych opłat | Częściej **węższe** pasma (więcej fee na jednostkę czasu *gdy* jesteś w zakresie i idzie wolumen) — **bez** kary za IL vs HODL w score |
| **`composite`** | `fees − α·|IL|·capital − koszty` | Kompromis fee / drag; strojenie `--alpha` |
| **`pnl`**, **`risk_adj`** | Zysk końcowy / ryzyko | Zależy od ścieżki i kosztów rebalance |

**Niskie fee w raporcie** zwykle oznacza: bardzo mały **`--lp-share`** (np. `0.0001` = 0.01% puli), krótki horyzont, lub **słaby model wolumenu** (np. Birdeye bez Dune — skala dzienna z puli). Pełniejszy model: **`--dune-swaps`**, **`--fee-source snapshots`** + lokalne `data/swaps`, lub snapshot pool fees.

## Wspólne założenia

### Szerokość zakresu (`width_pct`)

Dla każdego wiersza siatki wybierana jest **względna szerokość** pasma wokół ceny A/B:

- `half = width_pct / 2`
- początkowe granice (przy wejściu):  
  `lower_ab = center_ab × (1 − half)`, `upper_ab = center_ab × (1 + half)`  
  (czyli **multiplikatywnie**, nie „±X punktów procentowych” w USD).

Kolumny **Lower/Upper** w raporcie optimize to zwykle **ten początkowy** zakres (z pierwszego kroku / siatki), a nie pełna historia wszystkich okien po kolejnych rebalance’ach.

### Co się dzieje przy rebalance (uproszczenie)

1. Wycena pozycji po bieżącym `[lower, upper]` i `liquidity L`.
2. Odjęcie kosztu rebalance’u (`tx_cost` lub model realistyczny).
3. **Nowe granice** i nowe `L` — zależnie od strategii:
   - **Pełne przesunięcie (recenter)** — jak `Threshold`, `Periodic`, `OorRecenter`: nowe pasmo **wyśrodkowane na bieżącej cenie A/B**, ta sama względna szerokość `width_pct`.
   - **`RetouchShift`** — przesuwa się **jedna krawędź** (overflow), **szerokość pasma w A/B się zachowuje**; geometria jest inna niż recenter.

Szczegóły kosztów: `doc/ORCA_RUNBOOK.md` (sekcja o rebalance cost).

---

## Lista strategii

### 1. `static` (etykieta w tabeli: `static`)

- **Rebalance:** nigdy.
- **Użycie:** punkt odniesienia — „otwierasz raz i trzymasz” przy danym `width_pct`.

---

### 2. `oor_recenter` (etykieta: `oor_recenter`) — *nowa*

**Idea:** „pilnowanie” pasma wokół spotu **tylko przez wyjście poza zakres**: dopóki cena A/B jest **wewnątrz** `[lower, upper]`, **nie** robisz rebalance’u. Gdy cena **wyjdzie** (OOR — *out of range*), wykonujesz rebalance i **otwierasz nowe** symetryczne pasmo **wokół bieżącej ceny**, z tą samą względną szerokością co w siatce (`width_pct`).

**Różnica względem `threshold_*`:**  
`threshold` może zrebalance’ować także wtedy, gdy cena jest **jeszcze w zakresie**, ale **oddaliła się od środka pasma** o co najmniej zadany procent — wtedy wiele wierszy `threshold_2%` … `threshold_10%` może dawać **identyczne** wyniki, jeśli w praktyce wszystkie rebalance’e i tak wynikają tylko z OOR.  
`oor_recenter` **usuwa** ten drugi trigger — zostaje wyłącznie OOR + recenter.

**Kod:** `StratConfig::OorRecenter` w `backtest_engine.rs`; domyślna lista w `commands/backtest_optimize.rs::default_strategies`.  
**Test:** `oor_recenter_skips_in_range_mid_rebalance_that_threshold_fires` w `crates/cli/src/engine/tests.rs`.

---

### 3. `threshold` (etykieta: `threshold_<N>%`, np. `threshold_2%`)

**Rebalance gdy:**

1. Cena A/B **poza** `[lower, upper]` → **zawsze** rebalance (jak przy OOR), **lub**
2. Cena **w zakresie**, ale  
   `|(price_ab − mid) / mid| ≥ threshold`,  
   gdzie `mid = (lower + upper) / 2`.

**Uwaga:** liczba w nazwie (`2%`, `10%` itd.) to **próg od środka pasma** przy cenie jeszcze **w środku**, a **nie** definicja szerokości okna — szerokość to nadal `width_pct` z siatki.

Po rebalance (poza ścieżką retouch): **pełny recenter** na bieżącą cenę z tym samym `width_pct`.

---

### 4. `periodic` (etykieta: `periodic_<N>h`, np. `periodic_48h`)

- **Rebalance:** po upływie **N godzin** od ostatniego otwarcia / rebalance’u (tryb ścienny: `WallClockSeconds` na ścieżkach snapshotów).
- Po rebalance: **recenter** jak wyżej.

---

### 5. `il_limit` (etykieta zależna od parametrów, np. `il_limit_5%_grace_0`)

- **Rebalance** m.in. przy OOR lub gdy „IL‑like” vs HODL (w silniku) przekracza `max_il` (po ewentualnym `grace_steps`).
- Opcjonalnie **zamknięcie** pozycji przy `close_il` (osobna ścieżka w silniku).

Parametry CLI gridu: `--il-max-pct`, `--il-close-pct`, `--il-grace-steps`.

---

### 6. `retouch_shift` (etykieta: `retouch_shift`)

- Przy **pierwszym** (lub kolejnym, jeśli włączona hybryda) wyjściu poza zakres: **nie** pełny recenter — **przesunięcie tylko krawędzi „wyjścia”** w stronę ceny, **zachowana szerokość** pasma w A/B.
- **Koszty i licznik rebalance’ów** jak przy innych strategiach (symulacja zamknięcia + nowe okno).

**Hybryda czas + %** (domyślnie włączona w optimize): po pierwszym retouchu w epizodzie OOR kolejne retouchy wg cooldown / rearm / ruchu ceny — flagi CLI opisane w `doc/ORCA_RUNBOOK.md` (`--retouch-repeat-*`, `--retouch-repeat-off`).

---

## Planowane (niezaimplementowane)

**Koperta ±X% + sub‑pasma** (np. dyskretne kroki 0,2% / 0,5% wewnątrz szerszej koperty): wymaga nowego wariantu `StratConfig` i osobnej logiki triggerów — **nie ma** jeszcze w `backtest_engine`.

---

## FAQ: dlaczego wiele strategii ma **identyczne** wiersze?

To zwykle **nie jest błąd silnika**, tylko brak zdarzeń rebalance:

1. **Kolumna `Rebals = 0`** — pozycja nigdy się nie przeładowała; wtedy wynik zależy tylko od tego samego początkowego pasma i ścieżki cen, więc `static`, `oor_recenter`, `threshold_*` (bez OOR i bez „dokręcenia” od środka) itd. mogą wyjść **bit‑w‑bit** tak samo.
2. **Krótkie okno `--hours`** — np. przy **24h** strategie **`periodic_48h`** i **`periodic_72h`** **nie mają prawa** zrobić ani jednego rebalance’u (interwał dłuższy niż horyzont) → zachowują się jak `static`, o ile nic innego nie triggeruje.
3. **Szerokie pasmo + spokojna cena** — cena zostaje w `[lower, upper]`; `oor_recenter` i OOR‑część `threshold` nie odpalają się.

Nagłówek raportu `BACKTEST OPTIMIZE` musi pokazywać **rzeczywisty** horyzont (`last N hour(s)` vs `last N day(s)`). Wcześniejsza wersja CLI błędnie wypisywała zawsze **dni** z flagi `--days` nawet przy `--hours` — to było mylące względem sekcji `WINDOW: …` (poprawione w kodzie).

---

## Gdzie szukać w kodzie

| Element | Lokalizacja |
|--------|-------------|
| Enum strategii + logika kroków | `crates/cli/src/backtest_engine.rs` |
| Domyślny zestaw dla optimize | `crates/cli/src/commands/backtest_optimize.rs` → `default_strategies` |
| Testy zachowania | `crates/cli/src/engine/tests.rs` |

## Zobacz też

- `doc/ORCA_RUNBOOK.md` — komendy, koszty rebalance, flagi retouch; **zalecana ścieżka: snapshoty Orca** (`--price-path-source snapshots`, `--fee-source snapshots`).
- `STARTUP.md` — skrót `backtest-optimize`: przykład **najpierw ze snapshotami**, Birdeye jako alternatywa.
- `scripts/export_optimize_merged_24_48_72_full.ps1` — domyślnie snapshoty + `vs-hodl`; `-Objective fees` (lub `export_optimize_merged_24_48_72_fees_snapshots.ps1`) — ranking pod opłaty przy fee ze snapshotów; `-UseBirdeye` tylko świadomie.
