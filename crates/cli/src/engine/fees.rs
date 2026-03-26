use crate::backtest_engine::StepData;
use rust_decimal::Decimal;

/// How the LP share of pool fees is computed.
#[derive(Clone, Copy, Debug)]
pub enum FeeShareModel {
    /// Legacy model: constant share proxy per step (capital/TVL or override).
    LegacyLpShare,
    /// Concentrated liquidity model: constant position liquidity share vs pool active liquidity.
    LiquidityShare {
        position_liquidity: u128,
        pool_active_liquidity: u128,
    },
}

impl FeeShareModel {
    pub fn step_fee_share(&self, p: &StepData) -> Decimal {
        match *self {
            FeeShareModel::LegacyLpShare => p.lp_share,
            FeeShareModel::LiquidityShare {
                position_liquidity,
                pool_active_liquidity,
            } => {
                if pool_active_liquidity == 0 || position_liquidity == 0 {
                    Decimal::ZERO
                } else {
                    Decimal::from(position_liquidity) / Decimal::from(pool_active_liquidity)
                }
            }
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            FeeShareModel::LegacyLpShare => "legacy_lp_share",
            FeeShareModel::LiquidityShare { .. } => "liquidity_share",
        }
    }
}
