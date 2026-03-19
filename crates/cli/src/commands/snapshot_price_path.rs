//! Backtest price path built only from local Orca `snapshots.jsonl` (no Birdeye).
//!
//! Steps align with snapshot timestamps. Spot price uses `sqrt_price_x64` when present,
//! otherwise `tick_current`. Quote-token USD (for cross-pairs) is approximated via
//! Dexscreener (free HTTP, optional cache) — not on-chain.

use anyhow::{bail, Context, Result};
use clmm_lp_domain::entities::token::Token;
use clmm_lp_domain::prelude::Price;
use clmm_lp_data::providers::{DexChain, DexscreenerClient};
use clmm_lp_protocols::orca::pool_reader::tick_to_price;
use primitive_types::U256;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr;

use crate::backtest_engine::StepDataPoint;

/// One row from Orca snapshot JSONL (subset of fields).
#[derive(Clone, Debug)]
struct OrcaSnapRow {
    ts: i64,
    ts_u64: u64,
    pool_mint_a: String,
    pool_mint_b: String,
    vault_a: u64,
    vault_b: u64,
    tick: i32,
    sqrt_price: Option<u128>,
    fee_growth_a: Option<u128>,
    fee_growth_b: Option<u128>,
    liq: Option<u128>,
    protocol_fee_a: u128,
    protocol_fee_b: u128,
}

#[derive(Debug)]
pub struct SnapshotPricePathResult {
    pub step_data: Vec<StepDataPoint>,
    /// Per-step pool fee proxy in USD (index `i` = fee accrued between snapshot i-1 and i).
    pub per_step_fees_usd: Option<BTreeMap<usize, Decimal>>,
}

fn json_u128(x: Option<&Value>) -> Option<u128> {
    match x {
        Some(v) if v.is_u64() => v.as_u64().map(|n| n as u128),
        Some(v) if v.is_string() => v.as_str().and_then(|s| s.trim().parse::<u128>().ok()),
        _ => None,
    }
}

fn scale_decimal_pow10(d: Decimal, exp: i32) -> Decimal {
    let e = exp.unsigned_abs() as u32;
    if e > 18 {
        return d;
    }
    let p = Decimal::from(10u64.pow(e.min(18)));
    if exp >= 0 {
        d * p
    } else {
        d / p
    }
}

/// Raw B/A from sqrt Q64.64, then human token B per token A (pool orientation).
fn pool_b_per_a_human(
    sqrt: Option<u128>,
    tick: i32,
    dec_pool_a: u32,
    dec_pool_b: u32,
) -> Decimal {
    let raw = if let Some(s) = sqrt.filter(|&x| x > 0) {
        let sqrt_f = s as f64 / (1u128 << 64) as f64;
        Decimal::from_f64(sqrt_f * sqrt_f).unwrap_or(Decimal::ZERO)
    } else {
        tick_to_price(tick)
    };
    scale_decimal_pow10(raw, dec_pool_a as i32 - dec_pool_b as i32)
}

fn user_price_ab(
    pool_b_per_a: Decimal,
    user_mint_a: &str,
    user_mint_b: &str,
    pool_mint_a: &str,
    pool_mint_b: &str,
) -> Result<Decimal> {
    let ua = user_mint_a.trim();
    let ub = user_mint_b.trim();
    let pa = pool_mint_a.trim();
    let pb = pool_mint_b.trim();
    if ua.eq_ignore_ascii_case(pa) && ub.eq_ignore_ascii_case(pb) {
        Ok(pool_b_per_a)
    } else if ua.eq_ignore_ascii_case(pb) && ub.eq_ignore_ascii_case(pa) {
        if pool_b_per_a.is_zero() {
            bail!("pool price ratio is zero; cannot invert");
        }
        Ok(Decimal::ONE / pool_b_per_a)
    } else {
        bail!(
            "Token mints do not match pool: user {}/{} vs pool {}/{}",
            ua,
            ub,
            pa,
            pb
        );
    }
}

async fn mint_usd_dexscreener(mint: &str) -> Option<Decimal> {
    let client = DexscreenerClient::new();
    let pairs = client.token_pairs(DexChain::Solana, mint).await.ok()?;
    let best = pairs.iter().max_by(|a, b| {
        a.liquidity
            .usd
            .partial_cmp(&b.liquidity.usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    })?;
    let px: f64 = best.price_usd.parse().ok()?;
    Decimal::from_f64(px)
}

fn parse_rows(path: &Path, start_ts: i64, end_ts: i64) -> Result<Vec<OrcaSnapRow>> {
    let txt = std::fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?;
    let mut rows = Vec::new();
    for line in txt.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line).with_context(|| "bad JSONL line")?;
        let ts = v
            .get("ts_utc")
            .and_then(|x| x.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp())
            .context("ts_utc missing")?;
        if ts < start_ts || ts > end_ts {
            continue;
        }
        let pool_mint_a = v
            .get("token_mint_a")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let pool_mint_b = v
            .get("token_mint_b")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let vault_a = v.get("vault_amount_a").and_then(|x| x.as_u64()).unwrap_or(0);
        let vault_b = v.get("vault_amount_b").and_then(|x| x.as_u64()).unwrap_or(0);
        let tick = v
            .get("tick_current")
            .and_then(|x| x.as_i64())
            .unwrap_or(0) as i32;
        let sqrt_price = v.get("sqrt_price_x64").and_then(|x| json_u128(Some(x)));
        let fee_growth_a = json_u128(v.get("fee_growth_global_a"));
        let fee_growth_b = json_u128(v.get("fee_growth_global_b"));
        let liq = v
            .get("liquidity_active")
            .and_then(|x| x.as_str())
            .and_then(|s| s.parse::<u128>().ok())
            .or_else(|| json_u128(v.get("liquidity_active")));
        let protocol_fee_a = v
            .get("protocol_fee_owed_a")
            .and_then(|x| x.as_u64())
            .unwrap_or(0) as u128;
        let protocol_fee_b = v
            .get("protocol_fee_owed_b")
            .and_then(|x| x.as_u64())
            .unwrap_or(0) as u128;
        rows.push(OrcaSnapRow {
            ts,
            ts_u64: ts.max(0) as u64,
            pool_mint_a,
            pool_mint_b,
            vault_a,
            vault_b,
            tick,
            sqrt_price,
            fee_growth_a,
            fee_growth_b,
            liq,
            protocol_fee_a,
            protocol_fee_b,
        });
    }
    rows.sort_by_key(|r| r.ts);
    Ok(rows)
}

fn fee_delta_tokens(
    p0: &OrcaSnapRow,
    p1: &OrcaSnapRow,
) -> (u128, u128) {
    let q64: U256 = U256::from(1u128) << 64;
    let delta_from_growth =
        |g0: Option<u128>, g1: Option<u128>, liq: Option<u128>| -> Option<u128> {
            let (g0, g1, liq) = (g0?, g1?, liq?);
            if g1 <= g0 || liq == 0 {
                return Some(0);
            }
            let dg = g1 - g0;
            let prod = U256::from(dg).saturating_mul(U256::from(liq));
            let raw = prod / q64;
            Some(raw.low_u128())
        };
    let dv_a = delta_from_growth(p0.fee_growth_a, p1.fee_growth_a, p1.liq.or(p0.liq))
        .unwrap_or_else(|| p1.protocol_fee_a.saturating_sub(p0.protocol_fee_a));
    let dv_b = delta_from_growth(p0.fee_growth_b, p1.fee_growth_b, p1.liq.or(p0.liq))
        .unwrap_or_else(|| p1.protocol_fee_b.saturating_sub(p0.protocol_fee_b));
    (dv_a, dv_b)
}

/// Build [`StepDataPoint`] grid + optional per-step fee map from Orca JSONL only.
pub async fn build_from_orca_snapshots(
    pool_address: &str,
    start_ts: i64,
    end_ts: i64,
    token_a: &Token,
    token_b: &Token,
    capital: f64,
    lp_share_cli: Option<f64>,
) -> Result<SnapshotPricePathResult> {
    let path = Path::new("data")
        .join("pool-snapshots")
        .join("orca")
        .join(pool_address)
        .join("snapshots.jsonl");
    if !path.exists() {
        bail!("Snapshot file not found: {}", path.display());
    }

    let rows = parse_rows(&path, start_ts, end_ts)?;
    if rows.is_empty() {
        return Ok(SnapshotPricePathResult {
            step_data: Vec::new(),
            per_step_fees_usd: None,
        });
    }

    let dec_pool_a: u8 = {
        use crate::engine::token_meta::fetch_mint_decimals;
        use clmm_lp_protocols::rpc::RpcProvider;
        let rpc = RpcProvider::mainnet();
        fetch_mint_decimals(&rpc, &rows[0].pool_mint_a)
            .await
            .unwrap_or(9)
    };
    let dec_pool_b: u8 = {
        use crate::engine::token_meta::fetch_mint_decimals;
        use clmm_lp_protocols::rpc::RpcProvider;
        let rpc = RpcProvider::mainnet();
        fetch_mint_decimals(&rpc, &rows[0].pool_mint_b)
            .await
            .unwrap_or(9)
    };

    let quote_usd = mint_usd_dexscreener(&token_b.mint_address)
        .await
        .unwrap_or(Decimal::ONE);
    if quote_usd <= Decimal::ZERO {
        bail!("Could not resolve quote token USD price via Dexscreener");
    }

    let capital_dec = Decimal::from_f64(capital).unwrap_or(Decimal::ZERO);
    let lp_override = lp_share_cli
        .and_then(Decimal::from_f64)
        .filter(|s| *s > Decimal::ZERO && *s <= Decimal::ONE);

    let mut step_data: Vec<StepDataPoint> = Vec::with_capacity(rows.len());
    for r in &rows {
        let pool_bpa = pool_b_per_a_human(r.sqrt_price, r.tick, dec_pool_a as u32, dec_pool_b as u32);
        let price_ab = Price::new(user_price_ab(
            pool_bpa,
            &token_a.mint_address,
            &token_b.mint_address,
            &r.pool_mint_a,
            &r.pool_mint_b,
        )?);
        let price_usd = Price::new(price_ab.value * quote_usd);

        let pow10 = |d: u32| -> Decimal {
            let mut v = Decimal::ONE;
            for _ in 0..d {
                v *= Decimal::from(10u32);
            }
            v
        };
        let hum_pa = Decimal::from(r.vault_a) / pow10(dec_pool_a as u32);
        let hum_pb = Decimal::from(r.vault_b) / pow10(dec_pool_b as u32);
        let tvl_usd = hum_pa * quote_usd + hum_pb * (pool_bpa * quote_usd);

        let lp_share = if let Some(s) = lp_override {
            s
        } else if tvl_usd > Decimal::ZERO {
            (capital_dec / tvl_usd).min(Decimal::ONE).max(Decimal::ZERO)
        } else {
            Decimal::from_f64(0.01).unwrap()
        };

        step_data.push(StepDataPoint {
            price_usd,
            price_ab,
            step_volume_usd: Decimal::ZERO,
            quote_usd,
            lp_share,
            start_timestamp: r.ts_u64,
        });
    }

    let mut fee_map: BTreeMap<usize, Decimal> = BTreeMap::new();
    let pow10d = |d: u32| -> Decimal {
        let mut v = Decimal::ONE;
        for _ in 0..d {
            v *= Decimal::from(10u32);
        }
        v
    };
    for i in 1..rows.len() {
        let p0 = &rows[i - 1];
        let p1 = &rows[i];
        let (dv_a, dv_b) = fee_delta_tokens(p0, p1);
        if dv_a == 0 && dv_b == 0 {
            continue;
        }
        let step = step_data.get(i).context("step_data index")?;
        let mut usd = Decimal::ZERO;
        if dv_a > 0 {
            let dec_a = dec_pool_a as u32;
            let amt = Decimal::from_str(&dv_a.to_string()).unwrap_or(Decimal::ZERO) / pow10d(dec_a);
            if p1.pool_mint_a.eq_ignore_ascii_case(&token_a.mint_address) {
                usd += amt * step.price_usd.value;
            } else if p1.pool_mint_a.eq_ignore_ascii_case(&token_b.mint_address) {
                usd += amt * step.quote_usd;
            }
        }
        if dv_b > 0 {
            let dec_b = dec_pool_b as u32;
            let amt = Decimal::from_str(&dv_b.to_string()).unwrap_or(Decimal::ZERO) / pow10d(dec_b);
            if p1.pool_mint_b.eq_ignore_ascii_case(&token_a.mint_address) {
                usd += amt * step.price_usd.value;
            } else if p1.pool_mint_b.eq_ignore_ascii_case(&token_b.mint_address) {
                usd += amt * step.quote_usd;
            }
        }
        if usd > Decimal::ZERO {
            fee_map.insert(i, usd);
        }
    }

    let per_step_fees_usd = if fee_map.is_empty() {
        None
    } else {
        Some(fee_map)
    };

    Ok(SnapshotPricePathResult {
        step_data,
        per_step_fees_usd,
    })
}
