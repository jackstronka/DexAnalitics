# Orca API Service Contract (Read + Write)

Cel: miec jeden, konkretny kontrakt integracyjny dla Orca:
- `OrcaReadService` = odczyt (REST + on-chain readers),
- `OrcaTxService` = write (on-chain tx only),
- jasny podzial: co idzie przez read API, a co przez write API,
- mapa 1:1 do obecnego kodu w repo.

## 1) Podzial odpowiedzialnosci

### Read API (off-chain / REST, bez podpisywania)
Uzywamy Orca Public API (`https://api.orca.so/v2/solana`) do:
- discovery pooli,
- filtrow i rankingu (TVL, volume, fees),
- metadata tokenow/pooli,
- statystyk i telemetry.

### Write API (on-chain / signed tx)
Wszystkie operacje zmieniajace stan pozycji ida przez:
- `WhirlpoolExecutor` (`crates/protocols/src/orca/executor.rs`),
- podpis walletem lokalnym (`Wallet`, `Keypair`),
- RPC Solana (`send_and_confirm` + simulate/preflight).

To oznacza: Orca REST nie jest wykorzystywane do modyfikacji pozycji.

## 2) OrcaReadService - kontrakt metod

Interfejs docelowy (warstwa aplikacyjna):

```rust
pub struct OrcaReadService {
    // klient HTTP do api.orca.so + fallback reader on-chain
}

impl OrcaReadService {
    pub async fn list_pools(&self, q: ListPoolsQuery) -> anyhow::Result<PagedPools>;
    pub async fn search_pools(&self, q: SearchPoolsQuery) -> anyhow::Result<PagedPools>;
    pub async fn get_pool(&self, address: &str) -> anyhow::Result<PoolDetails>;
    pub async fn get_pool_lock_info(&self, address: &str) -> anyhow::Result<Vec<LockInfo>>;

    // read on-chain (source of truth dla tx safety)
    pub async fn get_pool_state_onchain(&self, address: &str) -> anyhow::Result<WhirlpoolState>;
    pub async fn get_position_onchain(&self, address: &str) -> anyhow::Result<OnChainPosition>;
    pub async fn get_positions_by_owner_onchain(
        &self,
        owner: &str,
    ) -> anyhow::Result<Vec<OnChainPosition>>;
}
```

### Mapa endpointow REST -> metody

- `GET /pools` -> `list_pools(...)`
  - query: `sortBy`, `sortDirection`, `size`, `next`, `previous`, `token`, `tokensBothOf`, `addresses`, `minTvl`, `minVolume`, `stats`, `hasRewards`, `hasAdaptiveFee`, `includeBlocked`.
- `GET /pools/search` -> `search_pools(...)`
  - query: `q`, `size`, `next`, `minTvl`, `minVolume`, `stats`, `verifiedOnly`.
- `GET /pools/{address}` -> `get_pool(address)`
- `GET /lock/{address}` -> `get_pool_lock_info(address)`

### Mapa 1:1 do obecnego kodu (read on-chain)

- `get_pool_state_onchain` -> `WhirlpoolReader::get_pool_state`
  - plik: `crates/protocols/src/orca/pool_reader.rs`
- `get_position_onchain` -> `PositionReader::get_position`
  - plik: `crates/protocols/src/orca/position_reader.rs`
- `get_positions_by_owner_onchain` -> `PositionReader::get_positions_by_owner`
  - plik: `crates/protocols/src/orca/position_reader.rs`

## 3) OrcaTxService - kontrakt metod (write)

Interfejs docelowy:

```rust
pub struct OrcaTxService {
    // WhirlpoolExecutor + wallet resolver + preflight policy
}

impl OrcaTxService {
    pub async fn open_position(&self, req: OpenPositionRequestTx) -> anyhow::Result<TxResult>;
    pub async fn increase_liquidity(
        &self,
        req: IncreaseLiquidityRequestTx,
    ) -> anyhow::Result<TxResult>;
    pub async fn decrease_liquidity(
        &self,
        req: DecreaseLiquidityRequestTx,
    ) -> anyhow::Result<TxResult>;
    pub async fn collect_fees(&self, req: CollectFeesRequestTx) -> anyhow::Result<TxResult>;
    pub async fn close_position(&self, req: ClosePositionRequestTx) -> anyhow::Result<TxResult>;
    pub async fn simulate(&self, req: SimulateRequestTx) -> anyhow::Result<SimResult>;
}
```

### Mapa 1:1 do obecnego kodu (write)

- `open_position` -> `WhirlpoolExecutor::open_position`
- `increase_liquidity` -> `WhirlpoolExecutor::increase_liquidity`
- `decrease_liquidity` -> `WhirlpoolExecutor::decrease_liquidity`
- `collect_fees` -> `WhirlpoolExecutor::collect_fees`
- `close_position` -> `WhirlpoolExecutor::close_position`
- `simulate` -> `WhirlpoolExecutor::simulate_transaction`

Plik: `crates/protocols/src/orca/executor.rs`.

## 4) Co idzie przez read API, a co przez write API (contract)

### Read API only
- lista i wyszukiwarka pooli pod UX/selection,
- telemetry (TVL/volume/fees) do rankingow,
- lock info / warnings / metadata.

### On-chain read only
- parametry potrzebne do bezpiecznego zbudowania tx:
  - tick spacing,
  - aktualny tick/sqrt price,
  - stan pozycji (liquidity, tick range, owner),
  - konta vault/ATA/tick arrays (gdy dopinamy full metas).

### Write API only (signed tx)
- open/increase/decrease/collect/close pozycji.

Zasada:
- REST Orca jest warstwa pomocnicza i szybka.
- Source of truth dla wykonania tx = on-chain read + walidacje lokalne.

## 5) Integracja 1:1 pod nasze crate'y

### `crates/protocols`
- pozostaje low-level adapter:
  - `WhirlpoolReader`, `PositionReader`, `WhirlpoolExecutor`.

### `crates/execution`
- `RebalanceExecutor` i `StrategyExecutor` powinny korzystac z `OrcaTxService` zamiast bezposrednio z `WhirlpoolExecutor` (thin wrapper + policy).

### `crates/api`
- `PositionService`:
  - read/validation przez `OrcaReadService` (plus monitor/lifecycle),
  - write przez `OrcaTxService`.

### `crates/cli`
- `orca-position-open`, `orca-position-decrease`, `orca-bot-run`:
  - wallet przez `load_signing_wallet`,
  - tx przez `OrcaTxService`,
  - odczyt przez `OrcaReadService` (lub on-chain reader fallback).

## 6) Checklist implementacji (konkret)

### A. Read layer
- [ ] Dodac `crates/data/src/providers/orca_rest.rs` (klient `api.orca.so/v2/solana`).
- [ ] Zaimplementowac endpointy: `/pools`, `/pools/search`, `/pools/{address}`, `/lock/{address}`.
- [ ] Typy DTO + mapowanie do domeny (`PoolSummary`, `PoolDetails`, `LockInfo`).
- [ ] Retry + backoff + mapowanie 429/5xx.
- [ ] Testy integracyjne read (mock HTTP + golden JSON).

### B. Tx layer
- [ ] Dodac `OrcaTxService` (wrapper nad `WhirlpoolExecutor`).
- [ ] Wspolna polityka preflight: `simulate -> send -> confirm`.
- [ ] Wspolna polityka slippage (`token_min_*` nie moze byc stale 0 w trybie produkcyjnym).
- [ ] Jednolite `TxResult` (signature, slot, success, error).
- [ ] Testy jednostkowe dla walidacji requestow tx.

### C. Contract enforcement
- [ ] W `api` i `cli` usunac bezposrednie call-site do `WhirlpoolExecutor` tam, gdzie ma byc `OrcaTxService`.
- [ ] Dla wyboru puli/rankingu uzywac `OrcaReadService` zamiast ad-hoc RPC.
- [ ] Zachowac fallback on-chain read dla krytycznych walidacji tx.

### D. Devnet test pack (MVP gate)
- [ ] E2E: open position (dry-run + execute).
- [ ] E2E: decrease (`pct` i `raw`) + weryfikacja liquidity po tx.
- [ ] E2E: collect fees.
- [ ] E2E: close position.
- [ ] E2E negatywne: zly tick spacing / zly range / zbyt niski slippage min.
- [ ] Raport z sygnaturami i statusem GO/NO-GO przed mainnet.

## 7) Priorytet implementacji (krotko)

1. `OrcaTxService` + devnet E2E write paths (najwyzszy priorytet).
2. `OrcaReadService` REST dla discovery/rankingu.
3. Refactor call-sites (`api`, `cli`, `execution`) do nowego kontraktu.

