# whETH/SOL — 2 strategie, ~10 USD deploy, zakres ~25–25,5 SOL/whETH

## Cel operacyjny

- **Dwie** pozycje (NFT), **dwie** strategie (`orca-bot-run` / `orca-bot-open-and-run` z różnym `--optimize-result-json`).
- **~10 USD** łącznie na depozyt (plan: **~5 USD na pozycję** w heurystyce 50/50 USD na nogi SOL / whETH). Reszta salda zostaje na **opłaty sieci i rebalansów** (nie „dokładamy” całego portfela do liquidity).
- **Zakres cenowy (SOL za 1 whETH):** mniej więcej **25,0–25,5**. Ticki na puli `HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF` (mint A = SOL, mint B = whETH): **`--tick-lower=-55416`**, **`--tick-upper=-55216`** (spacing 8). Przy cenie ok. **25,324** jesteś **w pasie** (dopóki tick/cena RPC pozostają w tym przedziale).
- Pliki strategii (schema v1):  
  - `data/experiments/wheth-sol-manual-range-25-25p5/winner-A-oor_recenter.json`  
  - `data/experiments/wheth-sol-manual-range-25-25p5/winner-B-periodic.json`  
  (wariant **C / threshold** na ten eksperyment pomijamy).

**Szerszy kontekst:** [`doc/WHETH_SOL_THREE_BOTS_FIRST_RUN.md`](WHETH_SOL_THREE_BOTS_FIRST_RUN.md), [`doc/ORCA_RUNBOOK.md`](ORCA_RUNBOOK.md), [`doc/BOT_OPERATIONS_MODEL_2026-03-23.md`](BOT_OPERATIONS_MODEL_2026-03-23.md).

## Czy „monitorujemy i rebalansujemy”?

Tak — przy **`orca-bot-run`** / **`orca-bot-open-and-run`** executor **cyklicznie** odczytuje stan pozycji z RPC i **ocenia strategię** (`Hold` / `Rebalance` / …).  
Żeby **wysyłał** transakcje rebalansu, proces musi mieć **`--execute`** oraz dostęp do **`--keypair`** (albo odpowiednie env). Bez `--execute` tylko loguje decyzje (dry-run).

Ustaw **`--eval-interval-secs`** (np. 300) i **`--poll-interval-secs`** (np. 30) wg tolerancji na RPC.

## Kapitał: przykładowe capy CLI (2× ~5 USD / pozycja)

Kwoty są **górnymi limitami** depozytu w raw; dokładny mix zależy od ceny w tickach. Przelicznik z CoinGecko jest **orientacyjny** — przed openem odpal `tools/orca_wheth_sol_three_bots_plan.ps1` z **`-DeployUsd 10 -NumBots 2`** i swoim **`-Owner`**.

Przykład przy kursie z jednego odczytu (nadpisz po świeżym planie):

| Na pozycję | `--amount-a` (SOL lamporty) | `--amount-b` (whETH raw, 8 dec) |
|------------|-----------------------------|-----------------------------------|
| Jedna pozycja (~5 USD heuryst.) | ok. **30 182 301** | ok. **119 048** |
| **Razem 2 pozycje** | ok. **60 364 602** | ok. **238 096** |

**Minimum rozmiaru pozycji** na Orca może odrzucić bardzo małe kwoty — jeśli `open` się nie uda, zwiększ capy lub sprawdź [`doc/MAINNET_MIN_POSITION_SIZING.md`](MAINNET_MIN_POSITION_SIZING.md).

## Koszty pozycji i „dzienniczek”

**Źródła prawdy w repo (append-only JSONL):**

| Ścieżka / mechanizm | Co zbierać |
|---------------------|------------|
| `data/ledger/orca_position_lifecycle.jsonl` (override: `CLMM_POSITION_LIFECYCLE_LEDGER_PATH`) | Zdarzenia tx bota (`source=orca_bot`), opłaty `tx_fee_lamports`, opcjonalnie `rebalance_session_id` |
| `data/position-fee-checkpoints.jsonl` (override / `--position-fee-ledger-path`; tryb `--fee-mode position-truth`) | Checkpointy fee pozycji (Tier3) |
| `data/positions/registry.jsonl` (`CLMM_POSITION_REGISTRY_PATH`) | Otwarcia / zamknięcia pozycji |
| **`CLMM_REBALANCE_SESSION_ID`** (to samo w jednej sesji operacji) | Łączy swap + close/open + bot — sumujesz koszt całej operacji |

**Dziennik ręczny (Notion / markdown):** skopiuj tabelę z [`WHETH_SOL_THREE_BOTS_FIRST_RUN.md`](WHETH_SOL_THREE_BOTS_FIRST_RUN.md) i zmniejsz do **2 wierszy** (slot A / B), kolumny: data, strategia, position PDA, szacowany deploy USD, tx rebalansu (sygnatura), notatka.

## Skrót komend (PowerShell, ticki; **2 uruchomienia**)

Użyj **`--tick-lower=-55416`** (nie goły `-55416`). Dopasuj `--amount-a` / `--amount-b` do planu. Dla każdej pozycji inny `--optimize-result-json`.

```powershell
. .\tools\mainnet_rpc_env.ps1
$kp = "$env:USERPROFILE\.config\solana\clmm_lp_bot_mainnet.json"
# Bot A — oor_recenter
cargo run -p clmm-lp-cli --bin clmm-lp-cli -- orca-bot-open-and-run `
  --pool HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF `
  --tick-lower=-55416 --tick-upper=-55216 `
  --amount-a 30182301 --amount-b 119048 `
  --keypair $kp `
  --optimize-result-json data/experiments/wheth-sol-manual-range-25-25p5/winner-A-oor_recenter.json `
  --eval-interval-secs 300 --poll-interval-secs 30
# Bot B — periodic: ten sam open, inny plik JSON; drugi terminal; potem dopisz --execute gdy gotowy.
```

**keywords:** WHETH_SOL, two-bots, 10USD, oor_recenter, periodic, tick range, ledger, journal
