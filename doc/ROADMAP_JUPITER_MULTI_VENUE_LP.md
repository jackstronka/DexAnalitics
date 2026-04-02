# Roadmap: hipoteza Jupiter → routing → fee na wielu venue (CLMM)

**Status:** założenia robocze do weryfikacji danymi (on-chain + lokalne logi), nie twarde twierdzenie.  
**Data zapisu:** 2026-04-01.

## Kontekst (co jest sensowne)

- **Jupiter** agreguje płynność z wielu źródeł (m.in. AMM/CLMM/DLMM na Raydium, Orca, Meteora itd.) i **wybiera trasę (lub split) swapu** tak, aby zrealizować zamianę np. SOL/USDC po jak najlepszej **egzekucji dla użytkownika** (cena po uwzględnieniu price impactu, opłat protokołowych po stronie trasy, itp.).
- **Różne poole / venue mają różne parametry** (m.in. opłata dla LPerów, głębokość, typ krzywej, koncentracja), więc **ten sam par** może „konkurować” pod kątem tego, skąd Jupiter weźmie płynność.

## Założenie produktowe (do potwierdzenia)

1. W **dniach lub okresach o niskiej ogólnej płynności** na danym rynku, **większość wolumenu może być absorbowana** przez poole o **niższym „oprocentowaniu”** (niższy fee tier / inna struktura nagrody) — bo przy cienkim rynku **tańsza trasa** częściej wygrywa w routerze.
2. W takiej sytuacji **Jupiter będzie kierował swapy** częściej tam, gdzie **dla swapującego** wychodzi optymalniej — co **nie musi** oznaczać maksymalnych przychodów dla LP po stronie każdego pojedynczego poola, ale tworzy **hipotezę o przesunięciu wolumenu między venue**.
3. **Strategia operacyjna (cel):** utrzymywać **zakresy płynności (CLMM)** i **przenosić kapitał / rebalance** w stronę **projektu / poola / pasma**, gdzie **największy zysk z fee** wynika z **wolumenu faktycznie tam skierowanego** (pośrednio przez routing agregatora), zamiast sztywnego trzymania się jednego venue bez obserwacji routingu i realized fees.

## Co należy jawnie zweryfikować (żeby nie przeinaczyć modelu)

- Router (Jupiter) **nie maksymalizuje fee LP** — maksymalizuje (w uproszczeniu) **wynik dla swapującego**. Związek „niski fee tier ⇒ więcej wolumenu w suchy dzień” jest **hipotezą empiryczną**, nie prawem.
- Przy niskiej płynności mogą się pojawić **split routes**, **większy price impact**, **preferencja głębszych pooli** — trzeba mierzyć **per venue** (np. udział wolumenu / fee z dekodowanych swapów / snapshotów), a nie tylko intuicję.
- **Meteora DLMM vs Orca Whirlpool vs Raydium** — różna mechanika i eventy; w tym repo i tak opieramy się na **on-chain + lokalnych plikach** i jawnej jakości danych (por. `doc/ONCHAIN_FEES_TRUTH_PLAN.md`, reguły workspace o darmowych danych).

## Następne kroki (kiedy będzie priorytet)

1. Zdefiniować **metryki**: udział wolumenu SOL/USDC (lub wybranego pary) **per venue / per pool** w czasie; **fee zbierane** (proxy vs on-chain truth) w tym samym oknie.
2. Oznaczyć **regimy rynku** (np. niski TVL / wysoki spread / niski wolumen) i sprawdzić, czy **routing faktycznie** przesuwa się zgodnie z hipotezą.
3. Spiąć z **decyzją bota**: progi rebalance, koszt tx vs oczekiwany przyrost fee — spójnie z istniejącym modelem kosztów (por. runbooki i backtest w `doc/`).

## Powiązane dokumenty

- [`TODO_ONCHAIN_NEXT_STEPS.md`](TODO_ONCHAIN_NEXT_STEPS.md) — kolejka prac on-chain.
- [`BACKTEST_OPTIMIZE_STRATEGIES.md`](BACKTEST_OPTIMIZE_STRATEGIES.md) — semantyka backtestu / optymalizacji.
- [`PROJECT_OVERVIEW.md`](PROJECT_OVERVIEW.md) — architektura danych i crate’ów.
