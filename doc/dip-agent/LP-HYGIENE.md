# LP hygiene — Orca range vs Dip Agent

Dip Agent **nie** rebalansuje pozycji Orca w TypeScript. Gdy LP jest poza zakresem, Decision = `adjust_lp` + odesłanie do runbooków Dex Analytics.

## Zakres referencyjny (seed 2026-11-04)

- Pool: SOL/USDC Whirlpool  
- Range: **89 – 110** (USD / tick-mapped range z setupu)  
- Notional orientacyjny: ~$6k w LP + limity Jupiter poza LP  

Aktualny stan: `config/dip/context/portfolio-state.md` (lub lokalna kopia `data/dip/context/`).

## Zachowanie w zakresie

| Cena vs range | Skład (orientacyjnie) | Akcja dip-agenta |
|---------------|----------------------|------------------|
| W środku (np. ~95) | mieszanka SOL+USDC + fee | `hold` na LP; dip = osobne limity USDC |
| Spadek w stronę lower | więcej SOL | nie dokładać cash do tego samego szerokiego LP „żeby średnia” |
| Poniżej **89** (OOR low) | prawie 100% jednego tokenu (quote/base wg kierunku) | **`adjust_lp`** → Withdraw / re-range w CLMM UI lub `orca-bot` |
| Powyżej **110** (OOR high) | jak wyżej, druga strona | **`adjust_lp`** jak wyżej |

Dokładna semantyka OOR: [`ORCA_RUNBOOK.md`](../ORCA_RUNBOOK.md) + `DecisionEngine` modes w `crates/execution`.

## Checklist Analysis (obowiązkowe)

Przed każdym `buy_dip` / `sell`:

1. Czy LP jest **in-range**? Jeśli nie → najpierw `adjust_lp`, nie nowy size limitu „na panice”.  
2. Czy cash buffer ≥ `config/dip/runtime.yaml` `min_cash_buffer_usd`?  
3. Czy otwarte limity Jupiter nie duplikują tego samego triggera?  
4. Kapital **LP** i kapital **dip** osobno w `portfolio-state.md`.

## Co robić przy OOR

1. Decision output: `action: adjust_lp`, `venue: orca_lp`.  
2. Operator: dashboard / CLI Orca w **tym** repo (dry-run first).  
3. Po re-range: zaktualizuj `portfolio-state.md` (nowy range + notional).  
4. Dopiero potem ewentualne nowe Buy-the-Dip limity.

## NO-GO

- Zgadywanie ticków / liquidity amount w warstwie dip bez RPC/pool state z CLMM.  
- Auto-rebalance Orca z procesu Trigger.  
- Traktowanie fee LP z backtestu jako PnL wypełnionego limitu Jupiter.
