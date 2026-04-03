# Roadmap (produkt / strategie)

**keywords:** roadmap, position, strategy, shadow, dry_run, counterfactual, history, assignment

Ten plik zbiera **kierunki produktowe** omówione poza krótkimi wpisami w `ENGINEERING_NOTES.md`. Szczegółowe roadmapy domenowe: np. [`ROADMAP_JUPITER_MULTI_VENUE_LP.md`](ROADMAP_JUPITER_MULTI_VENUE_LP.md).

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
