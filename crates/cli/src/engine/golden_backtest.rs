//! B4 golden: frozen Orca SOL/USDC snapshot path → `run_single` per strategy → ranking.

#[cfg(test)]
mod tests {
    use crate::backtest_engine::{StepDataPoint, StratConfig, run_single};
    use clmm_lp_domain::prelude::Price;
    use rust_decimal::Decimal;
    use serde::Deserialize;
    use std::collections::BTreeMap;
    use std::str::FromStr;

    const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/backtest_mini");

    #[derive(Debug, Deserialize)]
    struct Manifest {
        step_count: usize,
        capital_usd: String,
        width_pct: String,
        tx_cost_usd: String,
        fee_rate: String,
        token_a_decimals: u32,
        token_b_decimals: u32,
        pool_address: String,
        window_start_utc: String,
        window_end_utc: String,
    }

    #[derive(Debug, Deserialize)]
    struct StepDto {
        start_timestamp: u64,
        price_ab: String,
        quote_usd: String,
        step_volume_usd: String,
        lp_share: String,
        liquidity_active_raw: Option<String>,
        tick_current: Option<i32>,
    }

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s.trim()).unwrap_or_else(|e| panic!("decimal {s:?}: {e}"))
    }

    fn usd6(d: Decimal) -> String {
        d.round_dp(6).normalize().to_string()
    }

    fn load_manifest() -> Manifest {
        let raw =
            std::fs::read_to_string(format!("{FIXTURE_DIR}/manifest.json")).expect("manifest");
        serde_json::from_str(&raw).expect("manifest json")
    }

    fn load_steps() -> Vec<StepDataPoint> {
        let raw = std::fs::read_to_string(format!("{FIXTURE_DIR}/steps.jsonl")).expect("steps");
        raw.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|line| {
                let dto: StepDto = serde_json::from_str(line).expect("step dto");
                let price_ab = dec(&dto.price_ab);
                let quote_usd = dec(&dto.quote_usd);
                StepDataPoint {
                    price_usd: Price::new(price_ab * quote_usd),
                    price_ab: Price::new(price_ab),
                    step_volume_usd: dec(&dto.step_volume_usd),
                    quote_usd,
                    lp_share: dec(&dto.lp_share),
                    liquidity_active_raw: dto
                        .liquidity_active_raw
                        .as_deref()
                        .map(|s| s.parse::<u128>().expect("liquidity_active_raw")),
                    tick_current: dto.tick_current,
                    start_timestamp: dto.start_timestamp,
                }
            })
            .collect()
    }

    fn load_fees() -> BTreeMap<usize, Decimal> {
        let raw =
            std::fs::read_to_string(format!("{FIXTURE_DIR}/fees_by_step.json")).expect("fees");
        let map: BTreeMap<String, String> = serde_json::from_str(&raw).expect("fees json");
        map.into_iter()
            .map(|(k, v)| (k.parse::<usize>().expect("fee step index"), dec(&v)))
            .collect()
    }

    fn mini_strategies() -> Vec<StratConfig> {
        vec![
            StratConfig::Static,
            StratConfig::OorRecenter,
            StratConfig::Threshold {
                threshold_pct: 0.05,
                min_rebalance_interval_hours: 0,
                rebalance_on_range_exit_immediately: true,
            },
            StratConfig::Periodic(24),
            StratConfig::IlLimit {
                max_il_pct: 0.05,
                close_il_pct: None,
                grace_steps: 0,
            },
            StratConfig::RetouchShift {
                retouch_offset_pct: 0.0,
            },
            StratConfig::Bollinger {
                window: 20,
                k: 2.0,
                rebalance_steps: 24,
            },
            StratConfig::LastCandle {
                candle_steps: 12,
                rebalance_steps: 24,
            },
        ]
    }

    /// B4 golden: DTO → `run_single` × strategies → vs_hodl ranking.
    /// Snapshot path only (no RPC / DexScreener / repo `data/`). A diff is an `economic_regression`.
    #[test]
    fn golden_backtest_mini_run_single_ranking() {
        let mut env = crate::test_env::EnvGuard::blocking_lock();
        env.remove("CLMM_DEBUG_STEP_LIQ_SHARE");
        env.remove("CLMM_IN_RANGE_TICK");

        let manifest = load_manifest();
        let steps = load_steps();
        let fees = load_fees();
        assert_eq!(steps.len(), manifest.step_count);
        assert!(!fees.is_empty(), "fixture must include snapshot pool fees");

        let capital = dec(&manifest.capital_usd);
        let width_pct: f64 = manifest.width_pct.parse().expect("width");
        let tx_cost = dec(&manifest.tx_cost_usd);
        let fee_rate = dec(&manifest.fee_rate);
        let first = steps.first().expect("non-empty steps");
        let entry = first.price_usd;
        let center = first
            .price_usd
            .value
            .to_string()
            .parse::<f64>()
            .unwrap_or(0.0);

        let mut ranked: Vec<(Decimal, Decimal, u32, String, serde_json::Value)> = mini_strategies()
            .into_iter()
            .map(|strat| {
                let (_lo, _hi, label, summary) = run_single(
                    &steps,
                    entry,
                    center,
                    width_pct,
                    strat,
                    capital,
                    tx_cost,
                    fee_rate,
                    None,
                    manifest.token_a_decimals,
                    manifest.token_b_decimals,
                    None,
                    Some(&fees),
                );
                (
                    summary.vs_hodl,
                    summary.total_fees,
                    summary.rebalance_count,
                    label.clone(),
                    serde_json::json!({
                        "strategy": label,
                        "width_pct": &manifest.width_pct,
                        "total_fees_usd": usd6(summary.total_fees),
                        "final_il_pct": usd6(summary.final_il_pct),
                        "vs_hodl_usd": usd6(summary.vs_hodl),
                        "rebalance_count": summary.rebalance_count,
                    }),
                )
            })
            .collect();
        ranked.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| a.2.cmp(&b.2))
                .then_with(|| a.3.cmp(&b.3))
        });

        let ranking: Vec<serde_json::Value> = ranked
            .into_iter()
            .enumerate()
            .map(|(i, (_, _, _, _, mut row))| {
                row["rank"] = serde_json::json!(i + 1);
                row
            })
            .collect();

        let snapshot = serde_json::json!({
            "pool": manifest.pool_address,
            "window_start_utc": manifest.window_start_utc,
            "window_end_utc": manifest.window_end_utc,
            "steps": manifest.step_count,
            "objective": "vs_hodl",
            "ranking": ranking,
        });
        insta::assert_json_snapshot!(snapshot);
        drop(env);
    }
}
