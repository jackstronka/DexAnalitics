//! Run `clmm_lp_simulation` for `POST /analytics/simulate` (browser dashboard).

use crate::error::ApiError;
use crate::models::{SimulationRequest, SimulationResponse, StrategyType};
use crate::state::AppState;
use clmm_lp_domain::value_objects::price::Price;
use clmm_lp_domain::value_objects::price_range::PriceRange;
use clmm_lp_protocols::prelude::{WhirlpoolReader, tick_to_price};
use clmm_lp_simulation::prelude::{
    ConstantLiquidity, ConstantVolume, GeometricBrownianMotion, ILLimitStrategy, PeriodicRebalance,
    SimulationConfig, StaticRange, ThresholdRebalance, simulate_with_strategy,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

/// Executes LP strategy simulation using on-chain pool fee/tick context and a **synthetic GBM** price path.
pub async fn run_dashboard_simulation(
    state: &AppState,
    req: SimulationRequest,
) -> Result<SimulationResponse, ApiError> {
    if req.tick_lower == req.tick_upper {
        return Err(ApiError::Validation(
            "tick_lower and tick_upper must differ".to_string(),
        ));
    }

    let days_i64 = req
        .end_date
        .signed_duration_since(req.start_date)
        .num_days();
    if days_i64 <= 0 {
        return Err(ApiError::Validation(
            "end_date must be after start_date".to_string(),
        ));
    }
    let days: usize = (days_i64 as usize).clamp(1, 730);

    let reader = WhirlpoolReader::new(state.provider.clone());
    let pool = reader
        .get_pool_state(req.pool_address.trim())
        .await
        .map_err(|e| ApiError::bad_request(format!("Whirlpool fetch: {e}")))?;

    let mut p_lower = tick_to_price(req.tick_lower);
    let mut p_upper = tick_to_price(req.tick_upper);
    if p_lower > p_upper {
        std::mem::swap(&mut p_lower, &mut p_upper);
    }
    let initial_range = PriceRange::new(Price::new(p_lower), Price::new(p_upper));

    let mid = (p_lower + p_upper) / Decimal::from(2u32);
    let range_width_pct = if mid.is_zero() {
        Decimal::new(2, 1)
    } else {
        (p_upper - p_lower) / mid
    };

    let entry_price = pool.price;
    if entry_price <= Decimal::ZERO {
        return Err(ApiError::bad_request("Pool price from RPC is non-positive"));
    }

    let fee_rate = pool.fee_rate();
    let global_liq = pool.liquidity.max(1_000_000);
    let position_liquidity_share = (global_liq / 10_000).max(1_000).min(global_liq);

    let vol = req.gbm_volatility.clamp(0.01, 5.0);
    let drift = req.gbm_drift.clamp(-2.0, 2.0);
    let dt = 1.0_f64 / 365.0_f64;
    let mut gbm = GeometricBrownianMotion::new(entry_price, drift, vol, dt);

    let daily_volume_notional =
        (req.initial_capital_usd * Decimal::new(5, 1)).max(Decimal::new(1_000, 0));
    let mut volume_model = ConstantVolume::new(daily_volume_notional);
    let liquidity_model = ConstantLiquidity::new(global_liq);

    let config = SimulationConfig::new(req.initial_capital_usd, initial_range.clone())
        .with_fee_rate(fee_rate)
        .with_pool_liquidity(position_liquidity_share)
        .with_rebalance_cost(Decimal::new(25, 1))
        .with_steps(days)
        .with_step_duration(86_400);

    let threshold_pct = req.threshold_pct.unwrap_or_else(|| Decimal::new(5, 2));
    let il_lim = req.il_limit_pct.unwrap_or_else(|| Decimal::new(8, 2));
    let periodic_interval = req.periodic_interval_steps.unwrap_or(7).max(1);

    let result = match req.strategy_type {
        StrategyType::StaticRange => {
            let s = StaticRange::new();
            simulate_with_strategy(&config, &mut gbm, &mut volume_model, &liquidity_model, &s)
        }
        StrategyType::Periodic => {
            let s = PeriodicRebalance::new(periodic_interval, range_width_pct);
            simulate_with_strategy(&config, &mut gbm, &mut volume_model, &liquidity_model, &s)
        }
        StrategyType::Threshold => {
            let s = ThresholdRebalance::new(threshold_pct, range_width_pct);
            simulate_with_strategy(&config, &mut gbm, &mut volume_model, &liquidity_model, &s)
        }
        StrategyType::Bollinger => {
            // Synthetic simulator has no candle history/bands; use periodic surrogate.
            let s = PeriodicRebalance::new(periodic_interval, range_width_pct);
            simulate_with_strategy(&config, &mut gbm, &mut volume_model, &liquidity_model, &s)
        }
        StrategyType::OorRecenter => {
            let s = ThresholdRebalance::new(Decimal::ONE, range_width_pct)
                .rebalance_on_out_of_range(true);
            simulate_with_strategy(&config, &mut gbm, &mut volume_model, &liquidity_model, &s)
        }
        StrategyType::IlLimit => {
            let s = ILLimitStrategy::new(il_lim, range_width_pct);
            simulate_with_strategy(&config, &mut gbm, &mut volume_model, &liquidity_model, &s)
        }
        StrategyType::RetouchShift => {
            let s = ThresholdRebalance::new(Decimal::new(25, 2), range_width_pct)
                .rebalance_on_out_of_range(true);
            simulate_with_strategy(&config, &mut gbm, &mut volume_model, &liquidity_model, &s)
        }
        StrategyType::LastCandle => {
            let s = ThresholdRebalance::new(Decimal::ONE, range_width_pct)
                .rebalance_on_out_of_range(true);
            simulate_with_strategy(&config, &mut gbm, &mut volume_model, &liquidity_model, &s)
        }
        StrategyType::LastCandlePeriodic => {
            let s = PeriodicRebalance::new(periodic_interval, range_width_pct);
            simulate_with_strategy(&config, &mut gbm, &mut volume_model, &liquidity_model, &s)
        }
    };

    let summary = &result.summary;
    let initial = req.initial_capital_usd;
    let fee_pct = if initial.is_zero() {
        Decimal::ZERO
    } else {
        summary.total_fees / initial * Decimal::from(100)
    };
    let sharpe = sharpe_from_pnl_deltas(&result.pnl_history);
    let tir = summary.time_in_range_pct();

    let methodology_note = format!(
        "Synthetic daily GBM path (vol={:.2}, drift={:.2}) over {} days from {:?} to {:?}; \
         pool fee from on-chain Whirlpool; strategy={:?}. \
         Not a replay of historical candles — for snapshot-accurate backtests use CLI `backtest` / `backtest-optimize`.",
        vol, drift, days, req.start_date, req.end_date, req.strategy_type
    );

    Ok(SimulationResponse {
        id: uuid::Uuid::new_v4().to_string(),
        pool_address: req.pool_address,
        tick_lower: req.tick_lower,
        tick_upper: req.tick_upper,
        initial_capital_usd: initial,
        final_value_usd: summary.final_value,
        total_return_pct: summary.net_pnl_pct * Decimal::from(100),
        fee_earnings_pct: fee_pct,
        il_pct: summary.final_il_pct * Decimal::from(100),
        sharpe_ratio: sharpe,
        max_drawdown_pct: summary.max_drawdown_pct * Decimal::from(100),
        time_in_range_pct: tir * Decimal::from(100),
        vs_hodl_usd: summary.vs_hodl,
        rebalance_count: summary.rebalance_count,
        methodology_note,
    })
}

fn sharpe_from_pnl_deltas(pnl: &[Decimal]) -> Decimal {
    if pnl.len() < 3 {
        return Decimal::ZERO;
    }
    let mut deltas = Vec::new();
    for w in pnl.windows(2) {
        deltas.push(w[1] - w[0]);
    }
    let n = deltas.len() as f64;
    if n < 2.0 {
        return Decimal::ZERO;
    }
    let sum: f64 = deltas.iter().filter_map(|d| d.to_f64()).sum();
    let mean = sum / n;
    let var: f64 = deltas
        .iter()
        .filter_map(|d| d.to_f64())
        .map(|x| (x - mean).powi(2))
        .sum::<f64>()
        / n;
    let std = var.sqrt();
    if std < 1e-12 {
        return Decimal::ZERO;
    }
    // Daily steps → annualize Sharpe ~ mean/std * sqrt(365)
    let ratio = mean / std * 365.0_f64.sqrt();
    Decimal::from_f64_retain(ratio).unwrap_or(Decimal::ZERO)
}
