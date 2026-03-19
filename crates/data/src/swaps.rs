use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Single swap event as returned by Dune `dex_solana.trades` queries.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SwapEvent {
    /// Local timestamp string from Dune, e.g. "2026-03-10 16:06".
    pub block_time: String,

    pub project: String,
    pub token_sold_symbol: String,
    pub token_bought_symbol: String,

    /// USD notional of the swap (trade size), not the fee.
    pub amount_usd: Decimal,

    /// Fee tier for the pool (e.g. 0.003). May be zero when not populated.
    #[serde(default)]
    pub fee_tier: Decimal,

    /// Fee paid on the swap in USD. May be zero when not populated.
    #[serde(default)]
    pub fee_usd: Decimal,

    pub token_sold_mint_address: String,
    pub token_bought_mint_address: String,
    pub token_sold_vault: String,
    pub token_bought_vault: String,
}

impl SwapEvent {
    /// Parse `block_time` into UTC `DateTime`.
    ///
    /// Dune returns "YYYY-MM-DD HH:MM", we convert it to RFC3339 "YYYY-MM-DDTHH:MM:00Z".
    pub fn block_time_utc(&self) -> Option<DateTime<Utc>> {
        let ts = self.block_time.trim();
        if ts.len() < 16 {
            return None;
        }
        let ts = ts.replace(' ', "T");
        let rfc3339 = format!("{ts}:00Z");
        DateTime::parse_from_rfc3339(&rfc3339)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    }
}

