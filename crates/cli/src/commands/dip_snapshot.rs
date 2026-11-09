//! CLI: dip zones + optional Jupiter Trigger open Buy-the-Dip orders (read-only).

use anyhow::{Context, Result};
use clmm_lp_data::dip::{classify_zone, load_dip_zones_file};
use clmm_lp_data::providers::{JupiterProvider, JupiterTriggerClient};
use serde_json::json;
use std::path::PathBuf;

const SOL_MINT: &str = "So11111111111111111111111111111111111111112";

pub async fn run_dip_snapshot(
    zones_path: PathBuf,
    include_orders: bool,
    jwt: Option<String>,
) -> Result<()> {
    let zones = load_dip_zones_file(&zones_path)?;
    let api_key = std::env::var("JUPITER_API_KEY").ok().filter(|s| !s.is_empty());
    let price_provider = match &api_key {
        Some(k) => JupiterProvider::with_api_key(k.clone()),
        None => JupiterProvider::new(),
    };
    let sol_price = price_provider.get_price(SOL_MINT).await.ok();
    let zone = sol_price
        .map(|p| {
            use rust_decimal::prelude::ToPrimitive;
            classify_zone(&zones, "SOL", p.to_f64().unwrap_or(f64::NAN))
        })
        .unwrap_or(clmm_lp_data::dip::DipZoneLabel::None);

    let mut notes = vec![format!("zones_path={}", zones_path.display())];
    let mut buy_dips = Vec::new();
    let mut orders_source = "skipped".to_string();

    if include_orders {
        let jwt = jwt
            .or_else(|| std::env::var("CLMM_JUPITER_TRIGGER_JWT").ok())
            .filter(|s| !s.trim().is_empty());
        match (api_key, jwt) {
            (Some(key), Some(token)) => {
                let client = JupiterTriggerClient::new(Some(key));
                match client.list_open_buy_dips(&token, 50).await {
                    Ok(orders) => {
                        buy_dips = orders;
                        orders_source = "trigger_v2_orders_history".into();
                    }
                    Err(e) => {
                        orders_source = "error".into();
                        notes.push(format!("orders failed: {e}"));
                    }
                }
            }
            (None, _) => {
                orders_source = "no_api_key".into();
                notes.push("JUPITER_API_KEY missing".into());
            }
            (_, None) => {
                orders_source = "no_jwt".into();
                notes.push("pass --jwt or CLMM_JUPITER_TRIGGER_JWT".into());
            }
        }
    }

    let out = json!({
        "symbol": "SOL",
        "mint": SOL_MINT,
        "price_usd": sol_price.map(|p| p.to_string()),
        "zone": zone.as_str(),
        "zones": zones.symbols.get("SOL"),
        "buy_dip_orders": buy_dips.iter().map(|o| json!({
            "id": o.id,
            "order_state": o.order_state,
            "trigger_price_usd": o.trigger_price_usd,
            "remaining_input_amount": o.remaining_input_amount,
        })).collect::<Vec<_>>(),
        "orders_source": orders_source,
        "notes": notes,
        "mode": "propose-only-read",
    });
    println!("{}", serde_json::to_string_pretty(&out).context("serialize")?);
    Ok(())
}
