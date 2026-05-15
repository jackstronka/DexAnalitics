//! Dev helper: print `position_stream_valuation_snapshots` rows for one `position_pubkey`.
//!
//! ```text
//! DATABASE_URL=postgres://… cargo run -p clmm-lp-data --bin snapshot_probe -- <POSITION_PUBKEY>
//! ```

use anyhow::Context;
use rust_decimal::Decimal;
use sqlx::Row;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let pos = std::env::args()
        .nth(1)
        .context("usage: snapshot_probe <position_pubkey>")?;
    let pos = pos.trim().to_string();
    if pos.is_empty() {
        anyhow::bail!("empty position_pubkey");
    }

    let url = std::env::var("DATABASE_URL").context("set DATABASE_URL (e.g. from .env)")?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .context("connect DATABASE_URL")?;

    let rows = sqlx::query(
        r#"SELECT ts_utc, value_usd, amount_a_ui, amount_b_ui, price_a_usd, price_b_usd,
                  token_mint_a, token_mint_b, raw_json
           FROM position_stream_valuation_snapshots
           WHERE position_pubkey = $1
           ORDER BY ts_utc ASC"#,
    )
    .bind(&pos)
    .fetch_all(&pool)
    .await
    .context("query position_stream_valuation_snapshots")?;

    println!("position_pubkey={pos}");
    println!("row_count={}", rows.len());
    for (i, r) in rows.iter().enumerate() {
        let ts: chrono::DateTime<chrono::Utc> = r.try_get("ts_utc")?;
        let vu: Decimal = r.try_get("value_usd").unwrap_or(Decimal::ZERO);
        let aa: Option<Decimal> = r
            .try_get::<Option<Decimal>, _>("amount_a_ui")
            .ok()
            .flatten();
        let bb: Option<Decimal> = r
            .try_get::<Option<Decimal>, _>("amount_b_ui")
            .ok()
            .flatten();
        let pa: Option<Decimal> = r
            .try_get::<Option<Decimal>, _>("price_a_usd")
            .ok()
            .flatten();
        let pb: Option<Decimal> = r
            .try_get::<Option<Decimal>, _>("price_b_usd")
            .ok()
            .flatten();
        let ma: Option<String> = r
            .try_get::<Option<String>, _>("token_mint_a")
            .ok()
            .flatten();
        let mb: Option<String> = r
            .try_get::<Option<String>, _>("token_mint_b")
            .ok()
            .flatten();
        let raw: serde_json::Value = r
            .try_get::<serde_json::Value, _>("raw_json")
            .unwrap_or_else(|_| serde_json::json!({}));
        let kind = raw.get("kind").cloned().unwrap_or(serde_json::Value::Null);
        let nav = match (aa, bb, pa, pb) {
            (Some(a), Some(b), Some(pa), Some(pb)) if pa > Decimal::ZERO && pb > Decimal::ZERO => {
                Some(a * pa + b * pb)
            }
            _ => None,
        };
        println!("--- row[{i}] ts_utc={ts} ---");
        println!(
            "  columns: value_usd={vu} amount_a_ui={aa:?} amount_b_ui={bb:?} price_a_usd={pa:?} price_b_usd={pb:?}"
        );
        println!("  mints: token_mint_a={ma:?} token_mint_b={mb:?}");
        println!("  raw_json.kind={kind}");
        if let Some(n) = nav {
            println!("  recomputed_nav_amounts_x_prices={n}");
        } else {
            println!("  recomputed_nav_amounts_x_prices=<need all four column decimals positive>");
        }
    }

    Ok(())
}
