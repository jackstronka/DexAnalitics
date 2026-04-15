//! Aerodrome **Slipstream** (concentrated liquidity on Base).
//!
//! Operacyjny plan wdrożenia live (fazy, RPC, bezpieczeństwo): `doc/AERODROME_SLIPSTREAM_BASE_LIVE_PLAN.md`.
//!
//! # Product scope (Bociarz)
//! **Fee z handlu only:** zakładamy **unstaked** LP — zbieracie **swap / trading fees** przez
//! [`gauges_v3::NONFUNGIBLE_POSITION_MANAGER`] + pulę (`collect` / stan z NPM i poola).
//! **Stake w gauge** i **emisje AERO** są poza pierwszym zakresem (inna ekonomia opłat w spec).
//!
//! # Sources of truth
//! - Deployed contract addresses: [`Gauges V3`](https://github.com/aerodrome-finance/slipstream/blob/main/README.md)
//!   section *Gauges V3 Deployment* in the official Slipstream repository.
//! - Behaviour and fee/gauge semantics: `SPECIFICATION.md` in the same repository.
//!
//! # Integration checklist (read path first)
//! 1. **Base JSON-RPC** — chain id [`BASE_MAINNET_CHAIN_ID`].
//! 2. **Resolve pool** — `PoolFactory.getPool(...)` (or the pool address from config/UI); confirm `token0 < token1`.
//! 3. **Pool state** — `slot0`, `liquidity`, `tickSpacing`, dynamic fee reads per pool/factory (do not assume static v3 fee tiers).
//! 4. **Quotes** — prefer the deployment-appropriate quoter ([`gauges_v3::MIXED_QUOTER_V3`], [`gauges_v3::QUOTER`]) for routing across Slipstream factories.
//! 5. **Positions** — [`gauges_v3::NONFUNGIBLE_POSITION_MANAGER`]: `positions(tokenId)`, ownership, fee growth / `collect`; ignore gauge contracts until emissions are in scope.
//!
//! **Note:** Even fee-only LP uses the **unstaked fee module** path on swap fees (see upstream `SPECIFICATION.md`); do not assume plain Uniswap-v3 fee math without reading on-chain fee state.

/// Base (Coinbase L2) mainnet chain id.
pub const BASE_MAINNET_CHAIN_ID: u64 = 8453;

/// Gauges V3 Slipstream deployment on Base (new gauges use this stack per upstream README).
///
/// Re-verify on [BaseScan](https://basescan.org) and the Slipstream `README.md` before mainnet funds.
pub mod gauges_v3 {
    /// `PoolFactory` — create/get pool addresses.
    pub const POOL_FACTORY: &str = "0xf8f2eB4940CFE7d13603DDDD87f123820Fc061Ef";
    /// `NonfungiblePositionManager` — mint/increase/decrease/collect; NFT ids are positions.
    pub const NONFUNGIBLE_POSITION_MANAGER: &str = "0xe1f8cd9AC4e4A65F54f38a5CdAfCA44f6dD68b53";
    /// `SwapRouter` — swaps against Slipstream pools.
    pub const SWAP_ROUTER: &str = "0x698Cb2b6dd822994581fEa6eA4Fc755d1363A92F";
    /// `Quoter` — single-factory style quotes (v3-like).
    pub const QUOTER: &str = "0x514c8B5f54112481E28028F1166Bd78501089259";
    /// `MixedQuoterV3` — quotes across multiple CL factories (bitmask encoding per upstream).
    pub const MIXED_QUOTER_V3: &str = "0xCd2A7D98e82D6107eac1828ce8DeAA6acB65b555";
    /// `MixedQuoter` / `MixedQuoterV2` — older mixed-route quoters from the same deployment table (keep for legacy pools/paths).
    pub const MIXED_QUOTER: &str = "0x9951FF0b830E46ef0e7Ce34d9117e3214B1F0b5a";
    pub const MIXED_QUOTER_V2: &str = "0xb4A9E5Fc0727BEF09D819fcfc5ece8CA9bCf09EB";
    /// `GaugeFactory` — for **gauge / emissions** flows only; not required for fee-only unstaked LP.
    pub const GAUGE_FACTORY: &str = "0x385293CaE378C813F16f0C1334d774AdDDf56AbB";
    /// `DynamicSwapFeeModule` — dynamic swap fee logic for this deployment generation.
    pub const DYNAMIC_SWAP_FEE_MODULE: &str = "0x87D8f999BBa9343E8099552426775B51C338E8CB";
    /// `UnstakedFeeModule` — extra fee path on unstaked liquidity (see Slipstream `SPECIFICATION.md`).
    pub const UNSTAKED_FEE_MODULE: &str = "0xc2cc3256434AfbC36Bb5e815e1Bb2151310a1a0b";

    /// All `0x`-prefixed 20-byte addresses in this module (sanity for copy-paste drift).
    pub const ALL: &[&str] = &[
        POOL_FACTORY,
        NONFUNGIBLE_POSITION_MANAGER,
        SWAP_ROUTER,
        QUOTER,
        MIXED_QUOTER_V3,
        MIXED_QUOTER,
        MIXED_QUOTER_V2,
        GAUGE_FACTORY,
        DYNAMIC_SWAP_FEE_MODULE,
        UNSTAKED_FEE_MODULE,
    ];
}

#[cfg(test)]
mod tests {
    use super::gauges_v3;

    fn assert_evm_address(s: &str) {
        let b = s.as_bytes();
        assert_eq!(b.len(), 42, "len: {s}");
        assert_eq!(b[0], b'0');
        assert_eq!(b[1], b'x');
        for &ch in &b[2..] {
            assert!(ch.is_ascii_hexdigit(), "non-hex: {s}");
        }
    }

    #[test]
    fn gauges_v3_constants_are_valid_addresses() {
        for &addr in gauges_v3::ALL {
            assert_evm_address(addr);
        }
    }
}
