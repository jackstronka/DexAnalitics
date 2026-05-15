//! Dev helper: summarize `position_chain_history_*` — non-zero vs zero USD columns.
//!
//! ```text
//! cargo run -p clmm-lp-data --bin chain_history_sql_probe
//! cargo run -p clmm-lp-data --bin chain_history_sql_probe -- <ANCHOR_OR_POSITION_PUBKEY>
//! cargo run -p clmm-lp-data --bin chain_history_sql_probe -- --coverage-recent
//! ```
//!
//! `--coverage-recent`: last N distinct `position_pubkey` from `position_stream_valuation_snapshots`
//! (default window 14 days, cap 150) vs `position_chain_history_*` (`live`).

use anyhow::Context;
use sqlx::Row;

async fn print_coverage_recent(pool: &sqlx::PgPool, days: i32, cap: i64) -> anyhow::Result<()> {
    println!(
        "=== coverage: valuation snapshots (last {days}d, max {cap} PDAs by latest ts) vs chain-history (live) ==="
    );
    let rows = sqlx::query(
        r#"
        WITH recent AS (
            SELECT position_pubkey, MAX(ts_utc) AS last_ts
            FROM position_stream_valuation_snapshots
            WHERE ts_utc > NOW() - make_interval(days => $1)
            GROUP BY position_pubkey
        ),
        ranked AS (
            SELECT position_pubkey, last_ts,
                   ROW_NUMBER() OVER (ORDER BY last_ts DESC) AS rn
            FROM recent
        )
        SELECT ranked.position_pubkey,
               ranked.last_ts,
               (
                   EXISTS (
                       SELECT 1 FROM position_chain_history_nodes n
                       WHERE n.metrics_mode = 'live'
                         AND (
                             n.position_pubkey = ranked.position_pubkey
                             OR n.chain_anchor_pubkey = ranked.position_pubkey
                         )
                   )
                   OR EXISTS (
                       SELECT 1 FROM position_chain_history_meta m
                       WHERE m.metrics_mode = 'live'
                         AND (
                             m.chain_anchor_pubkey = ranked.position_pubkey
                             OR m.entry_position_address = ranked.position_pubkey
                             OR m.chain_json @> jsonb_build_array(ranked.position_pubkey)
                         )
                   )
               ) AS chain_history_ok
        FROM ranked
        WHERE ranked.rn <= $2
        ORDER BY ranked.last_ts DESC
        "#,
    )
    .bind(days)
    .bind(cap)
    .fetch_all(pool)
    .await
    .context("coverage query")?;

    if rows.is_empty() {
        println!("(no valuation snapshots in window — cannot infer \"new\" PDAs)");
        return Ok(());
    }

    let mut ok: usize = 0;
    let mut miss: usize = 0;
    let mut missing: Vec<String> = Vec::new();
    for r in &rows {
        let ok_flag: bool = r.try_get("chain_history_ok")?;
        if ok_flag {
            ok += 1;
        } else {
            miss += 1;
            let pk: String = r.try_get("position_pubkey")?;
            if missing.len() < 40 {
                missing.push(pk);
            }
        }
    }

    println!("  distinct_pdas_checked={} covered={} missing={}", rows.len(), ok, miss);
    if !missing.is_empty() {
        println!("  missing_chain_history_live (first {}):", missing.len());
        for pk in &missing {
            println!("    {pk}");
        }
        if miss > missing.len() {
            println!("    … and {} more", miss - missing.len());
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let arg1 = std::env::args().nth(1).map(|s| s.trim().to_string());

    let url = std::env::var("DATABASE_URL").context("set DATABASE_URL (e.g. from .env)")?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .context("connect DATABASE_URL")?;

    if arg1.as_deref() == Some("--coverage-recent") {
        print_coverage_recent(&pool, 14, 150).await?;
        return Ok(());
    }

    let filter = arg1;
    println!("=== position_chain_history_meta (row counts) ===");
    let meta_counts = sqlx::query(
        r#"SELECT metrics_mode, COUNT(*)::bigint AS n
           FROM position_chain_history_meta
           GROUP BY metrics_mode
           ORDER BY metrics_mode"#,
    )
    .fetch_all(&pool)
    .await
    .context("query position_chain_history_meta")?;
    if meta_counts.is_empty() {
        println!("(no rows in position_chain_history_meta)");
    }
    for r in &meta_counts {
        let mode: String = r.try_get("metrics_mode")?;
        let n: i64 = r.try_get("n")?;
        println!("  metrics_mode={mode} meta_rows={n}");
    }

    println!("\n=== position_chain_history_meta — recent (anchor, mode, entry, ts) ===");
    let recent_meta = sqlx::query(
        r#"SELECT chain_anchor_pubkey, metrics_mode, entry_position_address, materialized_ts_utc
           FROM position_chain_history_meta
           ORDER BY materialized_ts_utc DESC NULLS LAST
           LIMIT 20"#,
    )
    .fetch_all(&pool)
    .await
    .context("recent meta")?;
    if recent_meta.is_empty() {
        println!("(no meta rows)");
    }
    for r in &recent_meta {
        let anchor: String = r.try_get("chain_anchor_pubkey")?;
        let mode: String = r.try_get("metrics_mode")?;
        let entry: String = r.try_get("entry_position_address")?;
        let ts: chrono::DateTime<chrono::Utc> = r.try_get("materialized_ts_utc")?;
        println!("  anchor={anchor} mode={mode} entry={entry} ts={ts}");
    }

    println!("\n=== position_chain_history_nodes — USD columns (aggregated) ===");
    let q = if let Some(ref pk) = filter {
        if pk.is_empty() {
            anyhow::bail!("empty pubkey filter");
        }
        sqlx::query(
            r#"SELECT metrics_mode,
                      COUNT(*)::bigint AS rows_n,
                      SUM(CASE WHEN start_value_usd > 0 THEN 1 ELSE 0 END)::bigint AS start_gt0,
                      SUM(CASE WHEN start_value_usd = 0 OR start_value_usd IS NULL THEN 1 ELSE 0 END)::bigint AS start_zero_or_null,
                      SUM(CASE WHEN end_value_usd > 0 THEN 1 ELSE 0 END)::bigint AS end_gt0,
                      SUM(CASE WHEN end_value_usd = 0 OR end_value_usd IS NULL THEN 1 ELSE 0 END)::bigint AS end_zero_or_null,
                      SUM(CASE WHEN current_value_usd > 0 THEN 1 ELSE 0 END)::bigint AS current_gt0,
                      SUM(CASE WHEN current_value_usd = 0 OR current_value_usd IS NULL THEN 1 ELSE 0 END)::bigint AS current_zero_or_null
               FROM position_chain_history_nodes
               WHERE chain_anchor_pubkey = $1 OR position_pubkey = $1
               GROUP BY metrics_mode
               ORDER BY metrics_mode"#,
        )
        .bind(pk)
        .fetch_all(&pool)
        .await
        .context("aggregate nodes (filter)")?
    } else {
        sqlx::query(
            r#"SELECT metrics_mode,
                      COUNT(*)::bigint AS rows_n,
                      SUM(CASE WHEN start_value_usd > 0 THEN 1 ELSE 0 END)::bigint AS start_gt0,
                      SUM(CASE WHEN start_value_usd = 0 OR start_value_usd IS NULL THEN 1 ELSE 0 END)::bigint AS start_zero_or_null,
                      SUM(CASE WHEN end_value_usd > 0 THEN 1 ELSE 0 END)::bigint AS end_gt0,
                      SUM(CASE WHEN end_value_usd = 0 OR end_value_usd IS NULL THEN 1 ELSE 0 END)::bigint AS end_zero_or_null,
                      SUM(CASE WHEN current_value_usd > 0 THEN 1 ELSE 0 END)::bigint AS current_gt0,
                      SUM(CASE WHEN current_value_usd = 0 OR current_value_usd IS NULL THEN 1 ELSE 0 END)::bigint AS current_zero_or_null
               FROM position_chain_history_nodes
               GROUP BY metrics_mode
               ORDER BY metrics_mode"#,
        )
        .fetch_all(&pool)
        .await
        .context("aggregate nodes (all)")?
    };

    if q.is_empty() {
        println!("(no matching rows in position_chain_history_nodes)");
    }
    for r in &q {
        let mode: String = r.try_get("metrics_mode")?;
        let rows_n: i64 = r.try_get("rows_n")?;
        let start_gt0: i64 = r.try_get("start_gt0")?;
        let start_z: i64 = r.try_get("start_zero_or_null")?;
        let end_gt0: i64 = r.try_get("end_gt0")?;
        let end_z: i64 = r.try_get("end_zero_or_null")?;
        let cur_gt0: i64 = r.try_get("current_gt0")?;
        let cur_z: i64 = r.try_get("current_zero_or_null")?;
        println!(
            "  mode={mode} nodes={rows_n} | start>0={start_gt0} start0/∅={start_z} | end>0={end_gt0} end0/∅={end_z} | current>0={cur_gt0} current0/∅={cur_z}"
        );
    }

    println!("\n=== sample: 8 newest nodes (any anchor) ===");
    let sample = sqlx::query(
        r#"SELECT n.chain_anchor_pubkey, n.metrics_mode, n.chain_seq, n.position_pubkey,
                  n.start_value_usd, n.end_value_usd, n.current_value_usd,
                  n.closed_ts_utc IS NOT NULL AS is_closed,
                  m.materialized_ts_utc
           FROM position_chain_history_nodes n
           JOIN position_chain_history_meta m
             ON m.chain_anchor_pubkey = n.chain_anchor_pubkey AND m.metrics_mode = n.metrics_mode
           ORDER BY m.materialized_ts_utc DESC NULLS LAST, n.chain_anchor_pubkey, n.chain_seq
           LIMIT 8"#,
    )
    .fetch_all(&pool)
    .await
    .context("sample nodes")?;

    if sample.is_empty() {
        println!("(no rows)");
    }
    for r in &sample {
        let anchor: String = r.try_get("chain_anchor_pubkey")?;
        let mode: String = r.try_get("metrics_mode")?;
        let seq: i16 = r.try_get("chain_seq")?;
        let pos: String = r.try_get("position_pubkey")?;
        let s: Option<rust_decimal::Decimal> = r.try_get("start_value_usd").ok();
        let e: Option<rust_decimal::Decimal> = r.try_get("end_value_usd").ok();
        let c: Option<rust_decimal::Decimal> = r.try_get("current_value_usd").ok();
        let closed: bool = r.try_get("is_closed")?;
        let mt: Option<chrono::DateTime<chrono::Utc>> = r.try_get("materialized_ts_utc").ok();
        println!(
            "  anchor={anchor} mode={mode} seq={seq} pos={pos} closed={closed} start={s:?} end={e:?} current={c:?} materialized_ts={mt:?}"
        );
    }

    if let Some(pk) = filter {
        println!("\n=== detail rows for filter position_pubkey = {pk} ===");
        let detail = sqlx::query(
            r#"SELECT chain_anchor_pubkey, metrics_mode, chain_seq, position_pubkey,
                      start_value_usd, end_value_usd, current_value_usd,
                      closed_ts_utc IS NOT NULL AS is_closed
               FROM position_chain_history_nodes
               WHERE position_pubkey = $1 OR chain_anchor_pubkey = $1
               ORDER BY metrics_mode, chain_seq"#,
        )
        .bind(&pk)
        .fetch_all(&pool)
        .await
        .context("detail rows")?;
        if detail.is_empty() {
            println!("(no rows for this pubkey)");
        }
        for r in &detail {
            let anchor: String = r.try_get("chain_anchor_pubkey")?;
            let mode: String = r.try_get("metrics_mode")?;
            let seq: i16 = r.try_get("chain_seq")?;
            let pos: String = r.try_get("position_pubkey")?;
            let s: Option<rust_decimal::Decimal> = r.try_get("start_value_usd").ok();
            let e: Option<rust_decimal::Decimal> = r.try_get("end_value_usd").ok();
            let c: Option<rust_decimal::Decimal> = r.try_get("current_value_usd").ok();
            let closed: bool = r.try_get("is_closed")?;
            println!(
                "  anchor={anchor} mode={mode} seq={seq} pos={pos} closed={closed} start={s:?} end={e:?} current={c:?}"
            );
        }
    }

    Ok(())
}
