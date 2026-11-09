# Strategia: Dip rotation (v1)

## Cel

Kupować przeceny na instrumentach z `config/dip/context/watchlist.md`, sprzedawać według poziomów / invalidacji; nie ograniczać się mentalnie do jednej klasy, ale **nie rozpraszać kapitału**.

## Entry (buy dip)

1. Asset musi być na watchliście Tier A (auto) lub Tier B (tylko z potwierdzeniem size ≤ max).  
2. Preferuj **Jupiter Buy the Dip** ze schodkami (2–3 triggery), nie jeden all-in.  
3. Nie kupuj Market w pierwszej godzinie po big headline (wybory, reopen CEX), chyba że strategia C (panic zone).  
4. Size: `runtime.limits.max_single_entry_usd` i zachowaj `min_cash_buffer_usd`.

### Schodki SOL (baseline z rozmowy)

| Strefa | Trigger | Rola |
|--------|---------|------|
| A | 91.5–93.5 | opcjonalny mały starter |
| B | 88–90.5 | główne dokupienie |
| C | 84–87 | panic add — nie obowiązkowe |

## Hold / LP

- Orca LP 89–110: trzymaj, zbieraj fee, **nie dokładaj** do LP po cenie powyżej średniej wejścia bez nowej tezy.  
- Alert out-of-range: zaproponuj Withdraw / nowy range.

## Exit (sell)

- TP1 orientacyjnie: powrót do pre-shock range (dla SOL ~100–103).  
- TP2 / górka range LP (~108–110): redukcja / pełne wyjście z części.  
- Invalidacja short-term: dzienne zamknięcie głęboko poniżej strefy C bez bounce → stop dokładek, przegląd tezy.  
- Preferuj Take Profit limit; Market gdy trzeba wyjść z ryzyka venue/LP.

## Multi-asset rotation

- Max 2 aktywne dip theses poza cash.  
- Nowa klasa (np. GLD): najpierw research emitenta + wpis watchlist + mały test size.  
- Nie sprzedawaj SOL tylko po to, by „mieć cash na złoto”, jeśli SOL jest w planie schodków.

## Headline rules

- Wypłaty CEX / PoR / exit polls → **vol**, nie automatyczny long.  
- Dem/GOP: handluj **niespodziankę vs wycena**, nie narrację BitHub samą w sobie.  
- ETF flows + risk assets (Nasdaq) ważniejsze dla BTC niż sam midterm jeśli wynik zgodny z rynkiem.
