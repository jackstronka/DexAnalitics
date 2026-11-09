# Jupiter — co wolno / z czego korzystać

Oficjalne docs (UI/produkt): https://docs.jup.ag/  
Developer API (agent): **`doc/dip-agent/jupiter-API.md`** + https://developers.jup.ag/  
Limit orders: https://docs.jup.ag/user-docs/trade/spot/limit-orders  
Tokenized stocks: https://docs.jup.ag/user-docs/trade/spot/tokenized-stocks  
Spend/Card: https://docs.jup.ag/user-docs/global/spend  

Dla automatyzacji preferuj **API.md** (Trigger V2 / Swap V2 / Price) zamiast klikać UI.  

## Allowlista produktów (v1)

| Produkt | Użycie agenta | Kiedy |
|---------|---------------|-------|
| **Spot Market / Ultra** | exit pilny, mały rebalance | rzadko; nie do „łapania dipu” |
| **Limit V2 — Buy the Dip** | **główne wejście** | trigger USD poniżej rynku |
| **Limit V2 — Take Profit** | wyjście z zysku | po fillu dip / redukcja |
| **Limit V2 — Stop Loss** | ochrona | tylko jeśli user włączy w strategii |
| **DCA** | budowa pozycji w czasie | gdy brak ostrego dipu, tier A |
| **Stocks / Commodities screener** | research + mały dip tier B | tylko watchlist |
| **Portfolio / Orders UI paths** | odczyt open orders | zawsze przed decyzją |
| **Lend** | idle USDC | dopiero gdy `lend_for_idle_cash: true` |
| **Bridge** | funding z innej chain | ręcznie / na życzenie usera |

## Explicit deny (v1)

- Perps  
- YOLO  
- Predict (chyba że user zmieni regułę)  
- Gacha  
- Card/Spend jako miejsce trzymania kapitału tradingowego  
- Wysyłanie środków na nieznane vault address ręcznie (tylko flow UI Limit)

## Ścieżki UI (orientacja)

- Limit: https://jup.ag/limit — zakładki Market \| **Limit** \| DCA na boxie swap  
- Buy the Dip: na formularzu Limit kliknij typ zlecenia → sekcja **Buy** → **Buy the Dip**  
- Open orders: poniżej widgetu Limit **lub** Portfolio (prawy górny) → **Limit**  
- Stocks: menu Stocks / `jup.ag/spot/stocks` — filtr Equities / ETFs / Commodities  

## Reguły egzekucji Limit V2 (skrót z docs)

- Trigger = **USD price** (lub mcap), nie para pool rate.  
- Środki idą do **vault** Jupitera na czas trwania orderu — w portfolio-state oznaczaj „locked in limit vault”.  
- Output amount **nie jest gwarantowany** jak na CLOB; slippage ma znaczenie.  
- Expiry: preferuj `7d` / `30d` z runtime.yaml.  
- Po Cancel może być **Pending Withdraw** — dopnij Withdraw.

## Orca LP (venue pomocniczy)

- Dozwolone gdy `venues.lp_allowed: orca`.  
- Range order / custom range ≠ klasyczny limit fill.  
- Po wyjściu z range agent **musi** zaproponować Withdraw lub re-range — nie „zostaw i zapomnij”.

## Hyperliquid

- Domyślnie **wyłączone** (`hyperliquid: false`).  
- Stock/commodity tam = zwykle **perpy**, nie spot ownership.
