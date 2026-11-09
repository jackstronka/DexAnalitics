# Portfolio state

> Aktualizuj po każdym fill / zmianie LP / ręcznej zmianie. Kwoty orientacyjne z rozmowy 2026-11-04.

## Snapshot

- **Data:** 2026-11-04 (CET evening)
- **Wallet:** Solana (DEX / Jupiter / Orca) — private wallet
- **Tryb agenta:** propose-only

## Pozycje

### 1) Orca LP — SOL/USDC

- Range: **89 – 110**
- Notional ≈ **$6000** (SOL kupiony ~@$94.5 za ~$4000 + ~$2000 USDC do zbalansowania range)
- Status: **in-range** (cena SOL ~94–95 w momencie setupu)
- Zachowanie: spadek → więcej SOL / mniej USDC; wzrost → więcej USDC / mniej SOL
- Poniżej 89 → praktycznie SOL; powyżej 110 → praktycznie USDC → wtedy decyzja Withdraw / re-range

### 2) Jupiter Limit — Buy the Dip (SOL)

| # | Trigger (USD) | Size (USDC) | Status |
|---|---------------|-------------|--------|
| 1 | ~90.5 (lub 90) | ~1200 | Open (potwierdź w UI) |
| 2 | ~89 | ~800 | Open (potwierdź w UI) |

### 3) Cash

- Wolne USDC poza limitami/LP: **sprawdź wallet** (plan zakładał ~2k na limity — po złożeniu limitów cash w vault Jupitera)
- Gas: utrzymaj **0.05–0.1 SOL** liquid

## Alokacja docelowa (strategia)

- Core SOL exposure: LP + filled dips
- Dry powder: limity + min buffer
- Max nowy single entry: wg `config/runtime.yaml`

## Open questions dla agenta

- [ ] Potwierdzić dokładne trigger prices i size z Jupiter Orders
- [ ] Potwierdzić aktualne salda on-chain vs vault limit orders
- [ ] Ustawić Take Profit / regułę wyjścia z LP przy ~103–110
