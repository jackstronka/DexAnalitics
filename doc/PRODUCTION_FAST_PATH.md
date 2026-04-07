# Produkcja — najkrótsza ścieżka (Orca bot CLI)

**Cel:** uruchomić **`orca-bot-run`** z podpisem na mainnecie z minimalnym zestawem decyzji i obserwowalnością, bez czytania całego repo.

**Pełniejsze tło:** [`MAINNET_OPERATIONAL_CHECKLIST.md`](MAINNET_OPERATIONAL_CHECKLIST.md) (klaster, dry-run, limited live), [`RPC_SOLANA_BOT_NOTES.md`](RPC_SOLANA_BOT_NOTES.md), [`OPERATIONAL_CONTINUITY.md`](OPERATIONAL_CONTINUITY.md) (restart, logi).

---

## 1. Zbuduj CLI (jednorazowo na maszynie)

```bash
make cli-release
# lub:
cargo build --release -p clmm-lp-cli
```

Skopiuj zmienne z [`.env.example`](../.env.example) (sekcja *Orca LP bot CLI*) do lokalnego `.env` — **nie commituj** kluczy.

Binary: `target/release/clmm-lp-cli` (Windows: `target\release\clmm-lp-cli.exe`).

---

## 2. Środowisko (minimum)

| Zmienna | Uwagi |
|--------|--------|
| `SOLANA_RPC_URL` | Stabilny JSON-RPC **mainnet** (publiczny = wyższy risk rate-limitów). |
| `CLMM_EXPECTED_CLUSTER=mainnet-beta` | Fail-fast przy pomyłce klastra (patrz checklist mainnet). |
| `SOLANA_KEYPAIR` / `KEYPAIR_PATH` | Klucz do podpisu — **nigdy** w repozytorium. |

Opcjonalnie: `SOLANA_RPC_FALLBACK_URLS` (ten sam klaster, po przecinku).

---

## 3. Środowisko bota (zalecane przed `--execute`)

| Zmienna | Domyślnie / sens |
|--------|-------------------|
| `CLMM_ALERT_WEBHOOK_URL` | URL na POST JSON przy alertach (range exit, IL, **rebalance incomplete**). |
| `CLMM_PENDING_OPEN_RECOVERY_PATH` | Plik kolejki recovery po incomplete; jeśli nie ustawisz, `orca-bot` z `--execute` ustawia `data/pending-open-recovery.json` (katalog musi być zapisywalny). |
| `CLMM_PENDING_OPEN_MAX_ATTEMPTS` | Max prób recovery (domyślnie 100). |
| `CLMM_REBALANCE_PROFITABILITY` | `off` \| `warn` \| `block` — na start **`off`** lub **`warn`** (heurystyka, nie księgowość). |
| `CLMM_REBALANCE_EST_TX_COST_LAMPORTS` | Szacunek kosztu tx do bramki (domyślnie `500000`). |

Szczegóły zachowań: najnowszy wpis z `keywords:` *RebalanceProfitabilityMode*, *pending_open* w [`ENGINEERING_NOTES.md`](ENGINEERING_NOTES.md).

---

## 4. Kolejność uruchomienia (zalecana)

1. **Bez podpisu:** ustaw `SOLANA_RPC_URL`, **nie** przekazuj `--execute` — obserwuj logi decyzji (dry path).
2. **Limited live:** jedna pozycja NFT, **mały** kapitał, operator dostępny, [`MAINNET_MIN_POSITION_SIZING.md`](MAINNET_MIN_POSITION_SIZING.md) jeśli nie znasz minimum puli.
3. **`--execute`:** dopiero gdy dry-run i RPC są OK.

Przykład (szkielet — dopnij `--position`, interwały, tryb fee):

```bash
# Obserwacja bez tx
cargo run --release -p clmm-lp-cli -- orca-bot-run --position <POSITION_NFT_PUBKEY> --eval-interval-secs 300 --poll-interval-secs 30

# Live (po akceptacji ryzyka)
cargo run --release -p clmm-lp-cli -- orca-bot-run --position <POSITION_NFT_PUBKEY> --execute --eval-interval-secs 300 --poll-interval-secs 30 --fee-mode heuristic
```

Opcjonalnie: `--optimize-result-json <plik>` jeśli masz wynik `backtest-optimize`.

---

## 5. Stop / awaria

- **Ctrl+C** — zatrzymanie procesu.
- Po **rebalance incomplete** (close OK, open nie): środki w portfelu; plik pending + alert webhook; bot może ponowić `open` w kolejnych cyklach — monitoruj logi `op = orca_rebalance`, `stage = recover_open`.
- Circuit breaker w executorze — przy powtarzających się błędach evaluacji sprawdź logi i RPC.

---

## 6. Co świadomie odkładasz na później

- „Idealny” RPC archival / Jito — poza minimalnym `SOLANA_RPC_URL`.
- Pełna zgodność fee backtest ↔ on-chain — projekt zakłada heurystyki; decyzje sterujące na **trendzie**, nie na dolarze co do centa (reguły workspace).
