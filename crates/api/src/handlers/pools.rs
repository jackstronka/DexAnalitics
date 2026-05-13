//! Pool handlers.

use crate::error::{ApiError, ApiResult};
use crate::models::{
    ListPoolsResponse, OrcaVolumeCollectResponse, OrcaVolumeHistoryQuery,
    OrcaVolumeHistoryResponse, OrcaVolumeSnapshotRow, PoolResponse, PoolStateResponse,
    QuoteOpenBudgetRequest, QuoteOpenBudgetResponse, SwapCostEstimateResponse,
};
use crate::services::price_fetch::fetch_mint_prices_usd;
use crate::state::AppState;
use axum::{
    Json,
    extract::{Path, Query, State},
};
use clmm_lp_data::providers::{OrcaListPoolsQuery, OrcaRestClient};
use clmm_lp_protocols::ledger::swap_cost_estimate::{
    DEFAULT_ESTIMATED_SWAP_NETWORK_FEE_LAMPORTS, median_historical_swap_network_fee_lamports,
};
use clmm_lp_protocols::orca::deposit_quote::quote_deposit_budget_in_range;
use clmm_lp_protocols::prelude::WhirlpoolReader;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use solana_sdk::pubkey::Pubkey;
use spl_token::solana_program::program_pack::Pack;
use spl_token::state::Mint;
use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::str::FromStr;

const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const DEV_USDC_MINT: &str = "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU";

fn parse_decimal_opt(v: Option<&String>) -> Option<Decimal> {
    v.and_then(|s| s.parse::<f64>().ok())
        .and_then(Decimal::from_f64)
}

fn is_usdc_mint_str(mint: &str) -> bool {
    mint == USDC_MINT || mint == DEV_USDC_MINT
}

fn b_per_a_ui_price(tick: i32, dec_a: u8, dec_b: u8) -> Option<f64> {
    let ln_10001 = 1.0001_f64.ln();
    let ln_ui =
        (tick as f64) * ln_10001 + ((dec_a as f64) - (dec_b as f64)) * std::f64::consts::LN_10;
    if !ln_ui.is_finite() {
        return None;
    }
    let price = ln_ui.exp();
    if price.is_finite() && price > 0.0 {
        Some(price)
    } else {
        None
    }
}

fn apply_usdc_pool_price_fallback(
    mint_a: &str,
    mint_b: &str,
    dec_a: u8,
    dec_b: u8,
    tick_current: i32,
    price_a_usd: &mut f64,
    price_b_usd: &mut f64,
) {
    if is_usdc_mint_str(mint_a) && (!price_a_usd.is_finite() || *price_a_usd <= 0.0) {
        *price_a_usd = 1.0;
    }
    if is_usdc_mint_str(mint_b) && (!price_b_usd.is_finite() || *price_b_usd <= 0.0) {
        *price_b_usd = 1.0;
    }

    let Some(b_per_a) = b_per_a_ui_price(tick_current, dec_a, dec_b) else {
        return;
    };

    if is_usdc_mint_str(mint_b)
        && price_b_usd.is_finite()
        && *price_b_usd > 0.0
        && (!price_a_usd.is_finite() || *price_a_usd <= 0.0)
    {
        *price_a_usd = b_per_a * *price_b_usd;
    } else if is_usdc_mint_str(mint_a)
        && price_a_usd.is_finite()
        && *price_a_usd > 0.0
        && (!price_b_usd.is_finite() || *price_b_usd <= 0.0)
    {
        *price_b_usd = *price_a_usd / b_per_a;
    }
}

fn volume_for_period(
    stats: &std::collections::HashMap<String, clmm_lp_data::providers::OrcaPoolStats>,
    period: &str,
) -> Option<Decimal> {
    let st = stats.get(period)?;
    parse_decimal_opt(st.volume.as_ref())
}

fn map_orca_pool_to_response(p: clmm_lp_data::providers::OrcaPoolSummary) -> PoolResponse {
    PoolResponse {
        address: p.address,
        protocol: "orca_whirlpool".to_string(),
        token_mint_a: p.token_mint_a,
        token_mint_b: p.token_mint_b,
        current_tick: p.tick_current_index,
        tick_spacing: p.tick_spacing as i32,
        price: Decimal::from_str_exact(&p.price).unwrap_or(Decimal::ZERO),
        liquidity: p.liquidity,
        fee_rate_bps: p.fee_rate,
        volume_24h_usd: volume_for_period(&p.stats, "24h"),
        volume_1h_usd: volume_for_period(&p.stats, "1h"),
        volume_5m_usd: volume_for_period(&p.stats, "5m"),
        volume_7d_usd: volume_for_period(&p.stats, "7d"),
        tvl_usd: p.tvl_usdc.parse::<f64>().ok().and_then(Decimal::from_f64),
        apy_estimate: None,
    }
}

fn orca_volume_history_path() -> std::path::PathBuf {
    std::path::PathBuf::from("data")
        .join("orca-rest")
        .join("pool_volume_history.jsonl")
}

/// List available pools.
#[utoipa::path(
    get,
    path = "/pools",
    tag = "Pools",
    responses(
        (status = 200, description = "List of pools", body = ListPoolsResponse)
    )
)]
pub async fn list_pools(State(_state): State<AppState>) -> ApiResult<Json<ListPoolsResponse>> {
    let base_url = std::env::var("ORCA_PUBLIC_API_BASE_URL")
        .unwrap_or_else(|_| "https://api.orca.so/v2/solana".to_string());
    let client = OrcaRestClient::new(base_url);

    let q = OrcaListPoolsQuery {
        size: Some(50),
        stats: Some("24h".to_string()),
        ..Default::default()
    };
    let paged = client
        .list_pools(q)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let pools = paged
        .data
        .into_iter()
        .map(map_orca_pool_to_response)
        .collect::<Vec<_>>();

    Ok(Json(ListPoolsResponse {
        total: pools.len(),
        pools,
    }))
}

/// Get pool details.
#[utoipa::path(
    get,
    path = "/pools/{address}",
    tag = "Pools",
    params(
        ("address" = String, Path, description = "Pool address")
    ),
    responses(
        (status = 200, description = "Pool details", body = PoolResponse),
        (status = 404, description = "Pool not found")
    )
)]
pub async fn get_pool(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<PoolResponse>> {
    let _pubkey =
        Pubkey::from_str(&address).map_err(|_| ApiError::bad_request("Invalid pool address"))?;

    let reader = WhirlpoolReader::new(state.provider.clone());

    let pool_state = reader
        .get_pool_state(&address)
        .await
        .map_err(|e| ApiError::not_found(format!("Pool not found: {}", e)))?;

    let response = PoolResponse {
        address: pool_state.address,
        protocol: "orca_whirlpool".to_string(),
        token_mint_a: pool_state.token_mint_a.to_string(),
        token_mint_b: pool_state.token_mint_b.to_string(),
        current_tick: pool_state.tick_current,
        tick_spacing: pool_state.tick_spacing as i32,
        price: pool_state.price,
        liquidity: pool_state.liquidity.to_string(),
        fee_rate_bps: pool_state.fee_rate_bps,
        volume_24h_usd: None,
        volume_1h_usd: None,
        volume_5m_usd: None,
        volume_7d_usd: None,
        tvl_usd: None,
        apy_estimate: None,
    };

    Ok(Json(response))
}

/// Fetch current Orca volumes and append normalized rows to local JSONL history.
#[utoipa::path(
    post,
    path = "/pools/orca/volume-history/collect",
    tag = "Pools",
    responses(
        (status = 200, description = "Orca volume snapshot stored", body = OrcaVolumeCollectResponse)
    )
)]
pub async fn collect_orca_volume_history() -> ApiResult<Json<OrcaVolumeCollectResponse>> {
    let base_url = std::env::var("ORCA_PUBLIC_API_BASE_URL")
        .unwrap_or_else(|_| "https://api.orca.so/v2/solana".to_string());
    let client = OrcaRestClient::new(base_url);
    let stats_windows = vec![
        "5m".to_string(),
        "1h".to_string(),
        "24h".to_string(),
        "7d".to_string(),
    ];
    let q = OrcaListPoolsQuery {
        size: Some(200),
        stats: Some(stats_windows.join(",")),
        ..Default::default()
    };
    let paged = client
        .list_pools(q)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let now = chrono::Utc::now().to_rfc3339();
    let rows: Vec<OrcaVolumeSnapshotRow> = paged
        .data
        .into_iter()
        .map(|p| OrcaVolumeSnapshotRow {
            ts_utc: now.clone(),
            source: "orca_public_api".to_string(),
            pool_address: p.address,
            token_mint_a: p.token_mint_a,
            token_mint_b: p.token_mint_b,
            fee_rate_bps: p.fee_rate,
            tvl_usd: p.tvl_usdc.parse::<f64>().ok().and_then(Decimal::from_f64),
            volume_5m_usd: volume_for_period(&p.stats, "5m"),
            volume_1h_usd: volume_for_period(&p.stats, "1h"),
            volume_24h_usd: volume_for_period(&p.stats, "24h"),
            volume_7d_usd: volume_for_period(&p.stats, "7d"),
        })
        .collect();

    let path = orca_volume_history_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ApiError::internal(format!("create history dir failed: {e}")))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| ApiError::internal(format!("open history file failed: {e}")))?;
    for row in &rows {
        let line = serde_json::to_string(row)
            .map_err(|e| ApiError::internal(format!("serialize history row failed: {e}")))?;
        file.write_all(line.as_bytes())
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|e| ApiError::internal(format!("append history row failed: {e}")))?;
    }

    Ok(Json(OrcaVolumeCollectResponse {
        collected_at_utc: now,
        path: path.to_string_lossy().to_string(),
        rows_appended: rows.len(),
        stats_windows,
    }))
}

/// Read persisted Orca volume history rows.
#[utoipa::path(
    get,
    path = "/pools/orca/volume-history",
    tag = "Pools",
    params(
        ("pool_address" = Option<String>, Query, description = "Optional pool address filter"),
        ("limit" = Option<u32>, Query, description = "Max rows to return (default 200, max 5000)")
    ),
    responses(
        (status = 200, description = "Persisted Orca volume snapshots", body = OrcaVolumeHistoryResponse)
    )
)]
pub async fn get_orca_volume_history(
    Query(q): Query<OrcaVolumeHistoryQuery>,
) -> ApiResult<Json<OrcaVolumeHistoryResponse>> {
    let path = orca_volume_history_path();
    let limit = q.limit.unwrap_or(200).min(5000) as usize;
    if !path.exists() {
        return Ok(Json(OrcaVolumeHistoryResponse {
            path: path.to_string_lossy().to_string(),
            rows: Vec::new(),
        }));
    }
    let f = std::fs::File::open(&path)
        .map_err(|e| ApiError::internal(format!("open history file failed: {e}")))?;
    let r = BufReader::new(f);
    let mut rows: Vec<OrcaVolumeSnapshotRow> = Vec::new();
    for line in r.lines().map_while(Result::ok) {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Ok(row) = serde_json::from_str::<OrcaVolumeSnapshotRow>(t) else {
            continue;
        };
        if let Some(ref want_pool) = q.pool_address
            && row.pool_address.trim() != want_pool.trim()
        {
            continue;
        }
        rows.push(row);
    }
    if rows.len() > limit {
        let start = rows.len() - limit;
        rows = rows[start..].to_vec();
    }
    Ok(Json(OrcaVolumeHistoryResponse {
        path: path.to_string_lossy().to_string(),
        rows,
    }))
}

/// Get current pool state.
#[utoipa::path(
    get,
    path = "/pools/{address}/state",
    tag = "Pools",
    params(
        ("address" = String, Path, description = "Pool address")
    ),
    responses(
        (status = 200, description = "Current pool state", body = PoolStateResponse),
        (status = 404, description = "Pool not found")
    )
)]
pub async fn get_pool_state(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<PoolStateResponse>> {
    let _pubkey =
        Pubkey::from_str(&address).map_err(|_| ApiError::bad_request("Invalid pool address"))?;

    let reader = WhirlpoolReader::new(state.provider.clone());

    let pool_state = reader
        .get_pool_state(&address)
        .await
        .map_err(|e| ApiError::not_found(format!("Pool not found: {}", e)))?;

    let response = PoolStateResponse {
        address: pool_state.address,
        current_tick: pool_state.tick_current,
        sqrt_price: pool_state.sqrt_price.to_string(),
        price: pool_state.price,
        liquidity: pool_state.liquidity.to_string(),
        fee_growth_global_a: pool_state.fee_growth_global_a.to_string(),
        fee_growth_global_b: pool_state.fee_growth_global_b.to_string(),
        timestamp: chrono::Utc::now(),
    };

    Ok(Json(response))
}

/// Rough **network fee** estimate for an in-pool Orca swap (from local lifecycle JSONL + default).
#[utoipa::path(
    get,
    path = "/pools/{address}/estimate-swap-cost",
    tag = "Pools",
    params(
        ("address" = String, Path, description = "Whirlpool pool address")
    ),
    responses(
        (status = 200, description = "Swap cost estimate", body = SwapCostEstimateResponse),
        (status = 404, description = "Pool not found")
    )
)]
pub async fn get_swap_cost_estimate(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<SwapCostEstimateResponse>> {
    let _ = Pubkey::from_str(address.trim())
        .map_err(|_| ApiError::bad_request("Invalid pool address"))?;

    let reader = WhirlpoolReader::new(state.provider.clone());
    reader
        .get_pool_state(address.trim())
        .await
        .map_err(|e| ApiError::not_found(format!("Pool not found: {}", e)))?;

    let (median, n) = median_historical_swap_network_fee_lamports(Some(address.trim()));
    let default = DEFAULT_ESTIMATED_SWAP_NETWORK_FEE_LAMPORTS;
    let est = median.map(|m| m.max(default)).unwrap_or(default);

    Ok(Json(SwapCostEstimateResponse {
        pool_address: address.trim().to_string(),
        historical_median_network_fee_lamports: median,
        historical_sample_count: n as u32,
        default_network_fee_lamports: default,
        estimated_network_fee_lamports: est,
        note: "Shows estimated Solana network fee (meta.fee) for Whirlpool swaps from local ledger history. Full wallet delta (tokens + rent) is recorded after execution in orca_position_lifecycle.jsonl. Send cost_session_id with POST /positions to group swap + open rows for per-position cost totals.".to_string(),
    }))
}

async fn mint_decimals_or_err(
    state: &AppState,
    mint: &Pubkey,
    label: &str,
) -> Result<u8, ApiError> {
    let account = state
        .provider
        .get_account(mint)
        .await
        .map_err(|e| ApiError::internal(format!("fetch mint {label}: {e}")))?;
    let m = Mint::unpack(&account.data)
        .map_err(|e| ApiError::internal(format!("unpack SPL mint {label}: {e}")))?;
    Ok(m.decimals)
}

/// Suggest `token_max_a/b` so an **in-range** open targets ~`target_usd` (fixes naive 50/50 USD caps).
#[utoipa::path(
    post,
    path = "/pools/{address}/quote-open-budget",
    tag = "Pools",
    request_body = QuoteOpenBudgetRequest,
    responses(
        (status = 200, description = "Suggested deposit caps", body = QuoteOpenBudgetResponse),
        (status = 400, description = "Invalid input or price out of range"),
        (status = 404, description = "Pool not found")
    )
)]
pub async fn quote_open_budget(
    State(state): State<AppState>,
    Path(address): Path<String>,
    Json(body): Json<QuoteOpenBudgetRequest>,
) -> ApiResult<Json<QuoteOpenBudgetResponse>> {
    let addr = address.trim();
    let _ = Pubkey::from_str(addr).map_err(|_| ApiError::bad_request("Invalid pool address"))?;

    let reader = WhirlpoolReader::new(state.provider.clone());
    let pool = reader
        .get_pool_state(addr)
        .await
        .map_err(|e| ApiError::not_found(format!("Pool not found: {e}")))?;

    let dec_a = mint_decimals_or_err(&state, &pool.token_mint_a, "A").await?;
    let dec_b = mint_decimals_or_err(&state, &pool.token_mint_b, "B").await?;

    let mut mints = BTreeSet::new();
    mints.insert(pool.token_mint_a.to_string());
    mints.insert(pool.token_mint_b.to_string());
    let (prices, _) = fetch_mint_prices_usd(&mints).await;

    let mut pa = prices
        .get(&pool.token_mint_a.to_string())
        .copied()
        .unwrap_or(0.0);
    let mut pb = prices
        .get(&pool.token_mint_b.to_string())
        .copied()
        .unwrap_or(0.0);

    let ma = pool.token_mint_a.to_string();
    let mb = pool.token_mint_b.to_string();
    apply_usdc_pool_price_fallback(&ma, &mb, dec_a, dec_b, pool.tick_current, &mut pa, &mut pb);

    if !pa.is_finite() || !pb.is_finite() || pa <= 0.0 || pb <= 0.0 {
        return Err(ApiError::bad_request(
            "Missing USD price for one or both pool mints; cannot size deposit",
        ));
    }

    let in_range = pool.tick_current >= body.tick_lower && pool.tick_current < body.tick_upper;
    let q = quote_deposit_budget_in_range(
        body.tick_lower,
        body.tick_upper,
        pool.tick_current,
        pool.sqrt_price,
        dec_a,
        dec_b,
        pa,
        pb,
        body.target_usd,
    )
    .map_err(|m| ApiError::bad_request(m.to_string()))?;

    let a_ui = q.amount_a as f64 / 10f64.powi(i32::from(dec_a));
    let b_ui = q.amount_b as f64 / 10f64.powi(i32::from(dec_b));
    Ok(Json(QuoteOpenBudgetResponse {
        token_max_a: q.token_max_a,
        token_max_b: q.token_max_b,
        amount_a: q.amount_a,
        amount_b: q.amount_b,
        amount_a_ui: a_ui,
        amount_b_ui: b_ui,
        estimated_value_usd: q.estimated_value_usd,
        liquidity: q.liquidity.to_string(),
        in_range,
        note: Some(
            "Use token_max_a/b as POST /positions amount_a/b. Estimated USD uses the same mint prices as this quote; on-chain fill may differ slightly."
                .to_string(),
        ),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
    const OTHER_MINT: &str = "B11111111111111111111111111111111111111111";

    #[test]
    fn usdc_pool_fallback_prices_token_a_from_tick_when_token_b_is_usdc() {
        let mut pa = 0.0;
        let mut pb = 0.0;

        apply_usdc_pool_price_fallback(WSOL_MINT, USDC_MINT, 9, 6, -25_250, &mut pa, &mut pb);

        assert_eq!(pb, 1.0);
        assert!(pa.is_finite() && (70.0..90.0).contains(&pa), "pa={pa}");
    }

    #[test]
    fn usdc_pool_fallback_prices_token_b_from_tick_when_token_a_is_usdc() {
        let mut pa = 0.0;
        let mut pb = 0.0;

        apply_usdc_pool_price_fallback(USDC_MINT, WSOL_MINT, 6, 9, 25_250, &mut pa, &mut pb);

        assert_eq!(pa, 1.0);
        assert!(pb.is_finite() && (70.0..90.0).contains(&pb), "pb={pb}");
    }

    #[test]
    fn usdc_pool_fallback_leaves_non_usdc_pair_unpriced() {
        let mut pa = 0.0;
        let mut pb = 0.0;

        apply_usdc_pool_price_fallback(WSOL_MINT, OTHER_MINT, 9, 6, -25_250, &mut pa, &mut pb);

        assert_eq!(pa, 0.0);
        assert_eq!(pb, 0.0);
    }
}
