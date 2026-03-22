# CLMM Liquidity Provider (aktualny stan projektu)

Repozytorium ewoluowało: obecnie to przede wszystkim **workspace Rust** do analizy i backtestów LP na Solanie, z naciskiem na:

- lokalne snapshoty on-chain (`pool-snapshots`)
- lokalny sync strumienia swapów (`swaps` + `decoded_swaps`)
- backtesty i optymalizację zakresów dla CLMM/DLMM

Obsługiwane protokoły:

- Orca Whirlpool
- Raydium CLMM
- Meteora DLMM

## Co tu jest najważniejsze dzisiaj

- **CLI jako główny interfejs** (`crates/cli`)
- **pipeline danych lokalnych** zamiast pełnego oparcia o płatne API
- **porównywanie strategii/range'ów** na tych samych danych historycznych

## Struktura workspace

`Cargo.toml` definiuje 8 crate'ów:

- `clmm-lp-domain` - model domeny i matematyka CLMM
- `clmm-lp-simulation` - backtest i tracking pozycji
- `clmm-lp-optimization` - optymalizacja zakresów i celów
- `clmm-lp-protocols` - adaptery protokołów Solana
- `clmm-lp-execution` - logika wykonawcza/monitoring
- `clmm-lp-data` - providerzy danych i cache
- `clmm-lp-api` - REST API
- `clmm-lp-cli` - narzędzie CLI

## Szybki start

Wymagania:

- Rust (edition 2024)
- dostęp do Solana RPC
- opcjonalnie klucze API (np. `BIRDEYE_API_KEY`, `DUNE_API_KEY`) dla wybranych trybów

Budowanie i testy:

```bash
make build
make test
```

Uruchamianie CLI:

```bash
cargo run --bin clmm-lp-cli -- --help
```

## Najczęściej używane komendy CLI

### 1) Snapshoty curated pooli (Orca + Raydium + Meteora)

```bash
cargo run --bin clmm-lp-cli -- snapshot-run-curated-all
```

Opcjonalnie:

```bash
cargo run --bin clmm-lp-cli -- snapshot-run-curated-all --limit 1
```

### 2) Sync surowych swapów (P1)

```bash
cargo run --bin clmm-lp-cli -- swaps-sync-curated-all --max-signatures 300
```

### 3) Dekodowanie swapów do `decoded_swaps.jsonl` (P1.1)

```bash
cargo run --bin clmm-lp-cli -- swaps-enrich-curated-all --max-decode 120
```

Dla wolnego RPC:

```bash
cargo run --bin clmm-lp-cli -- swaps-enrich-curated-all --max-decode 120 --decode-timeout-secs 30 --decode-retries 3
```

Po poprawce dekodera — przebudowa `decoded_swaps.jsonl` z surowych sygnatur:

```bash
cargo run --bin clmm-lp-cli -- swaps-enrich-curated-all --max-decode 2000 --refresh-decoded
```

Audyt jakości dekodowania:

```bash
cargo run --bin clmm-lp-cli -- swaps-decode-audit --save-report
```

Monitoring/alerty:

```bash
cargo run --bin clmm-lp-cli -- data-health-check --max-age-minutes 30 --min-decode-ok-pct 65 --fail-on-alert
```

### 4) Backtest na danych historycznych

```bash
cargo run --bin clmm-lp-cli -- backtest \
  --symbol-a SOL \
  --mint-a So11111111111111111111111111111111111111112 \
  --lower 140 \
  --upper 180 \
  --days 30 \
  --strategy static
```

Backtest z lokalnymi snapshotami jako źródłem ścieżki ceny:

```bash
cargo run --bin clmm-lp-cli -- backtest \
  --symbol-a whETH \
  --mint-a 7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs \
  --symbol-b SOL \
  --mint-b So11111111111111111111111111111111111111112 \
  --hours 12 \
  --lower 22.89 \
  --upper 24.67 \
  --strategy static \
  --price-path-source snapshots \
  --snapshot-protocol orca \
  --snapshot-pool-address HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF \
  --fee-source snapshots
```

### 5) Backtest-optimize (grid po range + strategiach)

Bez `--dune-swaps`: jeśli podasz `--snapshot-protocol` i `--snapshot-pool-address` (lub `--whirlpool-address`), opłaty mogą być liczone z lokalnego `data/swaps` (jak w `backtest`). Opcja `--fee-swap-decode-status loose|ok`.

```bash
cargo run --bin clmm-lp-cli -- backtest-optimize \
  --symbol-a whETH \
  --mint-a 7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs \
  --symbol-b SOL \
  --mint-b So11111111111111111111111111111111111111112 \
  --days 30 \
  --capital 7000 \
  --objective vs_hodl \
  --top-n 5
```

### 6) Monte Carlo optimize

```bash
cargo run --bin clmm-lp-cli -- optimize \
  --symbol-a SOL \
  --mint-a So11111111111111111111111111111111111111112 \
  --days 30 \
  --capital 1000 \
  --iterations 100
```

## Gdzie lądują dane

- Snapshoty:
  - `data/pool-snapshots/orca/<pool>/snapshots.jsonl`
  - `data/pool-snapshots/raydium/<pool>/snapshots.jsonl`
  - `data/pool-snapshots/meteora/<pool>/snapshots.jsonl`
- Swapy:
  - `data/swaps/orca/<pool>/swaps.jsonl`
  - `data/swaps/raydium/<pool>/swaps.jsonl`
  - `data/swaps/meteora/<pool>/swaps.jsonl`
- Zdekodowane swapy:
  - `data/swaps/orca/<pool>/decoded_swaps.jsonl`
  - `data/swaps/raydium/<pool>/decoded_swaps.jsonl`
  - `data/swaps/meteora/<pool>/decoded_swaps.jsonl`

## Aktualny workflow (praktycznie)

1. Odpal cyklicznie snapshoty (`snapshot-run-curated-all`).
2. Równolegle zbieraj swapy (`swaps-sync-curated-all`).
3. Dokładaj dekodowanie (`swaps-enrich-curated-all`).
4. Uruchamiaj `backtest` / `backtest-optimize` na lokalnym cache.
5. Porównuj protokoły i strategie na tych samych oknach czasu.

## Dalsza dokumentacja

- `STARTUP.md` - procedury startowe i przykłady end-to-end
- `doc/PROJECT_OVERVIEW.md` - skrócony opis architektury i pipeline
- `doc/ONCHAIN_FEES_TRUTH_PLAN.md` - plan dojścia do bardziej "on-chain truth" dla fee accounting
- `doc/TODO_ONCHAIN_NEXT_STEPS.md` - **co jest do zrobienia** (fazy A–E) i log wykonania

`backtest` z lokalnym `decoded_swaps.jsonl`: domyślnie tylko wiersze z `decode_status=ok` (`--fee-swap-decode-status loose` = poprzednie zachowanie).

## Licencja

Dual license:

- MIT (`LICENSE-MIT`)
- Apache-2.0 (`LICENSE-APACHE`)

## Disclaimer

To narzędzie badawcze/analityczne. LP i trading na krypto wiążą się z ryzykiem. Używasz na własną odpowiedzialność.