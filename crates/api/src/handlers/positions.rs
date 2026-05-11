//! Position handlers.

use crate::error::{ApiError, ApiResult};
use crate::handlers::strategies::ensure_strategy_running_after_position_link;
use crate::models::{
    BackfillValuationSnapshotsRequest, BackfillValuationSnapshotsResponse, ClosedPositionEntry,
    ClosedPositionsResponse, DecreaseLiquidityRequest, LinkPositionStrategyRequest,
    ListPositionsResponse, MessageResponse, OpenPositionRequest, PnLResponse,
    PositionDiagnosticsResponse, PositionExperimentConfigResponse, PositionLastEvalSnapshot,
    PositionLifecycleEvent, PositionLifecycleSessionSummary, PositionLifecycleSummaryResponse,
    PositionOpenResponse, PositionResponse, PositionStatus, PositionStrategyDiagnostics,
    PositionStreamLineageResponse, PositionStreamPerformanceResponse, PositionStreamPnLResponse,
    RebalanceRequest, SuggestStrategyLinkResponse, SwapBeforeOpenRequest, SwapBeforeOpenResponse,
    UncollectedFeesInfo,
};
use crate::services::position_stream_lineage::{
    backfill_valuation_snapshots_from_lifecycle_current_prices, compute_position_stream_lineage,
    infer_parent_position_from_lifecycle_best_effort,
};
use crate::services::position_stream_performance::compute_position_stream_performance;
use crate::services::position_stream_pnl::{
    compute_position_stream_pnl, compute_position_stream_pnl_settlement_v1,
};
use crate::services::strategy_service::{
    append_position_address_to_strategy, heal_rotated_strategy_link_best_effort,
    remove_position_address_from_all_strategies,
};
use crate::state::{AppState, PositionUpdate};
use axum::{
    Json,
    extract::{Path, State},
};
use rust_decimal::Decimal;
use rust_decimal::MathematicalOps;
use rust_decimal::prelude::ToPrimitive;
use solana_sdk::program_pack::Pack;
use solana_sdk::pubkey::Pubkey;
use spl_token::state::Mint;
use std::str::FromStr;
use tracing::{info, warn};

use crate::position_registry_seed::{registry_open_position_pubkeys, registry_position_open_map};
use crate::services::PositionService;
use crate::services::position_executor::resolve_executor_for_position_ops;
use crate::services::position_valuation::{
    compute_position_usd_valuation, fetch_prices_for_positions, monitored_position_from_chain,
    range_usdc_and_in_range_for_pool_ticks, refresh_position_fees_from_chain,
    uncollected_fees_info_for_position,
};
use crate::services::price_fetch::fetch_mint_prices_usd;
use axum::extract::Query;
use clmm_lp_domain::math::price_tick::tick_to_price;
use clmm_lp_protocols::ledger::position_registry::registry_path;
use clmm_lp_protocols::ledger::tx_lifecycle::ledger_read_path;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};

fn token_short_label(mint: &str) -> String {
    match mint.trim() {
        "So11111111111111111111111111111111111111112" => "SOL".to_string(),
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" => "USDC".to_string(),
        "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB" => "USDT".to_string(),
        "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs" => "whETH".to_string(),
        _ => mint.trim().to_string(),
    }
}

async fn fetch_mint_decimals_best_effort(
    provider: &clmm_lp_protocols::rpc::RpcProvider,
    mint_s: &str,
) -> Option<u8> {
    let pk = Pubkey::from_str(mint_s).ok()?;
    let account = provider.get_account(&pk).await.ok()?;
    let mint_state = Mint::unpack(&account.data).ok()?;
    Some(mint_state.decimals)
}

#[derive(Debug, Deserialize)]
pub struct CostSessionQuery {
    #[serde(default)]
    cost_session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StreamModeQuery {
    #[serde(default)]
    mode: Option<String>,
}

impl StreamModeQuery {
    fn is_settlement_v1(&self) -> bool {
        self.mode
            .as_deref()
            .map(str::trim)
            .is_some_and(|m| m.eq_ignore_ascii_case("settlement_v1"))
    }
}

/// Best-effort experiment config derived from `registry_open.details` and open-session lifecycle rows.
#[utoipa::path(
    get,
    path = "/positions/{address}/experiment-config",
    tag = "Positions",
    params(
        ("address" = String, Path, description = "Position PDA (base58)")
    ),
    responses(
        (status = 200, description = "Experiment config", body = PositionExperimentConfigResponse),
        (status = 400, description = "Invalid address")
    )
)]
pub async fn get_position_experiment_config(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<PositionExperimentConfigResponse>> {
    let pos = address.trim();
    Pubkey::from_str(pos).map_err(|_| ApiError::bad_request("Invalid position address"))?;

    let path = registry_path();
    if !path.exists() {
        return Ok(Json(PositionExperimentConfigResponse {
            position_address: pos.to_string(),
            open_session_id: None,
            open_details: None,
            tick_lower: None,
            tick_upper: None,
            derived_lower: None,
            derived_upper: None,
            derived_initial_capital_usd: None,
            note: Some(format!(
                "Registry file missing on API host: {}",
                path.display()
            )),
        }));
    }

    let file =
        File::open(&path).map_err(|e| ApiError::internal(format!("open registry file: {e}")))?;
    let reader = BufReader::new(file);

    let mut open_details: Option<serde_json::Value> = None;
    let mut open_sid: Option<String> = None;

    for line in reader.lines().map_while(Result::ok) {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(t) else {
            continue;
        };
        if v.get("event").and_then(|x| x.as_str()) != Some("registry_open") {
            continue;
        }
        let p = v
            .get("position_pubkey")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim();
        if p != pos {
            continue;
        }
        open_sid = v
            .get("rebalance_session_id")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        open_details = v.get("details").cloned();
    }

    let tick_lower = open_details
        .as_ref()
        .and_then(|d| d.get("tick_lower"))
        .and_then(|x| x.as_i64())
        .map(|n| n as i32);
    let tick_upper = open_details
        .as_ref()
        .and_then(|d| d.get("tick_upper"))
        .and_then(|x| x.as_i64())
        .map(|n| n as i32);

    let derived_lower = tick_lower
        .and_then(|t| tick_to_price(t).ok())
        .and_then(|d| d.to_f64());
    let derived_upper = tick_upper
        .and_then(|t| tick_to_price(t).ok())
        .and_then(|d| d.to_f64());

    let derived_initial_capital_usd = if let Some(sid) = open_sid.as_deref() {
        let ledger = ledger_read_path();
        if !ledger.exists() {
            None
        } else {
            let txt = std::fs::read_to_string(&ledger).unwrap_or_default();
            let mut mint_deltas: std::collections::HashMap<String, rust_decimal::Decimal> =
                std::collections::HashMap::new();
            for line in txt.lines() {
                let t = line.trim();
                if t.is_empty() {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<serde_json::Value>(t) else {
                    continue;
                };
                if v.get("rebalance_session_id")
                    .and_then(|x| x.as_str())
                    .map(str::trim)
                    != Some(sid)
                {
                    continue;
                }
                let Some(obj) = v.get("fee_payer_token_deltas").and_then(|x| x.as_object()) else {
                    continue;
                };
                for (mint, dv) in obj {
                    if let Some(s) = dv.as_str()
                        && let Ok(d) = rust_decimal::Decimal::from_str(s.trim())
                    {
                        *mint_deltas
                            .entry(mint.clone())
                            .or_insert(rust_decimal::Decimal::ZERO) += d;
                    }
                }
            }

            // Convert only pool leg mints (A and B) to USD at current free prices.
            // Prefer registry_open.details.pool_address, but for older rows fallback to
            // lifecycle session rows where pool is recorded as `pool_address`.
            let mut pool_address = open_details
                .as_ref()
                .and_then(|d| d.get("pool_address"))
                .and_then(|x| x.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            if pool_address.is_none() {
                for line in txt.lines() {
                    let t = line.trim();
                    if t.is_empty() {
                        continue;
                    }
                    let Ok(v) = serde_json::from_str::<serde_json::Value>(t) else {
                        continue;
                    };
                    if v.get("rebalance_session_id")
                        .and_then(|x| x.as_str())
                        .map(str::trim)
                        != Some(sid)
                    {
                        continue;
                    }
                    pool_address = v
                        .get("pool_pubkey")
                        .and_then(|x| x.as_str())
                        .or_else(|| v.get("pool_address").and_then(|x| x.as_str()))
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                    if pool_address.is_some() {
                        break;
                    }
                }
            }
            let pool_state =
                clmm_lp_protocols::prelude::WhirlpoolReader::new(state.provider.clone())
                    .get_pool_state(pool_address.as_deref().unwrap_or(""))
                    .await;
            // If pool not in details (it often isn't), fall back to resolving by reading last closed list? Keep best-effort.
            let (mint_a, mint_b) = if let Ok(ps) = pool_state {
                (ps.token_mint_a.to_string(), ps.token_mint_b.to_string())
            } else {
                // Fallback: cannot derive without pool mints.
                return Ok(Json(PositionExperimentConfigResponse {
                    position_address: pos.to_string(),
                    open_session_id: open_sid,
                    open_details,
                    tick_lower,
                    tick_upper,
                    derived_lower,
                    derived_upper,
                    derived_initial_capital_usd: None,
                    note: Some("Could not resolve pool mints to convert open-session deltas to USD; derived_initial_capital_usd unavailable.".to_string()),
                }));
            };
            let mut mints = BTreeSet::new();
            mints.insert(mint_a.clone());
            mints.insert(mint_b.clone());
            let (px, _src) = fetch_mint_prices_usd(&mints).await;
            let pa = px.get(&mint_a).copied().unwrap_or(0.0);
            let pb = px.get(&mint_b).copied().unwrap_or(0.0);
            let da = mint_deltas
                .get(&mint_a)
                .cloned()
                .unwrap_or(rust_decimal::Decimal::ZERO);
            let dbb = mint_deltas
                .get(&mint_b)
                .cloned()
                .unwrap_or(rust_decimal::Decimal::ZERO);
            let spend_a = (-da).max(rust_decimal::Decimal::ZERO);
            let spend_b = (-dbb).max(rust_decimal::Decimal::ZERO);
            let usd = spend_a
                * rust_decimal::Decimal::from_f64_retain(pa).unwrap_or(rust_decimal::Decimal::ZERO)
                + spend_b
                    * rust_decimal::Decimal::from_f64_retain(pb)
                        .unwrap_or(rust_decimal::Decimal::ZERO);
            usd.to_f64()
        }
    } else {
        None
    };

    Ok(Json(PositionExperimentConfigResponse {
        position_address: pos.to_string(),
        open_session_id: open_sid,
        open_details,
        tick_lower,
        tick_upper,
        derived_lower,
        derived_upper,
        derived_initial_capital_usd,
        note: None,
    }))
}

#[derive(Debug, Deserialize)]
pub struct ClosedPositionsQuery {
    #[serde(default = "default_closed_limit")]
    limit: usize,
    /// How many newest closed rows to skip (pagination). `0` = newest page.
    #[serde(default)]
    offset: usize,
    /// When `false`, skip Whirlpool pool RPC (no mint labels); registry replay only — fastest.
    #[serde(default = "default_enrich_pools")]
    enrich_pools: bool,
}

fn default_closed_limit() -> usize {
    100
}

fn default_enrich_pools() -> bool {
    true
}

fn clamp_closed_limit(n: usize) -> usize {
    n.clamp(1, 2000)
}

/// List all positions.
#[utoipa::path(
    get,
    path = "/positions",
    tag = "Positions",
    responses(
        (status = 200, description = "List of positions", body = ListPositionsResponse)
    )
)]
pub async fn list_positions(
    State(state): State<AppState>,
) -> ApiResult<Json<ListPositionsResponse>> {
    let mut positions = state.monitor.get_positions().await;

    // If a position is explicitly marked as closed in the registry, do not show it as "monitored/open"
    // even if its account still exists on-chain (e.g. NFT mint metadata persists).
    let reg_state = registry_position_open_map();
    if !reg_state.is_empty() {
        positions.retain(|p| reg_state.get(&p.address).copied().unwrap_or(true));
    }
    let monitored: HashSet<Pubkey> = positions.iter().map(|p| p.address).collect();

    // Registry remembers opens across restarts; monitor can be empty or miss a PDA. Merge chain state
    // for registry opens not yet in monitor so `GET /positions` matches what users see on-chain.
    for pk in registry_open_position_pubkeys() {
        if monitored.contains(&pk) {
            continue;
        }
        match monitored_position_from_chain(state.provider.clone(), &pk).await {
            Ok(p) => {
                positions.push(p);
            }
            Err(e) => {
                warn!(
                    position = %pk,
                    error = %e,
                    "list_positions: registry open but not on-chain or RPC error; skipping"
                );
            }
        }
    }

    let prices = fetch_prices_for_positions(state.provider.clone(), &positions).await;

    let mut responses: Vec<PositionResponse> = Vec::with_capacity(positions.len());
    for p in &positions {
        let valuation =
            match compute_position_usd_valuation(state.provider.clone(), p, &prices).await {
                Ok(v) => Some(v),
                Err(e) => {
                    warn!(
                        position = %p.address,
                        pool = %p.pool,
                        error = %e,
                        "USD valuation failed; falling back to monitor zeros"
                    );
                    None
                }
            };

        let value_usd = valuation
            .as_ref()
            .map(|v| v.value_usd)
            .unwrap_or(p.pnl.current_value_usd);
        let valuation_source = if valuation.is_some() {
            Some("live_valuation".to_string())
        } else {
            Some("fallback_monitor".to_string())
        };
        let fees_usd = valuation
            .as_ref()
            .map(|v| v.fees_usd)
            .unwrap_or(p.pnl.fees_usd);

        let (range_usdc, in_range_fresh) = match valuation.as_ref() {
            Some(v) => (v.range_usdc.as_ref().cloned(), v.in_range),
            None => {
                let (range, in_range) = range_usdc_and_in_range_for_pool_ticks(
                    state.provider.clone(),
                    &p.pool,
                    p.on_chain.tick_lower,
                    p.on_chain.tick_upper,
                )
                .await;
                (range, in_range)
            }
        };

        let base_uncollected_fees = match valuation.as_ref() {
            Some(v) => Some(UncollectedFeesInfo {
                token_a_label: v.token_a_label.clone(),
                token_b_label: v.token_b_label.clone(),
                amount_a: v.fees_owed_a_ui,
                amount_b: v.fees_owed_b_ui,
            }),
            None => uncollected_fees_info_for_position(state.provider.clone(), p).await,
        };

        let (
            token_a_label,
            token_b_label,
            token_mint_a,
            token_mint_b,
            token_price_a_usd,
            token_price_b_usd,
        ) = match valuation.as_ref() {
            Some(v) => (
                Some(v.token_a_label.clone()),
                Some(v.token_b_label.clone()),
                Some(v.token_mint_a.to_string()),
                Some(v.token_mint_b.to_string()),
                Some(v.price_a_usd),
                Some(v.price_b_usd),
            ),
            None => (None, None, None, None, None, None),
        };
        let (range_lower_price, range_upper_price, range_price_quote) = match valuation.as_ref() {
            Some(v) => (
                Some(v.range_price.lower),
                Some(v.range_price.upper),
                Some(v.range_price.quote.clone()),
            ),
            None => (None, None, None),
        };

        // Prefer cached "claimable now" uncollected fees (background refreshed).
        let cached_uncollected = {
            let g = state.uncollected_fees_cache.read().await;
            g.get(&p.address.to_string()).cloned()
        };
        let uncollected_fees = match cached_uncollected {
            Some(c) => {
                // Labels come from valuation if available; otherwise fall back to whatever we already computed.
                let (a_label, b_label) = match valuation.as_ref() {
                    Some(v) => (v.token_a_label.clone(), v.token_b_label.clone()),
                    None => ("A".to_string(), "B".to_string()),
                };
                Some(UncollectedFeesInfo {
                    token_a_label: a_label,
                    token_b_label: b_label,
                    amount_a: c.amount_a,
                    amount_b: c.amount_b,
                })
            }
            None => base_uncollected_fees,
        };

        responses.push(PositionResponse {
            address: p.address.to_string(),
            pool_address: p.pool.to_string(),
            owner: p.on_chain.owner.to_string(),
            tick_lower: p.on_chain.tick_lower,
            tick_upper: p.on_chain.tick_upper,
            range_lower_usdc: range_usdc.as_ref().map(|r| r.lower),
            range_upper_usdc: range_usdc.as_ref().map(|r| r.upper),
            range_usdc_quote: range_usdc.as_ref().map(|r| r.quote.clone()),
            range_lower_price,
            range_upper_price,
            range_price_quote,
            token_a_label,
            token_b_label,
            token_mint_a,
            token_mint_b,
            token_price_a_usd,
            token_price_b_usd,
            uncollected_fees,
            liquidity: p.on_chain.liquidity.to_string(),
            in_range: in_range_fresh,
            value_usd,
            valuation_source,
            pnl: PnLResponse {
                unrealized_pnl_usd: p.pnl.net_pnl_usd,
                unrealized_pnl_pct: p.pnl.net_pnl_pct,
                fees_earned_a: p.pnl.fees_earned_a,
                fees_earned_b: p.pnl.fees_earned_b,
                fees_earned_usd: fees_usd,
                il_pct: p.pnl.il_pct,
                net_pnl_usd: p.pnl.net_pnl_usd,
                net_pnl_pct: p.pnl.net_pnl_pct,
            },
            status: if in_range_fresh {
                PositionStatus::Active
            } else {
                PositionStatus::OutOfRange
            },
            created_at: None,
        });
    }

    Ok(Json(ListPositionsResponse {
        total: responses.len(),
        positions: responses,
    }))
}

/// List closed positions from the append-only registry (`registry.jsonl`).
#[utoipa::path(
    get,
    path = "/positions/closed",
    tag = "Positions",
    params(
        ("limit" = Option<usize>, Query, description = "Max rows (1–2000, default 100)"),
        ("offset" = Option<usize>, Query, description = "How many newest closed positions to skip (pagination)"),
        ("enrich_pools" = Option<bool>, Query, description = "Fetch pool mints for pair labels (default true); false = registry only, no RPC")
    ),
    responses(
        (status = 200, description = "Closed positions (from registry)", body = ClosedPositionsResponse)
    )
)]
pub async fn list_closed_positions(
    State(state): State<AppState>,
    Query(q): Query<ClosedPositionsQuery>,
) -> ApiResult<Json<ClosedPositionsResponse>> {
    let path = registry_path();
    if !path.exists() {
        return Ok(Json(ClosedPositionsResponse {
            total: 0,
            items: Vec::new(),
            note: Some(format!(
                "Registry file missing on API host: {}",
                path.display()
            )),
        }));
    }

    // Replay registry: last event per position wins. Keep last open + last close timestamps for context.
    #[derive(Debug, Clone)]
    struct RowState {
        last_event: String,
        pool: String,
        owner: String,
        close_kind: Option<String>,
        opened_ts: Option<String>,
        closed_ts: Option<String>,
        last_sid: Option<String>,
    }

    let file =
        File::open(&path).map_err(|e| ApiError::internal(format!("open registry file: {e}")))?;
    let reader = BufReader::new(file);

    let mut by_pos: HashMap<String, RowState> = HashMap::new();
    for line in reader.lines().map_while(Result::ok) {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(t) else {
            continue;
        };
        let event = v.get("event").and_then(|x| x.as_str()).unwrap_or("").trim();
        if event != "registry_open" && event != "registry_close" {
            continue;
        }
        let pos = v
            .get("position_pubkey")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim();
        if pos.is_empty() {
            continue;
        }
        let pool = v
            .get("pool_address")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let owner = v
            .get("owner_pubkey")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let ts = v
            .get("ts_utc")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string());
        let sid = v
            .get("rebalance_session_id")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let entry = by_pos.entry(pos.to_string()).or_insert(RowState {
            last_event: event.to_string(),
            pool: pool.clone(),
            owner: owner.clone(),
            close_kind: None,
            opened_ts: None,
            closed_ts: None,
            last_sid: None,
        });
        entry.last_event = event.to_string();
        if !pool.is_empty() {
            entry.pool = pool;
        }
        if !owner.is_empty() {
            entry.owner = owner;
        }
        if let Some(sid) = sid {
            entry.last_sid = Some(sid);
        }
        match event {
            "registry_open" if ts.is_some() => {
                entry.opened_ts = ts;
            }
            "registry_open" => {}
            "registry_close" => {
                if ts.is_some() {
                    entry.closed_ts = ts;
                }
                entry.close_kind = v
                    .get("close_kind")
                    .and_then(|x| x.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
            }
            _ => {}
        }
    }

    let mut closed: Vec<ClosedPositionEntry> = Vec::new();
    for (pos, st) in by_pos {
        if st.last_event != "registry_close" {
            continue;
        }
        closed.push(ClosedPositionEntry {
            position_address: pos,
            pool_address: st.pool,
            token_mint_a: None,
            token_mint_b: None,
            token_a_label: None,
            token_b_label: None,
            owner: st.owner,
            close_kind: st.close_kind,
            opened_ts_utc: st.opened_ts,
            closed_ts_utc: st.closed_ts,
            last_rebalance_session_id: st.last_sid,
        });
    }

    // Newest first if close timestamp is present; else stable string sort.
    closed.sort_by(|a, b| {
        b.closed_ts_utc
            .cmp(&a.closed_ts_utc)
            .then_with(|| a.position_address.cmp(&b.position_address))
    });

    // `closed` is sorted newest-first; skip `offset` from the head, then take up to `limit`.
    let total = closed.len();
    let limit = clamp_closed_limit(q.limit);
    let offset = q.offset.min(total);
    let end = offset.saturating_add(limit).min(total);
    let mut items = if offset < end {
        closed[offset..end].to_vec()
    } else {
        Vec::new()
    };

    let note = if q.enrich_pools {
        None
    } else {
        Some(
            "enrich_pools=false: registry-only rows; pair labels omitted (no pool RPC)."
                .to_string(),
        )
    };

    if q.enrich_pools {
        // Pair labels need pool mints; **only** resolve pools for this page (not every closed row).
        // Previously we did one RPC per registry row before pagination → N× slow and UI 15s timeouts.
        let mut unique_pools: HashSet<String> = HashSet::new();
        for e in &items {
            let p = e.pool_address.trim();
            if !p.is_empty() {
                unique_pools.insert(p.to_string());
            }
        }
        let reader = clmm_lp_protocols::prelude::WhirlpoolReader::new(state.provider.clone());
        let mut pool_mints: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
        for pool in unique_pools {
            let (ma, mb) = match reader.get_pool_state(&pool).await {
                Ok(ps) => (
                    Some(ps.token_mint_a.to_string()),
                    Some(ps.token_mint_b.to_string()),
                ),
                Err(_) => (None, None),
            };
            pool_mints.insert(pool, (ma, mb));
        }
        for e in &mut items {
            let p = e.pool_address.trim();
            if p.is_empty() {
                continue;
            }
            if let Some((ma, mb)) = pool_mints.get(p) {
                e.token_mint_a = ma.clone();
                e.token_mint_b = mb.clone();
                e.token_a_label = ma.as_deref().map(token_short_label);
                e.token_b_label = mb.as_deref().map(token_short_label);
            }
        }
    }

    Ok(Json(ClosedPositionsResponse { total, items, note }))
}

/// Get lifecycle summary for a position: group lifecycle ledger rows by session id and compute aggregates.
#[utoipa::path(
    get,
    path = "/positions/{address}/lifecycle-summary",
    tag = "Positions",
    params(
        ("address" = String, Path, description = "Position PDA (base58)"),
    ),
    responses(
        (status = 200, description = "Lifecycle breakdown + aggregates", body = PositionLifecycleSummaryResponse),
        (status = 400, description = "Invalid address")
    )
)]
pub async fn get_position_lifecycle_summary(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<PositionLifecycleSummaryResponse>> {
    let pos = address.trim();
    Pubkey::from_str(pos).map_err(|_| ApiError::bad_request("Invalid position address"))?;

    // Reuse stream connectivity (IL edges + sessions). If DB disabled, this returns just the PDA.
    let perf = compute_position_stream_performance(&state, pos, false).await?;
    let positions = perf.positions.clone();
    let sessions = perf.sessions.clone();

    let ledger = ledger_read_path();
    if !ledger.exists() {
        return Ok(Json(PositionLifecycleSummaryResponse {
            position_address: pos.to_string(),
            positions,
            sessions,
            total_tx_fee_lamports: 0,
            total_tx_fee_usd: Decimal::ZERO,
            collect_events: 0,
            collected_fee_token_a_ui: None,
            collected_fee_token_a_raw: None,
            collected_fee_token_b_ui: None,
            collected_fee_token_b_raw: None,
            collected_fees_usd: None,
            realized_cashflow_usd: Decimal::ZERO,
            session_summaries: Vec::new(),
            note: Some(format!(
                "Lifecycle ledger file missing on API host: {}",
                ledger.display()
            )),
        }));
    }

    // Parse lifecycle JSONL and keep rows that match either a stream session id or any position pubkey.
    let txt = std::fs::read_to_string(&ledger)
        .map_err(|e| ApiError::internal(format!("read lifecycle ledger: {e}")))?;
    let mut grouped: HashMap<String, Vec<PositionLifecycleEvent>> = HashMap::new();

    let mut total_tx_fee_lamports: u64 = 0;
    let mut collect_events: u32 = 0;
    let mut collected_a_ui: Decimal = Decimal::ZERO;
    let mut collected_b_ui: Decimal = Decimal::ZERO;
    let mut any_collected_a = false;
    let mut any_collected_b = false;
    let mut collected_fee_token_a_raw: Option<u64> = None;
    let mut collected_fee_token_b_raw: Option<u64> = None;

    // Realized cashflow: sum of fee_payer_token_deltas for pool leg mints requires mint prices.
    // We can only do that reliably when we know pool leg mints; use stream PnL endpoint (baseline snapshot) when available.
    // Best-effort fallback: keep it 0 with a note if prices cannot be resolved.
    let mut mint_deltas_sum: HashMap<String, Decimal> = HashMap::new();
    let mut pool_mints_by_pool: HashMap<String, (String, String)> = HashMap::new();
    let mut mints_for_pricing: BTreeSet<String> = BTreeSet::new();

    for line in txt.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(t) else {
            continue;
        };
        let session_id = v
            .get("rebalance_session_id")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let position_pubkey = v
            .get("position_pubkey")
            .and_then(|x| x.as_str())
            .or_else(|| v.get("position_pda").and_then(|x| x.as_str()))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let matches = lifecycle_row_matches_stream_members(
            session_id.as_deref(),
            position_pubkey.as_deref(),
            &sessions,
            &positions,
        );
        if !matches {
            continue;
        }

        let key = session_id
            .clone()
            .unwrap_or_else(|| "_no_session".to_string());

        let tx_fee_lamports = v.get("tx_fee_lamports").and_then(|x| x.as_u64());
        if let Some(f) = tx_fee_lamports {
            total_tx_fee_lamports = total_tx_fee_lamports.saturating_add(f);
        }

        let event_s = v
            .get("event")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string());
        let is_collect = event_s.as_deref() == Some("bot_collect_fees");
        if is_collect {
            collect_events = collect_events.saturating_add(1);
        }

        if let Some(deltas) = v.get("fee_payer_token_deltas").cloned()
            && let Some(obj) = deltas.as_object()
        {
            // If this is a collect-fees tx, derive per-leg collected amounts by mint.
            if is_collect {
                let pool_addr = v
                    .get("pool_address")
                    .and_then(|x| x.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                if let Some(pool_addr) = pool_addr {
                    let (mint_a, mint_b) =
                        if let Some(pair) = pool_mints_by_pool.get(&pool_addr).cloned() {
                            pair
                        } else {
                            // Best-effort: resolve pool mints on demand (free RPC).
                            match clmm_lp_protocols::prelude::WhirlpoolReader::new(
                                state.provider.clone(),
                            )
                            .get_pool_state(&pool_addr)
                            .await
                            {
                                Ok(ps) => {
                                    let a = ps.token_mint_a.to_string();
                                    let b = ps.token_mint_b.to_string();
                                    pool_mints_by_pool
                                        .insert(pool_addr.clone(), (a.clone(), b.clone()));
                                    (a, b)
                                }
                                Err(_) => (String::new(), String::new()),
                            }
                        };

                    if !mint_a.is_empty()
                        && let Some(dv) = obj.get(&mint_a)
                        && let Some(s) = dv.as_str()
                        && let Ok(d) = Decimal::from_str(s.trim())
                        && d > Decimal::ZERO
                    {
                        collected_a_ui += d;
                        any_collected_a = true;
                        mints_for_pricing.insert(mint_a.clone());
                    }
                    if !mint_b.is_empty()
                        && let Some(dv) = obj.get(&mint_b)
                        && let Some(s) = dv.as_str()
                        && let Ok(d) = Decimal::from_str(s.trim())
                        && d > Decimal::ZERO
                    {
                        collected_b_ui += d;
                        any_collected_b = true;
                        mints_for_pricing.insert(mint_b.clone());
                    }
                }
            }
            for (mint, dv) in obj {
                if let Some(s) = dv.as_str()
                    && let Ok(d) = Decimal::from_str(s.trim())
                {
                    let e = mint_deltas_sum.entry(mint.clone()).or_insert(Decimal::ZERO);
                    *e += d;
                }
            }
        }

        let ev = PositionLifecycleEvent {
            ts_utc: v
                .get("ts_utc")
                .and_then(|x| x.as_str())
                .map(|s| s.trim().to_string()),
            source: v
                .get("source")
                .and_then(|x| x.as_str())
                .map(|s| s.trim().to_string()),
            event: event_s,
            operation: v
                .get("operation")
                .and_then(|x| x.as_str())
                .map(|s| s.trim().to_string()),
            signature: v
                .get("signature")
                .and_then(|x| x.as_str())
                .map(|s| s.trim().to_string()),
            pool_address: v
                .get("pool_address")
                .and_then(|x| x.as_str())
                .map(|s| s.trim().to_string()),
            position_pubkey,
            rebalance_session_id: session_id,
            tx_fee_lamports,
            fee_payer_net_lamports_delta: v
                .get("fee_payer_net_lamports_delta")
                .and_then(|x| x.as_i64()),
            fee_payer_token_deltas: v.get("fee_payer_token_deltas").cloned(),
        };
        grouped.entry(key).or_default().push(ev);
    }

    // Session summaries: sort each session by ts_utc string (RFC3339, lexicographic works).
    let mut session_summaries: Vec<PositionLifecycleSessionSummary> = Vec::new();
    for (sid, mut events) in grouped {
        events.sort_by(|a, b| a.ts_utc.cmp(&b.ts_utc));
        let mut sum_fee: u64 = 0;
        let mut rebalance_related: u32 = 0;
        for e in &events {
            if let Some(f) = e.tx_fee_lamports {
                sum_fee = sum_fee.saturating_add(f);
            }
            if let Some(ref ev) = e.event
                && (ev.contains("rebalance")
                    || ev.contains("open_position")
                    || ev.contains("close_position"))
            {
                rebalance_related = rebalance_related.saturating_add(1);
            }
        }
        session_summaries.push(PositionLifecycleSessionSummary {
            session_id: sid,
            events,
            total_tx_fee_lamports: sum_fee,
            rebalance_related_events: rebalance_related,
        });
    }
    session_summaries.sort_by(|a, b| b.session_id.cmp(&a.session_id));

    // Convert tx fee lamports to USD using free price fetch (SOL mint).
    const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
    let mut mints = BTreeSet::new();
    mints.insert(WSOL_MINT.to_string());
    let (px, src) = fetch_mint_prices_usd(&mints).await;
    let sol_usd = px.get(WSOL_MINT).copied().unwrap_or(0.0);
    let total_tx_fee_usd = if sol_usd > 0.0 {
        Decimal::from_f64_retain((total_tx_fee_lamports as f64 / 1e9) * sol_usd)
            .unwrap_or(Decimal::ZERO)
    } else {
        Decimal::ZERO
    };

    // Best-effort: collected LP fees in USD (A/B legs at current mint USD prices).
    //
    // We only compute USD when all collect rows point to the same pool (so token A/B identity is stable).
    let collected_fees_usd =
        if pool_mints_by_pool.len() == 1 && (any_collected_a || any_collected_b) {
            let (_pool, (ma, mb)) = pool_mints_by_pool.iter().next().unwrap();
            let mut mint_set = BTreeSet::new();
            mint_set.insert(ma.clone());
            mint_set.insert(mb.clone());
            let (px2, _src2) = fetch_mint_prices_usd(&mint_set).await;
            let pa = px2.get(ma).copied().unwrap_or(0.0);
            let pb = px2.get(mb).copied().unwrap_or(0.0);
            let pa_d = Decimal::from_f64_retain(pa).unwrap_or(Decimal::ZERO);
            let pb_d = Decimal::from_f64_retain(pb).unwrap_or(Decimal::ZERO);

            // Also expose collected fees in smallest units (raw/base units), so UI can always sanity-check
            // amounts even when USD pricing is unavailable.
            let decimal_ui_to_raw_u64 = |v: Decimal, decimals: u8| -> Option<u64> {
                if v <= Decimal::ZERO {
                    return Some(0);
                }
                let scale = Decimal::from(10u64).checked_powu(u64::from(decimals))?;
                (v * scale).round().to_u64()
            };

            let dec_a = fetch_mint_decimals_best_effort(&state.provider, ma).await;
            let dec_b = fetch_mint_decimals_best_effort(&state.provider, mb).await;
            collected_fee_token_a_raw = dec_a.and_then(|d| {
                any_collected_a
                    .then_some(collected_a_ui)
                    .and_then(|x| decimal_ui_to_raw_u64(x, d))
            });
            collected_fee_token_b_raw = dec_b.and_then(|d| {
                any_collected_b
                    .then_some(collected_b_ui)
                    .and_then(|x| decimal_ui_to_raw_u64(x, d))
            });

            Some(collected_a_ui * pa_d + collected_b_ui * pb_d)
        } else {
            None
        };

    // Realized cashflow USD: best-effort using stream PnL (it knows baseline mints), otherwise 0.
    let pnl = compute_position_stream_pnl(&state, pos).await.ok();
    let realized_cashflow_usd = pnl
        .as_ref()
        .map(|p| p.realized_cashflow_usd)
        .unwrap_or(Decimal::ZERO);

    let note = Some(format!(
        "Lifecycle summary is best-effort from lifecycle JSONL. tx fees use SOL/USD ({src}). collected_fees (if present) use positive token deltas from bot_collect_fees × current mint USD. realized_cashflow is sourced from stream PnL when available."
    ));

    Ok(Json(PositionLifecycleSummaryResponse {
        position_address: pos.to_string(),
        positions,
        sessions,
        total_tx_fee_lamports,
        total_tx_fee_usd,
        collect_events,
        collected_fee_token_a_ui: any_collected_a.then_some(collected_a_ui),
        collected_fee_token_a_raw,
        collected_fee_token_b_ui: any_collected_b.then_some(collected_b_ui),
        collected_fee_token_b_raw,
        collected_fees_usd,
        realized_cashflow_usd,
        session_summaries,
        note,
    }))
}

fn lifecycle_row_matches_stream_members(
    session_id: Option<&str>,
    position_pubkey: Option<&str>,
    sessions: &[String],
    positions: &[String],
) -> bool {
    let matches_session = session_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_some_and(|sid| sessions.iter().any(|x| x == sid));
    let matches_position = position_pubkey
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_some_and(|p| positions.iter().any(|x| x == p));
    matches_session || matches_position
}

#[cfg(test)]
mod lifecycle_summary_tests {
    use super::lifecycle_row_matches_stream_members;

    #[test]
    fn lifecycle_row_matches_by_position_even_when_session_unknown() {
        let sessions = vec!["session-known".to_string()];
        let positions = vec!["DfjqibKyfMtXqkZrfsfmWvbxZxdZTH6m6J1L5qKnv4Xq".to_string()];

        let matched = lifecycle_row_matches_stream_members(
            Some("session-external"),
            Some("DfjqibKyfMtXqkZrfsfmWvbxZxdZTH6m6J1L5qKnv4Xq"),
            &sessions,
            &positions,
        );

        assert!(matched);
    }

    #[test]
    fn lifecycle_row_does_not_match_unrelated_session_and_position() {
        let sessions = vec!["session-known".to_string()];
        let positions = vec!["DfjqibKyfMtXqkZrfsfmWvbxZxdZTH6m6J1L5qKnv4Xq".to_string()];

        let matched = lifecycle_row_matches_stream_members(
            Some("session-external"),
            Some("11111111111111111111111111111111"),
            &sessions,
            &positions,
        );

        assert!(!matched);
    }
}

/// Get a specific position.
#[utoipa::path(
    get,
    path = "/positions/{address}",
    tag = "Positions",
    params(
        ("address" = String, Path, description = "Position address")
    ),
    responses(
        (status = 200, description = "Position details", body = PositionResponse),
        (status = 400, description = "Invalid address or not a position account"),
        (status = 404, description = "Position account absent on-chain for this RPC cluster"),
        (status = 502, description = "RPC/upstream error while fetching position")
    )
)]
pub async fn get_position(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<PositionResponse>> {
    let pubkey = Pubkey::from_str(&address)
        .map_err(|_| ApiError::bad_request("Invalid position address"))?;

    let positions = state.monitor.get_positions().await;
    let mut position = if let Some(p) = positions.iter().find(|p| p.address == pubkey) {
        p.clone()
    } else {
        let p = monitored_position_from_chain(state.provider.clone(), &pubkey).await?;
        let st = state.clone();
        let addr = address.clone();
        tokio::spawn(async move {
            if let Err(e) = st.monitor.add_position(&addr).await {
                warn!(
                    error = %e,
                    position = %addr,
                    "get_position fallback: monitor.add_position failed (detail still returned)"
                );
            }
        });
        p
    };

    refresh_position_fees_from_chain(state.provider.clone(), &mut position).await;

    let prices =
        fetch_prices_for_positions(state.provider.clone(), std::slice::from_ref(&position)).await;
    let valuation = compute_position_usd_valuation(state.provider.clone(), &position, &prices)
        .await
        .ok();
    if let (Some(db), Some(v)) = (state.db.as_ref(), valuation.as_ref()) {
        // Best-effort snapshot for stream PnL/IL across rotated PDAs.
        let raw = serde_json::json!({
            "position": position.address.to_string(),
            "pool": position.pool.to_string(),
            "value_usd": v.value_usd,
            "fees_usd": v.fees_usd,
            "amount_a_ui": v.amount_a_ui,
            "amount_b_ui": v.amount_b_ui,
            "token_mint_a": v.token_mint_a.to_string(),
            "token_mint_b": v.token_mint_b.to_string(),
            "price_a_usd": v.price_a_usd,
            "price_b_usd": v.price_b_usd,
            "source": "get_position"
        });
        let _ = sqlx::query(
            r#"
            INSERT INTO position_stream_valuation_snapshots
              (position_pubkey, ts_utc, pool_pubkey, value_usd, amount_a_ui, amount_b_ui, fees_usd, token_mint_a, token_mint_b, price_a_usd, price_b_usd, price_source, raw_json)
            VALUES ($1, NOW(), $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            ON CONFLICT (position_pubkey, ts_utc) DO NOTHING
            "#,
        )
        .bind(position.address.to_string())
        .bind(position.pool.to_string())
        .bind(v.value_usd)
        .bind(v.amount_a_ui)
        .bind(v.amount_b_ui)
        .bind(v.fees_usd)
        .bind(v.token_mint_a.to_string())
        .bind(v.token_mint_b.to_string())
        .bind(Decimal::from_f64_retain(v.price_a_usd).unwrap_or(Decimal::ZERO))
        .bind(Decimal::from_f64_retain(v.price_b_usd).unwrap_or(Decimal::ZERO))
        .bind("free_prices")
        .bind(raw)
        .execute(db.pool())
        .await;
    }
    let value_usd = valuation
        .as_ref()
        .map(|v| v.value_usd)
        .unwrap_or(position.pnl.current_value_usd);
    let valuation_source = if valuation.is_some() {
        Some("live_valuation".to_string())
    } else {
        Some("fallback_monitor".to_string())
    };
    let fees_usd = valuation
        .as_ref()
        .map(|v| v.fees_usd)
        .unwrap_or(position.pnl.fees_usd);

    let (range_usdc, in_range_fresh) = match valuation.as_ref() {
        Some(v) => (v.range_usdc.as_ref().cloned(), v.in_range),
        None => {
            let (range, in_range) = range_usdc_and_in_range_for_pool_ticks(
                state.provider.clone(),
                &position.pool,
                position.on_chain.tick_lower,
                position.on_chain.tick_upper,
            )
            .await;
            (range, in_range)
        }
    };

    let uncollected_fees = match valuation.as_ref() {
        Some(v) => Some(UncollectedFeesInfo {
            token_a_label: v.token_a_label.clone(),
            token_b_label: v.token_b_label.clone(),
            amount_a: v.fees_owed_a_ui,
            amount_b: v.fees_owed_b_ui,
        }),
        None => uncollected_fees_info_for_position(state.provider.clone(), &position).await,
    };

    let (
        token_a_label,
        token_b_label,
        token_mint_a,
        token_mint_b,
        token_price_a_usd,
        token_price_b_usd,
    ) = match valuation.as_ref() {
        Some(v) => (
            Some(v.token_a_label.clone()),
            Some(v.token_b_label.clone()),
            Some(v.token_mint_a.to_string()),
            Some(v.token_mint_b.to_string()),
            Some(v.price_a_usd),
            Some(v.price_b_usd),
        ),
        None => (None, None, None, None, None, None),
    };
    let (range_lower_price, range_upper_price, range_price_quote) = match valuation.as_ref() {
        Some(v) => (
            Some(v.range_price.lower),
            Some(v.range_price.upper),
            Some(v.range_price.quote.clone()),
        ),
        None => (None, None, None),
    };

    let response = PositionResponse {
        address: position.address.to_string(),
        pool_address: position.pool.to_string(),
        owner: position.on_chain.owner.to_string(),
        tick_lower: position.on_chain.tick_lower,
        tick_upper: position.on_chain.tick_upper,
        range_lower_usdc: range_usdc.as_ref().map(|r| r.lower),
        range_upper_usdc: range_usdc.as_ref().map(|r| r.upper),
        range_usdc_quote: range_usdc.as_ref().map(|r| r.quote.clone()),
        range_lower_price,
        range_upper_price,
        range_price_quote,
        token_a_label,
        token_b_label,
        token_mint_a,
        token_mint_b,
        token_price_a_usd,
        token_price_b_usd,
        uncollected_fees,
        liquidity: position.on_chain.liquidity.to_string(),
        in_range: in_range_fresh,
        value_usd,
        valuation_source,
        pnl: PnLResponse {
            unrealized_pnl_usd: position.pnl.net_pnl_usd,
            unrealized_pnl_pct: position.pnl.net_pnl_pct,
            fees_earned_a: position.pnl.fees_earned_a,
            fees_earned_b: position.pnl.fees_earned_b,
            fees_earned_usd: fees_usd,
            il_pct: position.pnl.il_pct,
            net_pnl_usd: position.pnl.net_pnl_usd,
            net_pnl_pct: position.pnl.net_pnl_pct,
        },
        status: if in_range_fresh {
            PositionStatus::Active
        } else {
            PositionStatus::OutOfRange
        },
        created_at: None,
    };

    Ok(Json(response))
}

async fn linked_strategies_for_position_diagnostics(
    state: &AppState,
    pubkey: &Pubkey,
    address_trim: &str,
) -> Vec<PositionStrategyDiagnostics> {
    let strategies = state.strategies.read().await;
    let mut linked: Vec<PositionStrategyDiagnostics> = Vec::new();
    for s in strategies.values() {
        let params = s.config.get("parameters");
        let position_addresses = params
            .and_then(|p| p.get("position_addresses"))
            .and_then(|v| v.as_array());
        let is_linked = position_addresses.is_some_and(|arr| {
            arr.iter()
                .any(|v| v.as_str().map(str::trim) == Some(address_trim))
        });
        if !is_linked {
            continue;
        }

        let disabled = params
            .and_then(|p| p.get("executor_disabled_position_addresses"))
            .and_then(|v| v.as_array())
            .is_some_and(|arr| {
                arr.iter()
                    .any(|v| v.as_str().map(str::trim) == Some(address_trim))
            });

        let strategy_type = s
            .config
            .get("strategy_type")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or(crate::models::StrategyType::StaticRange);
        let dry_run = s
            .config
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let auto_execute = s
            .config
            .get("auto_execute")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let last_eval = if s.running {
            let exec_opt = { state.executors.read().await.get(&s.id).cloned() };
            if let Some(exec) = exec_opt {
                let g = exec.read().await;
                g.last_evaluation_for_position(pubkey)
                    .await
                    .map(|snap| PositionLastEvalSnapshot {
                        ts_utc: snap.ts_utc,
                        in_range: snap.in_range,
                        pool_tick_current: snap.pool_tick_current,
                        decision: snap.decision,
                        requires_transaction: snap.requires_transaction,
                        auto_execute: snap.auto_execute,
                        hours_since_rebalance: snap.hours_since_rebalance,
                        minutes_since_rebalance: snap.minutes_since_rebalance,
                    })
            } else {
                None
            }
        } else {
            None
        };

        linked.push(PositionStrategyDiagnostics {
            strategy_id: s.id.clone(),
            name: s.name.clone(),
            strategy_type,
            running: s.running,
            dry_run,
            auto_execute,
            automation_disabled_for_position: disabled,
            last_eval,
        });
    }
    linked
}

/// Get "why didn't this position rebalance?" diagnostics.
#[utoipa::path(
    get,
    path = "/positions/{address}/diagnostics",
    tag = "Positions",
    params(
        ("address" = String, Path, description = "Position address")
    ),
    responses(
        (status = 200, description = "Diagnostics", body = PositionDiagnosticsResponse),
        (status = 400, description = "Invalid address")
    )
)]
pub async fn get_position_diagnostics(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<PositionDiagnosticsResponse>> {
    let pubkey = Pubkey::from_str(&address)
        .map_err(|_| ApiError::bad_request("Invalid position address"))?;

    let monitored = state.monitor.get_position(&pubkey).await;
    let in_monitor = monitored.is_some();
    let monitor_in_range = monitored.as_ref().map(|p| p.in_range);

    let address_trim = address.trim();
    let linked = linked_strategies_for_position_diagnostics(&state, &pubkey, address_trim).await;

    Ok(Json(PositionDiagnosticsResponse {
        address,
        in_monitor,
        monitor_in_range,
        linked_strategies: linked,
    }))
}

/// Explicit, opt-in repair for strategy link after a close->open rotation.
#[utoipa::path(
    post,
    path = "/positions/{address}/heal-strategy-link",
    tag = "Positions",
    params(
        ("address" = String, Path, description = "Position PDA (base58)")
    ),
    responses(
        (status = 200, description = "Heal result", body = MessageResponse),
        (status = 400, description = "Invalid address")
    )
)]
pub async fn heal_position_strategy_link(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<MessageResponse>> {
    let pda = address.trim();
    Pubkey::from_str(pda).map_err(|_| ApiError::bad_request("Invalid position address"))?;
    match heal_rotated_strategy_link_best_effort(&state, pda).await {
        Ok(Some(sids)) if !sids.is_empty() => Ok(Json(MessageResponse::new(format!(
            "Strategy link healed for {pda}. Updated strategies: {}",
            sids.join(", ")
        )))),
        Ok(_) => Ok(Json(MessageResponse::new(format!(
            "No strategy-link heal needed for {pda}."
        )))),
        Err(e) => Err(ApiError::internal(format!(
            "heal strategy link failed: {e}"
        ))),
    }
}

/// Get "stream" performance aggregates for a position PDA (across rotated PDAs).
#[utoipa::path(
    get,
    path = "/positions/{address}/stream-performance",
    tag = "Positions",
    params(
        ("address" = String, Path, description = "Position PDA (base58)")
    ),
    responses(
        (status = 200, description = "Stream performance", body = PositionStreamPerformanceResponse),
        (status = 400, description = "Invalid address")
    )
)]
pub async fn get_position_stream_performance(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<PositionStreamPerformanceResponse>> {
    let pos = address.trim();
    Pubkey::from_str(pos).map_err(|_| ApiError::bad_request("Invalid position address"))?;
    let resp = compute_position_stream_performance(&state, pos, false).await?;
    Ok(Json(resp))
}

/// Get stream-level Net PnL / IL for a position PDA (across rotated PDAs).
#[utoipa::path(
    get,
    path = "/positions/{address}/stream-pnl",
    tag = "Positions",
    params(
        ("address" = String, Path, description = "Position PDA (base58)")
    ),
    responses(
        (status = 200, description = "Stream PnL/IL", body = PositionStreamPnLResponse),
        (status = 400, description = "Invalid address")
    )
)]
pub async fn get_position_stream_pnl(
    State(state): State<AppState>,
    Path(address): Path<String>,
    Query(mode_q): Query<StreamModeQuery>,
) -> ApiResult<Json<PositionStreamPnLResponse>> {
    let pos = address.trim();
    Pubkey::from_str(pos).map_err(|_| ApiError::bad_request("Invalid position address"))?;
    let resp = if mode_q.is_settlement_v1() {
        compute_position_stream_pnl_settlement_v1(&state, pos).await?
    } else {
        compute_position_stream_pnl(&state, pos).await?
    };
    Ok(Json(resp))
}

/// Get ordered stream lineage + per-node metrics for a position PDA.
#[utoipa::path(
    get,
    path = "/positions/{address}/stream-lineage",
    tag = "Positions",
    params(
        ("address" = String, Path, description = "Position PDA (base58)")
    ),
    responses(
        (status = 200, description = "Stream lineage (ordered chain + per-node metrics)", body = PositionStreamLineageResponse),
        (status = 400, description = "Invalid address")
    )
)]
pub async fn get_position_stream_lineage(
    State(state): State<AppState>,
    Path(address): Path<String>,
    Query(mode_q): Query<StreamModeQuery>,
) -> ApiResult<Json<PositionStreamLineageResponse>> {
    let pos = address.trim();
    Pubkey::from_str(pos).map_err(|_| ApiError::bad_request("Invalid position address"))?;
    let mut resp = compute_position_stream_lineage(&state, pos).await?;
    if mode_q.is_settlement_v1() {
        resp.totals = Some(compute_position_stream_pnl_settlement_v1(&state, pos).await?);
        let mut note = resp.note.unwrap_or_default();
        if !note.is_empty() {
            note.push(' ');
        }
        note.push_str(
            "Settlement v1 mode: totals are computed from persisted DB snapshots only (no live self-seed).",
        );
        resp.note = Some(note);
    }
    Ok(Json(resp))
}

/// Backfill synthetic DB valuation snapshots from lifecycle JSONL using current free prices.
#[utoipa::path(
    post,
    path = "/positions/backfill-valuation-snapshots",
    tag = "Positions",
    request_body = BackfillValuationSnapshotsRequest,
    responses(
        (status = 200, description = "Backfill executed", body = BackfillValuationSnapshotsResponse),
        (status = 503, description = "DB disabled")
    )
)]
pub async fn backfill_valuation_snapshots(
    State(state): State<AppState>,
    Json(request): Json<BackfillValuationSnapshotsRequest>,
) -> ApiResult<Json<BackfillValuationSnapshotsResponse>> {
    let resp = backfill_valuation_snapshots_from_lifecycle_current_prices(&state, &request).await?;
    Ok(Json(resp))
}

/// Executes an Orca Whirlpool swap (ExactIn) inside the same pool (SWAP-only step).
#[utoipa::path(
    post,
    path = "/positions/swap-before-open",
    tag = "Positions",
    request_body = SwapBeforeOpenRequest,
    responses(
        (status = 200, description = "Swap executed", body = SwapBeforeOpenResponse),
        (status = 400, description = "Invalid request")
    )
)]
pub async fn swap_before_open(
    State(state): State<AppState>,
    Json(request): Json<SwapBeforeOpenRequest>,
) -> ApiResult<Json<SwapBeforeOpenResponse>> {
    info!(
        pool = %request.pool_address,
        specified_mint = %request.specified_mint,
        amount_in = request.amount_in,
        dry_run = state.dry_run,
        "Swapping before open (swap-only step)"
    );

    let mut svc = PositionService::new(state.clone());
    svc.set_dry_run(state.dry_run);

    if !state.dry_run
        && let Some(exec) = resolve_executor_for_position_ops(&state).await
    {
        svc.set_executor(exec);
    }

    let op = svc.swap_before_open_exact_in(&request).await?;

    if op.success {
        let data = op.data.as_ref();
        let message = data
            .and_then(|d| d.get("message"))
            .and_then(|v| v.as_str())
            .unwrap_or("Swap executed")
            .to_string();

        let swap_signature = data
            .and_then(|d| d.get("swap_signature"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string);

        let cost_session_id = request
            .cost_session_id
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        Ok(Json(SwapBeforeOpenResponse {
            message,
            swap_signature,
            cost_session_id,
        }))
    } else {
        Err(ApiError::ServiceUnavailable(
            op.error.unwrap_or_else(|| "Swap failed".to_string()),
        ))
    }
}

/// Open a new position.
#[utoipa::path(
    post,
    path = "/positions",
    tag = "Positions",
    request_body = OpenPositionRequest,
    responses(
        (status = 201, description = "Position opened", body = PositionOpenResponse),
        (status = 400, description = "Invalid request")
    )
)]
pub async fn open_position(
    State(state): State<AppState>,
    Json(request): Json<OpenPositionRequest>,
) -> ApiResult<Json<PositionOpenResponse>> {
    let strategy_id = request.strategy_id.clone();
    info!(
        pool = %request.pool_address,
        tick_lower = request.tick_lower,
        tick_upper = request.tick_upper,
        dry_run = state.dry_run,
        strategy_id = ?strategy_id.as_deref(),
        "Opening position"
    );

    if let Some(ref sid) = strategy_id {
        let strategies = state.strategies.read().await;
        strategies
            .get(sid)
            .ok_or_else(|| ApiError::not_found("Strategy not found"))?;
    }

    let mut svc = PositionService::new(state.clone());
    svc.set_dry_run(state.dry_run);

    // Non-dry-run: strategy executor or lazy KEYPAIR_PATH executor (swap/open work without a running strategy).
    if !state.dry_run
        && let Some(exec) = resolve_executor_for_position_ops(&state).await
    {
        svc.set_executor(exec);
    }

    let cost_session_id = request
        .cost_session_id
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let op = svc.open_position(&request).await?;
    if op.success {
        let data = op.data.as_ref();
        let position_pda_opt = data
            .and_then(|d| d.get("position_pda"))
            .and_then(|v| v.as_str());

        // New positions exist on-chain but `GET /positions/:addr` reads the in-memory monitor first.
        // Without this, the dashboard shows "Position not found" until API restart + registry seed.
        if !state.dry_run
            && let Some(pda) = position_pda_opt
            && let Err(e) = state.monitor.add_position(pda).await
        {
            warn!(
                error = %e,
                position = %pda,
                "open_position: monitor.add_position failed (detail may 404 until retry)"
            );
        }

        // Legacy: response with only `message` and no PDA (avoid swallowing idempotent replay which has both).
        if let Some(m) = data.and_then(|d| d.get("message")).and_then(|v| v.as_str())
            && position_pda_opt.is_none()
        {
            return Ok(Json(PositionOpenResponse {
                message: m.to_string(),
                position_pda: None,
                swap_signature: None,
                cost_session_id,
            }));
        }
        if let Some(pda) = position_pda_opt {
            let swap_signature = data
                .and_then(|d| d.get("swap_signature"))
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string);
            let resp_cost_session = data
                .and_then(|d| d.get("cost_session_id"))
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string)
                .or_else(|| cost_session_id.clone());

            if let Some(ref sid) = strategy_id {
                append_position_address_to_strategy(&state, sid, pda).await?;
                match ensure_strategy_running_after_position_link(&state, sid, pda).await {
                    Ok(None) => {
                        let mut msg = format!("Position opened. PDA: {pda}");
                        msg.push_str(" — linked to strategy; automation started.");
                        return Ok(Json(PositionOpenResponse {
                            message: msg,
                            position_pda: Some(pda.to_string()),
                            swap_signature,
                            cost_session_id: resp_cost_session,
                        }));
                    }
                    Ok(Some(w)) => {
                        let mut msg = format!("Position opened. PDA: {pda}");
                        msg.push_str(" — linked to strategy; automation started. Note: ");
                        msg.push_str(&w);
                        return Ok(Json(PositionOpenResponse {
                            message: msg,
                            position_pda: Some(pda.to_string()),
                            swap_signature,
                            cost_session_id: resp_cost_session,
                        }));
                    }
                    Err(e) => {
                        warn!(
                            error = %e,
                            strategy_id = %sid,
                            "Could not start strategy automation after position link"
                        );
                        let mut msg = format!("Position opened. PDA: {pda}");
                        msg.push_str(&format!(
                            " — linked to strategy; automation could not start: {}.",
                            e
                        ));
                        return Ok(Json(PositionOpenResponse {
                            message: msg,
                            position_pda: Some(pda.to_string()),
                            swap_signature,
                            cost_session_id: resp_cost_session,
                        }));
                    }
                }
            }
            let msg = data
                .and_then(|d| d.get("message"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("Position opened. PDA: {pda}"));
            return Ok(Json(PositionOpenResponse {
                message: msg,
                position_pda: Some(pda.to_string()),
                swap_signature,
                cost_session_id: resp_cost_session,
            }));
        }
        return Ok(Json(PositionOpenResponse {
            message: "Position opened".to_string(),
            position_pda: None,
            swap_signature: None,
            cost_session_id,
        }));
    }

    Err(ApiError::ServiceUnavailable(
        op.error
            .unwrap_or_else(|| "Position opening failed".to_string()),
    ))
}

/// Close a position.
#[utoipa::path(
    delete,
    path = "/positions/{address}",
    tag = "Positions",
    params(
        ("address" = String, Path, description = "Position address")
    ),
    responses(
        (status = 200, description = "Position closed", body = MessageResponse),
        (status = 404, description = "Position not found")
    )
)]
pub async fn close_position(
    State(state): State<AppState>,
    Path(address): Path<String>,
    Query(q): Query<CostSessionQuery>,
) -> ApiResult<Json<MessageResponse>> {
    let pubkey = Pubkey::from_str(&address)
        .map_err(|_| ApiError::bad_request("Invalid position address"))?;

    info!(position = %address, dry_run = state.dry_run, "Closing position");

    // Match `GET /positions/:address`: allow close when the PDA is on-chain even if the in-memory
    // monitor has not finished `add_position` yet (race after opening the detail page) or after
    // restart before registry seed.
    let positions = state.monitor.get_positions().await;
    let position_snapshot = if let Some(p) = positions.iter().find(|p| p.address == pubkey) {
        p.clone()
    } else {
        monitored_position_from_chain(state.provider.clone(), &pubkey).await?
    };

    if state.dry_run {
        info!("Dry-run mode: would close position");

        // Broadcast simulated update
        state
            .broadcast_position_update(PositionUpdate {
                update_type: "close_simulated".to_string(),
                position_address: address.clone(),
                timestamp: chrono::Utc::now(),
                data: serde_json::json!({
                    "liquidity": position_snapshot.on_chain.liquidity.to_string(),
                    "dry_run": true
                }),
            })
            .await;

        return Ok(Json(MessageResponse::new(format!(
            "[DRY-RUN] Would close position {} with liquidity {}",
            address, position_snapshot.on_chain.liquidity
        ))));
    }

    let mut svc = PositionService::new(state.clone());
    svc.set_dry_run(false);
    if let Some(exec) = resolve_executor_for_position_ops(&state).await {
        svc.set_executor(exec);
    }

    let sid = q
        .cost_session_id
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let op = svc.close_position(&address, sid).await?;
    if op.success {
        // Manual close is an explicit end-of-history decision by operator.
        // Detach this PDA from all strategies so it cannot be managed/reopened via stale links.
        if let Err(e) = remove_position_address_from_all_strategies(&state, &address).await {
            warn!(
                position = %address,
                error = %e,
                "close_position: strategy unlink failed after manual close (continuing)"
            );
        }
        // Remove immediately so UI doesn't keep showing stale monitored entry.
        state.monitor.remove_position(&pubkey).await;
        state
            .broadcast_position_update(PositionUpdate {
                update_type: "closed".to_string(),
                position_address: address.clone(),
                timestamp: chrono::Utc::now(),
                data: serde_json::json!({}),
            })
            .await;

        Ok(Json(MessageResponse::new(format!(
            "Position closed: {address}"
        ))))
    } else {
        Err(ApiError::ServiceUnavailable(
            op.error
                .unwrap_or_else(|| "Position closing failed".to_string()),
        ))
    }
}

/// Collect fees from a position.
#[utoipa::path(
    post,
    path = "/positions/{address}/collect",
    tag = "Positions",
    params(
        ("address" = String, Path, description = "Position address")
    ),
    responses(
        (status = 200, description = "Fees collected", body = MessageResponse),
        (status = 404, description = "Position not found")
    )
)]
pub async fn collect_fees(
    State(state): State<AppState>,
    Path(address): Path<String>,
    Query(q): Query<CostSessionQuery>,
) -> ApiResult<Json<MessageResponse>> {
    let pubkey = Pubkey::from_str(&address)
        .map_err(|_| ApiError::bad_request("Invalid position address"))?;

    info!(position = %address, dry_run = state.dry_run, "Collecting fees");

    // Verify position exists
    let positions = state.monitor.get_positions().await;
    let position = positions
        .iter()
        .find(|p| p.address == pubkey)
        .ok_or_else(|| ApiError::not_found("Position not found"))?;

    if state.dry_run {
        info!("Dry-run mode: would collect fees");

        // Broadcast simulated update
        state
            .broadcast_position_update(PositionUpdate {
                update_type: "fees_collected_simulated".to_string(),
                position_address: address.clone(),
                timestamp: chrono::Utc::now(),
                data: serde_json::json!({
                    "fees_a": position.pnl.fees_earned_a,
                    "fees_b": position.pnl.fees_earned_b,
                    "dry_run": true
                }),
            })
            .await;

        return Ok(Json(MessageResponse::new(format!(
            "[DRY-RUN] Would collect fees from position {}: {} token A, {} token B",
            address, position.pnl.fees_earned_a, position.pnl.fees_earned_b
        ))));
    }

    let mut svc = PositionService::new(state.clone());
    svc.set_dry_run(false);
    if let Some(exec) = resolve_executor_for_position_ops(&state).await {
        svc.set_executor(exec);
    }

    let sid = q
        .cost_session_id
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let op = svc.collect_fees(&address, sid).await?;
    if op.success {
        let msg = op
            .data
            .as_ref()
            .and_then(|d| d.get("message"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Fees collected from position: {address}"));
        Ok(Json(MessageResponse::new(msg.to_string())))
    } else {
        Err(ApiError::ServiceUnavailable(
            op.error
                .unwrap_or_else(|| "Fee collection failed".to_string()),
        ))
    }
}

/// Decrease liquidity from a position.
#[utoipa::path(
    post,
    path = "/positions/{address}/decrease",
    tag = "Positions",
    params(
        ("address" = String, Path, description = "Position address")
    ),
    request_body = DecreaseLiquidityRequest,
    responses(
        (status = 200, description = "Liquidity decreased", body = MessageResponse),
        (status = 404, description = "Position not found")
    )
)]
pub async fn decrease_liquidity(
    State(state): State<AppState>,
    Path(address): Path<String>,
    Json(request): Json<DecreaseLiquidityRequest>,
) -> ApiResult<Json<MessageResponse>> {
    let mut svc = PositionService::new(state.clone());
    svc.set_dry_run(state.dry_run);

    if !state.dry_run
        && let Some(exec) = resolve_executor_for_position_ops(&state).await
    {
        svc.set_executor(exec);
    }

    let liquidity_amount: u128 = request.liquidity_amount.trim().parse().map_err(|_| {
        ApiError::bad_request("liquidity_amount must be a non-negative decimal integer string")
    })?;

    let op = svc.decrease_liquidity(&address, liquidity_amount).await?;
    if op.success {
        if state.dry_run {
            let msg = op
                .data
                .as_ref()
                .and_then(|d| d.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("Dry-run: liquidity decrease simulated");
            return Ok(Json(MessageResponse::new(format!("[DRY-RUN] {msg}"))));
        }
        Ok(Json(MessageResponse::new(format!(
            "Liquidity decreased for position: {address}"
        ))))
    } else {
        Err(ApiError::ServiceUnavailable(
            op.error
                .unwrap_or_else(|| "Decrease liquidity failed".to_string()),
        ))
    }
}

/// Rebalance a position.
#[utoipa::path(
    post,
    path = "/positions/{address}/rebalance",
    tag = "Positions",
    params(
        ("address" = String, Path, description = "Position address")
    ),
    request_body = RebalanceRequest,
    responses(
        (status = 200, description = "Position rebalanced", body = MessageResponse),
        (status = 404, description = "Position not found")
    )
)]
pub async fn rebalance_position(
    State(state): State<AppState>,
    Path(address): Path<String>,
    Json(request): Json<RebalanceRequest>,
) -> ApiResult<Json<MessageResponse>> {
    let mut svc = PositionService::new(state.clone());
    svc.set_dry_run(state.dry_run);

    if !state.dry_run
        && let Some(exec) = resolve_executor_for_position_ops(&state).await
    {
        svc.set_executor(exec);
    }

    let op = svc.rebalance_position(&address, &request).await?;
    if op.success {
        if state.dry_run {
            let msg = op
                .data
                .as_ref()
                .and_then(|d| d.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("Dry-run: rebalance simulated");
            return Ok(Json(MessageResponse::new(format!("[DRY-RUN] {msg}"))));
        }
        Ok(Json(MessageResponse::new(
            "Rebalance requested".to_string(),
        )))
    } else {
        Err(ApiError::ServiceUnavailable(
            op.error.unwrap_or_else(|| "Rebalance failed".to_string()),
        ))
    }
}

/// Get position PnL details.
#[utoipa::path(
    get,
    path = "/positions/{address}/pnl",
    tag = "Positions",
    params(
        ("address" = String, Path, description = "Position address")
    ),
    responses(
        (status = 200, description = "Position PnL", body = PnLResponse),
        (status = 404, description = "Position not found")
    )
)]
pub async fn get_position_pnl(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<PnLResponse>> {
    let pubkey = Pubkey::from_str(&address)
        .map_err(|_| ApiError::bad_request("Invalid position address"))?;

    let positions = state.monitor.get_positions().await;
    let position = positions
        .iter()
        .find(|p| p.address == pubkey)
        .ok_or_else(|| ApiError::not_found("Position not found"))?;

    let response = PnLResponse {
        unrealized_pnl_usd: position.pnl.net_pnl_usd,
        unrealized_pnl_pct: position.pnl.net_pnl_pct,
        fees_earned_a: position.pnl.fees_earned_a,
        fees_earned_b: position.pnl.fees_earned_b,
        fees_earned_usd: position.pnl.fees_usd,
        il_pct: position.pnl.il_pct,
        net_pnl_usd: position.pnl.net_pnl_usd,
        net_pnl_pct: position.pnl.net_pnl_pct,
    };

    Ok(Json(response))
}

/// Link this position to a strategy (or change strategy), or unlink from all strategies.
#[utoipa::path(
    post,
    path = "/positions/{address}/strategy",
    tag = "Positions",
    params(
        ("address" = String, Path, description = "Position PDA (base58)")
    ),
    request_body = LinkPositionStrategyRequest,
    responses(
        (status = 200, description = "Link updated", body = MessageResponse),
        (status = 400, description = "Invalid address"),
        (status = 404, description = "Strategy not found when strategy_id set")
    )
)]
pub async fn link_position_strategy(
    State(state): State<AppState>,
    Path(address): Path<String>,
    Json(body): Json<LinkPositionStrategyRequest>,
) -> ApiResult<Json<MessageResponse>> {
    let pos = address.trim();
    Pubkey::from_str(pos).map_err(|_| ApiError::bad_request("Invalid position address"))?;

    let target = body
        .strategy_id
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if let Some(ref sid) = target {
        let strategies = state.strategies.read().await;
        if !strategies.contains_key(sid) {
            return Err(ApiError::not_found("Strategy not found"));
        }
    }

    remove_position_address_from_all_strategies(&state, pos).await?;

    if let Some(ref sid) = target {
        append_position_address_to_strategy(&state, sid, pos).await?;
        let note = ensure_strategy_running_after_position_link(&state, sid, pos).await?;
        let mut msg = format!("Position linked to strategy {sid}");
        if let Some(w) = note {
            msg.push_str(". ");
            msg.push_str(&w);
        }
        Ok(Json(MessageResponse::new(msg)))
    } else {
        Ok(Json(MessageResponse::new(
            "Position unlinked from all strategies".to_string(),
        )))
    }
}

/// Suggest which strategy this position should be linked to (best-effort).
#[utoipa::path(
    get,
    path = "/positions/{address}/suggest-strategy",
    tag = "Positions",
    params(
        ("address" = String, Path, description = "Position PDA (base58)")
    ),
    responses(
        (status = 200, description = "Suggestion (may be null)", body = SuggestStrategyLinkResponse),
        (status = 400, description = "Invalid address")
    )
)]
pub async fn suggest_position_strategy(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<crate::models::SuggestStrategyLinkResponse>> {
    let pos = address.trim();
    Pubkey::from_str(pos).map_err(|_| ApiError::bad_request("Invalid position address"))?;

    // If already linked, do not suggest.
    let strategies = state.strategies.read().await;
    let already_linked = strategies.values().any(|s| {
        let params = s.config.get("parameters");
        let position_addresses = params
            .and_then(|p| p.get("position_addresses"))
            .and_then(|v| v.as_array());
        position_addresses
            .is_some_and(|arr| arr.iter().any(|v| v.as_str().map(str::trim) == Some(pos)))
    });
    drop(strategies);
    if already_linked {
        return Ok(Json(crate::models::SuggestStrategyLinkResponse {
            strategy_id: None,
            reason: "Position already linked to a strategy.".to_string(),
        }));
    }

    let Some(parent) = infer_parent_position_from_lifecycle_best_effort(pos).await else {
        return Ok(Json(crate::models::SuggestStrategyLinkResponse {
            strategy_id: None,
            reason:
                "No parent PDA inferred from lifecycle ledger (missing open/close correlation)."
                    .to_string(),
        }));
    };

    // Find a strategy that contains the parent PDA.
    let strategies = state.strategies.read().await;
    for s in strategies.values() {
        let params = s.config.get("parameters");
        let position_addresses = params
            .and_then(|p| p.get("position_addresses"))
            .and_then(|v| v.as_array());
        let has_parent = position_addresses.is_some_and(|arr| {
            arr.iter()
                .any(|v| v.as_str().map(str::trim) == Some(parent.trim()))
        });
        if has_parent {
            return Ok(Json(crate::models::SuggestStrategyLinkResponse {
                strategy_id: Some(s.id.clone()),
                reason: format!(
                    "Inferred parent PDA {parent} is linked to strategy {}.",
                    s.id
                ),
            }));
        }
    }

    Ok(Json(crate::models::SuggestStrategyLinkResponse {
        strategy_id: None,
        reason: format!(
            "Inferred parent PDA {parent} but no strategy contains it in parameters.position_addresses."
        ),
    }))
}
