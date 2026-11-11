# Dip Trigger — live smoke (operator)

**Cel:** zweryfikować real Jupiter: attach → create tiny Buy-the-Dip → monitor → cancel.  
**Nie commitować** `mode: live` ani sekretów.

## Wymagania

- `JUPITER_API_KEY` w `.env`
- Host wallet (np. `testWallet1`) w `CLMM_WALLETS_DIR` / domyślnym katalogu wallets
- API na `http://127.0.0.1:8081`
- USDC (**≥ 10 USD** — twardy limit Jupitera na deposit) + trochę SOL na gas
- Lokalnie: `config/dip/runtime.yaml` → `mode: live` (tylko na czas smoke; potem `propose-only`)

Minty:

| Symbol | Mint |
|--------|------|
| SOL | `So11111111111111111111111111111111111111112` |
| USDC | `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v` |

## Kroki (PowerShell)

```powershell
$BASE = "http://127.0.0.1:8081/api/v1"
$WID = "testWallet1"   # zmień na swój wallet_id

# 0) health + wallets
Invoke-RestMethod "$BASE/health"
Invoke-RestMethod "$BASE/wallets"

# 1) attach Trigger JWT (host keypair podpisuje challenge)
#    Header `x-api-key` = JUPITER_API_KEY jeśli env procesu API go nie widzi
Invoke-RestMethod -Method POST "$BASE/dip/trigger/attach" -ContentType "application/json" -Body (@{ wallet_id = $WID } | ConvertTo-Json)

# 1b) vault (raz na wallet) — bez tego create → 403 No vault registered
Invoke-RestMethod "$BASE/dip/trigger/vault?wallet_id=$WID&register_if_missing=true"

# 2) monitor (read)
Invoke-RestMethod "$BASE/dip/snapshot?include_orders=true&wallet_id=$WID"

# 3) create tiny Buy-the-Dip (LIVE — prawdziwy depozyt do vault)
# amount = USDC smallest units (6 decimals). Minimum Jupitera ≈ 10 USDC = "10000000"
# trigger poniżej rynku — ustaw sensownie względem ceny SOL
$body = @{
  wallet_id = $WID
  confirm_execute = $true
  input_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
  output_mint = "So11111111111111111111111111111111111111112"
  amount = "10000000"
  trigger_mint = "So11111111111111111111111111111111111111112"
  trigger_condition = "below"
  trigger_price_usd = 50.0
} | ConvertTo-Json
$created = Invoke-RestMethod -Method POST "$BASE/dip/trigger/orders/price" -ContentType "application/json" -Body $body
$created

# 4) monitor — powinien pokazać role=buy_dip
Invoke-RestMethod "$BASE/dip/snapshot?include_orders=true&wallet_id=$WID"
# UI: http://127.0.0.1:3000/dip (jeśli dashboard działa)

# 5) cancel + withdraw
$cancel = @{
  wallet_id = $WID
  order_id = $created.order_id
  confirm_execute = $true
} | ConvertTo-Json
Invoke-RestMethod -Method POST "$BASE/dip/trigger/orders/price/cancel" -ContentType "application/json" -Body $cancel

# 6) po smoke: przywróć mode: propose-only w runtime.yaml
```

## Gate’y

| Endpoint | Gate |
|----------|------|
| attach / snapshot | `JUPITER_API_KEY` + keypair |
| create / cancel / patch / swap execute | `mode: live` + `confirm_execute: true` |

## Po smoke

- Przywróć `mode: propose-only`
- Zapisz wynik (OK / błąd Jupiter) w `config/dip/context/trade-log.md` (lokalnie) lub ticket
