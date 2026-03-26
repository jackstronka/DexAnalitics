//! Backtest price path built from local protocol snapshots (no Birdeye).
//!
//! Steps align with snapshot timestamps. Spot price uses `sqrt_price_x64` when present,
//! otherwise `tick_current`. Quote-token USD (for cross-pairs) is approximated via
//! Dexscreener (free HTTP, optional cache) — not on-chain.

use anyhow::{Context, Result, bail};
use clmm_lp_data::providers::{DexChain, DexscreenerClient};
use clmm_lp_domain::entities::token::Token;
use clmm_lp_domain::prelude::Price;
use clmm_lp_protocols::orca::pool_reader::tick_to_price;
use primitive_types::U256;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, MathematicalOps};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr;

use base64::{Engine, engine::general_purpose::STANDARD as BASE64_STANDARD};

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
    if exp >= 0 { d * p } else { d / p }
}

/// Raw B/A from sqrt Q64.64, then human token B per token A (pool orientation).
fn pool_b_per_a_human(sqrt: Option<u128>, tick: i32, dec_pool_a: u32, dec_pool_b: u32) -> Decimal {
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
    // Snapshot-only mode should not require any external USD oracle for common quote mints.
    // We treat stablecoins as $1 exactly.
    const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
    const USDT_MINT: &str = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";
    if mint.eq_ignore_ascii_case(USDC_MINT) || mint.eq_ignore_ascii_case(USDT_MINT) {
        return Some(Decimal::ONE);
    }

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

/// One row from Raydium snapshot JSONL (subset of fields).
#[derive(Clone, Debug)]
struct RaydiumSnapRow {
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
    protocol_fee_a: u64,
    protocol_fee_b: u64,
}

fn json_u128_from_any(v: &Value) -> Option<u128> {
    if v.is_string() {
        return v.as_str()?.trim().parse::<u128>().ok();
    }
    if v.is_u64() {
        return v.as_u64().map(|n| n as u128);
    }
    None
}

fn json_opt_u128(x: Option<&Value>) -> Option<u128> {
    x.and_then(json_u128_from_any)
}

fn parse_rows_raydium(path: &Path, start_ts: i64, end_ts: i64) -> Result<Vec<RaydiumSnapRow>> {
    let txt = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut rows = Vec::new();
    for line in txt.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
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
            .unwrap_or_default()
            .to_string();
        let pool_mint_b = v
            .get("token_mint_b")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string();

        // If the snapshot collector had parse issues for a row, these may be absent.
        // We skip such rows to keep the simulation consistent.
        if pool_mint_a.is_empty() || pool_mint_b.is_empty() {
            continue;
        }

        let vault_a = v
            .get("vault_amount_a")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let vault_b = v
            .get("vault_amount_b")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);

        let tick = v.get("tick_current").and_then(|x| x.as_i64()).unwrap_or(0) as i32;

        let sqrt_price = v.get("sqrt_price_x64").and_then(|x| json_opt_u128(Some(x)));
        let fee_growth_a = v
            .get("fee_growth_global_a_x64")
            .and_then(|x| json_opt_u128(Some(x)));
        let fee_growth_b = v
            .get("fee_growth_global_b_x64")
            .and_then(|x| json_opt_u128(Some(x)));

        let liq = v
            .get("liquidity_active")
            .and_then(|x| json_opt_u128(Some(x)));

        let protocol_fee_a = v
            .get("protocol_fees_token_a")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let protocol_fee_b = v
            .get("protocol_fees_token_b")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);

        rows.push(RaydiumSnapRow {
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

fn fee_delta_tokens_raydium(p0: &RaydiumSnapRow, p1: &RaydiumSnapRow) -> (u128, u128) {
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
        .unwrap_or_else(|| p1.protocol_fee_a.saturating_sub(p0.protocol_fee_a).into());
    let dv_b = delta_from_growth(p0.fee_growth_b, p1.fee_growth_b, p1.liq.or(p0.liq))
        .unwrap_or_else(|| p1.protocol_fee_b.saturating_sub(p0.protocol_fee_b).into());
    (dv_a, dv_b)
}

/// Build [`StepDataPoint`] grid + optional per-step fee map from Raydium JSONL only.
pub async fn build_from_raydium_snapshots(
    pool_address: &str,
    start_ts: i64,
    end_ts: i64,
    token_a: &Token,
    token_b: &Token,
    capital: f64,
    lp_share_cli: Option<f64>,
) -> Result<SnapshotPricePathResult> {
    let snapshots_jsonl = Path::new("data")
        .join("pool-snapshots")
        .join("raydium")
        .join(pool_address)
        .join("snapshots.jsonl");

    let repaired_jsonl = snapshots_jsonl.with_file_name("snapshots.jsonl.repaired");
    let path = if repaired_jsonl.exists() {
        repaired_jsonl
    } else {
        snapshots_jsonl
    };

    if !path.exists() {
        bail!(
            "Snapshot file not found (tried {}): {}",
            "snapshots.jsonl[.repaired]",
            path.display()
        );
    }

    let rows = parse_rows_raydium(&path, start_ts, end_ts)?;
    if rows.is_empty() {
        return Ok(SnapshotPricePathResult {
            step_data: Vec::new(),
            per_step_fees_usd: None,
        });
    }

    let dec_pool_a: u8 = {
        // Raydium snapshot contains mints, but decimals can still vary across pools.
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
        bail!("Could not resolve quote token USD price for snapshots");
    }

    let capital_dec = Decimal::from_f64(capital).unwrap_or(Decimal::ZERO);
    let lp_override = lp_share_cli
        .and_then(Decimal::from_f64)
        .filter(|s| *s > Decimal::ZERO && *s <= Decimal::ONE);

    let mut step_data: Vec<StepDataPoint> = Vec::with_capacity(rows.len());
    for r in &rows {
        let pool_bpa =
            pool_b_per_a_human(r.sqrt_price, r.tick, dec_pool_a as u32, dec_pool_b as u32);
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
            pool_liquidity_active: r.liq,
            start_timestamp: r.ts_u64,
        });
    }

    // Per-step fee proxy from Raydium fee growth accumulators.
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
        let (dv_a, dv_b) = fee_delta_tokens_raydium(p0, p1);
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

/// One row from Meteora snapshot JSONL (subset of fields).
///
/// We decode the on-chain `lb_pair` from `data_b64` for every snapshot row and extract:
/// `active_id`, `bin_step`, token mints (token_x/token_y) and `protocol_fee_amount_*`.
///
/// When `vault_amount_a` / `vault_amount_b` are present in JSON (written by
/// `meteora-snapshot-curated`), we can estimate TVL and `lp_share` like Raydium.
/// Otherwise `--lp-share` is required for snapshot-only Meteora.
#[derive(Clone, Debug)]
struct MeteoraSnapRow {
    ts: i64,
    ts_u64: u64,
    pool_mint_a: String,
    pool_mint_b: String,
    active_id: i32,
    bin_step: u16,
    protocol_fee_a: u64,
    protocol_fee_b: u64,
    /// SPL reserve balance for pool token X (same as snapshot `vault_amount_a`).
    vault_x: Option<u64>,
    /// SPL reserve balance for pool token Y (`vault_amount_b`).
    vault_y: Option<u64>,
}

fn parse_rows_meteora(path: &Path, start_ts: i64, end_ts: i64) -> Result<Vec<MeteoraSnapRow>> {
    let txt = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut rows = Vec::new();

    for line in txt.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let ts = v
            .get("ts_utc")
            .and_then(|x| x.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp())
            .context("ts_utc missing")?;

        if ts < start_ts || ts > end_ts {
            continue;
        }

        let data_b64 = v.get("data_b64").and_then(|x| x.as_str()).unwrap_or("");
        if data_b64.is_empty() {
            continue;
        }
        let bytes = match BASE64_STANDARD.decode(data_b64) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let parsed = match clmm_lp_protocols::meteora::pool_reader::parse_lb_pair(&bytes) {
            Ok(p) => p,
            Err(_) => continue,
        };

        let pool_mint_a = parsed.token_mint_x.to_string();
        let pool_mint_b = parsed.token_mint_y.to_string();
        let active_id = parsed.active_id;
        let bin_step = parsed.bin_step;
        let protocol_fee_a = parsed.protocol_fee_amount_x;
        let protocol_fee_b = parsed.protocol_fee_amount_y;
        let vault_x = v.get("vault_amount_a").and_then(|x| x.as_u64());
        let vault_y = v.get("vault_amount_b").and_then(|x| x.as_u64());

        rows.push(MeteoraSnapRow {
            ts,
            ts_u64: ts.max(0) as u64,
            pool_mint_a,
            pool_mint_b,
            active_id,
            bin_step,
            protocol_fee_a,
            protocol_fee_b,
            vault_x,
            vault_y,
        });
    }

    rows.sort_by_key(|r| r.ts);
    Ok(rows)
}

/// DLMM price approximation: `price = (1 + bin_step/BASIS_POINT_MAX)^active_id`.
///
/// We assume the "pool price" is token Y per token X (i.e. `B per A` for `token_mint_x/token_mint_y`).
fn pool_bpa_meteora(
    active_id: i32,
    bin_step: u16,
    dec_pool_a: u32,
    dec_pool_b: u32,
) -> Result<Decimal> {
    const BASIS_POINT_MAX: u32 = 10_000;
    let base = Decimal::ONE + (Decimal::from(bin_step) / Decimal::from(BASIS_POINT_MAX));
    let price_raw = base.powi(active_id as i64);
    // Align to UI units (similar to Orca/Raydium decimals scaling).
    Ok(scale_decimal_pow10(
        price_raw,
        dec_pool_a as i32 - dec_pool_b as i32,
    ))
}

/// Build [`StepDataPoint`] grid + optional per-step fee map from Meteora JSONL only.
pub async fn build_from_meteora_snapshots(
    pool_address: &str,
    start_ts: i64,
    end_ts: i64,
    token_a: &Token,
    token_b: &Token,
    capital: f64,
    lp_share_cli: Option<f64>,
) -> Result<SnapshotPricePathResult> {
    let snapshots_jsonl = Path::new("data")
        .join("pool-snapshots")
        .join("meteora")
        .join(pool_address)
        .join("snapshots.jsonl");
    // Prefer repaired JSONL if it exists.
    let repaired_jsonl = snapshots_jsonl.with_file_name("snapshots.jsonl.repaired");
    let path = if repaired_jsonl.exists() {
        repaired_jsonl
    } else {
        snapshots_jsonl
    };

    if !path.exists() {
        bail!(
            "Snapshot file not found (tried {}): {}",
            "snapshots.jsonl[.repaired]",
            path.display()
        );
    }

    let rows = parse_rows_meteora(&path, start_ts, end_ts)?;
    if rows.is_empty() {
        return Ok(SnapshotPricePathResult {
            step_data: Vec::new(),
            per_step_fees_usd: None,
        });
    }

    // We treat the DLMM "pool price" as token_y per token_x (B per A).
    // Snapshot-only mode should still work even if decoded token mints don't match CLI mints,
    // so we fall back to an "assume direct mapping" strategy.
    enum MeteoraMintMap {
        Direct,
        Swapped,
        Assumed,
    }

    let map = {
        let px = rows[0].pool_mint_a.as_str();
        let py = rows[0].pool_mint_b.as_str();
        let ua = token_a.mint_address.as_str();
        let ub = token_b.mint_address.as_str();
        if px.eq_ignore_ascii_case(ua) && py.eq_ignore_ascii_case(ub) {
            MeteoraMintMap::Direct
        } else if px.eq_ignore_ascii_case(ub) && py.eq_ignore_ascii_case(ua) {
            MeteoraMintMap::Swapped
        } else {
            MeteoraMintMap::Assumed
        }
    };

    // Token decimals in pool-orientation (token_x, token_y).
    let (dec_pool_a, dec_pool_b): (u8, u8) = match map {
        MeteoraMintMap::Direct | MeteoraMintMap::Assumed => (token_a.decimals, token_b.decimals),
        MeteoraMintMap::Swapped => (token_b.decimals, token_a.decimals),
    };

    let quote_usd = mint_usd_dexscreener(&token_b.mint_address)
        .await
        .unwrap_or(Decimal::ONE);
    if quote_usd <= Decimal::ZERO {
        bail!("Could not resolve quote token USD price via snapshots/stable mapping");
    }

    let capital_dec = Decimal::from_f64(capital).unwrap_or(Decimal::ZERO);
    let lp_override = lp_share_cli
        .and_then(Decimal::from_f64)
        .filter(|s| *s > Decimal::ZERO && *s <= Decimal::ONE);

    let all_vaults = rows
        .iter()
        .all(|r| r.vault_x.is_some() && r.vault_y.is_some());
    if lp_override.is_none() && !all_vaults {
        bail!(
            "Meteora snapshot-only: set --lp-share, or re-run `meteora-snapshot-curated` so snapshots include vault_amount_a/vault_amount_b (needed for TVL → lp_share)."
        );
    }

    let pow10 = |d: u32| -> Decimal {
        let mut v = Decimal::ONE;
        for _ in 0..d {
            v *= Decimal::from(10u32);
        }
        v
    };

    let mut step_data: Vec<StepDataPoint> = Vec::with_capacity(rows.len());
    for r in &rows {
        let pool_bpa = pool_bpa_meteora(
            r.active_id,
            r.bin_step,
            dec_pool_a as u32,
            dec_pool_b as u32,
        )?;
        // Convert from pool-orientation (token_y/token_x) to user-orientation (token_b/token_a).
        let price_ab = match map {
            MeteoraMintMap::Direct | MeteoraMintMap::Assumed => Price::new(pool_bpa),
            MeteoraMintMap::Swapped => {
                if pool_bpa.is_zero() {
                    bail!("Meteora pool price is zero; cannot invert");
                }
                Price::new(Decimal::ONE / pool_bpa)
            }
        };
        let price_usd = Price::new(price_ab.value * quote_usd);

        let lp_share = if let Some(s) = lp_override {
            s
        } else if let (Some(vx), Some(vy)) = (r.vault_x, r.vault_y) {
            let hum_x = Decimal::from(vx) / pow10(dec_pool_a as u32);
            let hum_y = Decimal::from(vy) / pow10(dec_pool_b as u32);
            let tvl_usd = match map {
                MeteoraMintMap::Direct | MeteoraMintMap::Assumed => {
                    hum_x * price_usd.value + hum_y * quote_usd
                }
                MeteoraMintMap::Swapped => hum_x * quote_usd + hum_y * price_usd.value,
            };
            if tvl_usd > Decimal::ZERO {
                (capital_dec / tvl_usd).min(Decimal::ONE).max(Decimal::ZERO)
            } else {
                Decimal::from_f64(0.01).unwrap()
            }
        } else {
            Decimal::from_f64(0.01).unwrap()
        };

        step_data.push(StepDataPoint {
            price_usd,
            price_ab,
            step_volume_usd: Decimal::ZERO,
            quote_usd,
            lp_share,
            pool_liquidity_active: None,
            start_timestamp: r.ts_u64,
        });
    }

    // Fees per step from protocol fee amount deltas.
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
        let dv_a = p1.protocol_fee_a.saturating_sub(p0.protocol_fee_a);
        let dv_b = p1.protocol_fee_b.saturating_sub(p0.protocol_fee_b);
        if dv_a == 0 && dv_b == 0 {
            continue;
        }
        let step = step_data.get(i).context("step_data index")?;
        let mut usd = Decimal::ZERO;

        // dv_a/dv_b are in pool-token_x / token_y units; convert them to user-token USD.
        let amt_x = Decimal::from_str(&dv_a.to_string()).unwrap_or(Decimal::ZERO)
            / pow10d(dec_pool_a as u32);
        let amt_y = Decimal::from_str(&dv_b.to_string()).unwrap_or(Decimal::ZERO)
            / pow10d(dec_pool_b as u32);

        match map {
            MeteoraMintMap::Direct | MeteoraMintMap::Assumed => {
                // token_x == user token_a
                usd += amt_x * step.price_usd.value;
                // token_y == user token_b
                usd += amt_y * step.quote_usd;
            }
            MeteoraMintMap::Swapped => {
                // token_x == user token_b
                usd += amt_x * step.quote_usd;
                // token_y == user token_a
                usd += amt_y * step.price_usd.value;
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

fn parse_rows(path: &Path, start_ts: i64, end_ts: i64) -> Result<Vec<OrcaSnapRow>> {
    let txt = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut rows = Vec::new();
    for line in txt.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Robustness: snapshot collectors can occasionally leave behind partially written
        // JSONL lines. For backtests we prefer to skip such lines instead of failing
        // the entire run.
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
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
        let vault_a = v
            .get("vault_amount_a")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let vault_b = v
            .get("vault_amount_b")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let tick = v.get("tick_current").and_then(|x| x.as_i64()).unwrap_or(0) as i32;
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

fn fee_delta_tokens(p0: &OrcaSnapRow, p1: &OrcaSnapRow) -> (u128, u128) {
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
    let snapshots_jsonl = Path::new("data")
        .join("pool-snapshots")
        .join("orca")
        .join(pool_address)
        .join("snapshots.jsonl");
    // Some snapshot collectors may leave behind partially written/dirty JSONL lines
    // near the end of the file; a `.repaired` sibling is produced to make parsing deterministic.
    let repaired_jsonl = snapshots_jsonl.with_file_name("snapshots.jsonl.repaired");
    let path = if repaired_jsonl.exists() {
        repaired_jsonl
    } else {
        snapshots_jsonl
    };

    if !path.exists() {
        bail!(
            "Snapshot file not found (tried {}): {}",
            "snapshots.jsonl[.repaired]",
            path.display()
        );
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
        let pool_bpa =
            pool_b_per_a_human(r.sqrt_price, r.tick, dec_pool_a as u32, dec_pool_b as u32);
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
            pool_liquidity_active: r.liq,
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
