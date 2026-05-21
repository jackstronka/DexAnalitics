//! Postgres integration: lifecycle row → SESSION GL → read / PSLR / caps / idempotency.
//!
//! Skips when `DATABASE_URL` is unset. Run:
//! `DATABASE_URL=postgres://clmm_user:clmm_password@localhost:5432/clmm_lp cargo test -p clmm-lp-data --test session_gl_integration`

use clmm_lp_data::repositories::Database;
use clmm_lp_data::wallet_session::{
    apply_session_postings_from_lifecycle_row, compute_session_balances_from_pslr,
    gl_pslr_match, lifecycle_posting_event_id, parse_raw_i128, read_session_balances,
    resolve_session_mint_caps, session_lifecycle_posting_already_applied,
    SessionCapsSource, SessionLifecyclePostingOutcome, USDC_MINT, WSOL_MINT,
};
use serde_json::json;
use uuid::Uuid;

fn database_url() -> Option<String> {
    std::env::var("DATABASE_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

async fn test_db() -> Option<Database> {
    let url = database_url()?;
    let db = Database::connect(&url).await.ok()?;
    db.migrate().await.ok()?;
    Some(db)
}

fn close_lifecycle_json(session_id: &str, signature: &str) -> serde_json::Value {
    json!({
        "event": "bot_close_position",
        "signature": signature,
        "rebalance_session_id": session_id,
        "fee_payer_pubkey": "Owner1111111111111111111111111111111111111111",
        "lp_collected_token_a_raw": 50_000,
        "lp_collected_token_b_raw": 0,
        "details": {
            "token_mint_a": WSOL_MINT,
            "token_mint_b": USDC_MINT,
            "close_amount_a_raw": 1_000_000_000u64,
            "close_amount_b_raw": 2_000_000u64
        }
    })
}

async fn insert_pslr_row(
    db: &Database,
    session_id: &str,
    signature: &str,
    raw: &serde_json::Value,
    lp_a: i64,
    lp_b: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO position_stream_ledger_rows (
            signature, ts_utc, source, event, rebalance_session_id, raw_json,
            lp_collected_token_a_raw, lp_collected_token_b_raw
        )
        VALUES ($1, NOW(), 'integration_test', 'bot_close_position', $2, $3, $4, $5)
        ON CONFLICT (signature) DO UPDATE SET
            rebalance_session_id = EXCLUDED.rebalance_session_id,
            raw_json = EXCLUDED.raw_json,
            lp_collected_token_a_raw = EXCLUDED.lp_collected_token_a_raw,
            lp_collected_token_b_raw = EXCLUDED.lp_collected_token_b_raw
        "#,
    )
    .bind(signature)
    .bind(session_id)
    .bind(raw)
    .bind(lp_a)
    .bind(lp_b)
    .execute(db.pool())
    .await?;
    Ok(())
}

fn balance_map(rows: &[clmm_lp_data::wallet_session::SessionBalanceMint]) -> std::collections::BTreeMap<String, i128> {
    rows.iter()
        .filter_map(|b| {
            parse_raw_i128(&b.amount_raw).map(|v| (b.mint.clone(), v))
        })
        .collect()
}

#[tokio::test]
async fn session_gl_lifecycle_posting_matches_pslr_and_caps() {
    let Some(db) = test_db().await else {
        eprintln!("skip session_gl_integration: DATABASE_URL unset or connect/migrate failed");
        return;
    };

    let session_id = format!("itest-{}", Uuid::new_v4());
    let signature = format!("sig-itest-{}", Uuid::new_v4());
    let owner = "Owner1111111111111111111111111111111111111111";
    let v = close_lifecycle_json(&session_id, &signature);

    insert_pslr_row(&db, &session_id, &signature, &v, 50_000, 0)
        .await
        .expect("insert pslr");

    let outcome =
        apply_session_postings_from_lifecycle_row(&db, &v, Some(50_000), Some(0))
            .await
            .expect("post lifecycle");
    assert_eq!(outcome, SessionLifecyclePostingOutcome::Applied);

    let event_id = lifecycle_posting_event_id(&signature);
    assert!(
        session_lifecycle_posting_already_applied(&db, &event_id)
            .await
            .expect("idempotency check")
    );

    let again =
        apply_session_postings_from_lifecycle_row(&db, &v, Some(50_000), Some(0))
            .await
            .expect("post again");
    assert_eq!(again, SessionLifecyclePostingOutcome::SkippedAlready);

    let gl = read_session_balances(&db, &session_id, Some(owner))
        .await
        .expect("read gl");
    let pslr = compute_session_balances_from_pslr(&db, &session_id)
        .await
        .expect("read pslr");

    assert!(gl_pslr_match(&gl, &pslr), "gl={gl:?} pslr={pslr:?}");

    let gl_map = balance_map(&gl);
    assert_eq!(gl_map.get(WSOL_MINT), Some(&1_000_050_000)); // principal + lp on A
    assert_eq!(gl_map.get(USDC_MINT), Some(&2_000_000));

    let caps = resolve_session_mint_caps(Some(&db), &session_id, Some(owner)).await;
    assert!(matches!(
        caps.source,
        SessionCapsSource::Gl | SessionCapsSource::ReconciledMin
    ));
    assert_eq!(caps.caps_by_mint.get(WSOL_MINT), Some(&1_000_050_000));
    assert_eq!(caps.caps_by_mint.get(USDC_MINT), Some(&2_000_000));
}

#[tokio::test]
async fn session_gl_collect_row_accumulates() {
    let Some(db) = test_db().await else {
        eprintln!("skip session_gl_integration: DATABASE_URL unset or connect/migrate failed");
        return;
    };

    let session_id = format!("itest-collect-{}", Uuid::new_v4());
    let sig1 = format!("sig-collect-1-{}", Uuid::new_v4());
    let sig2 = format!("sig-collect-2-{}", Uuid::new_v4());
    let owner = "Owner1111111111111111111111111111111111111111";

    let v1 = json!({
        "event": "bot_collect_fees",
        "signature": sig1,
        "rebalance_session_id": session_id,
        "fee_payer_pubkey": owner,
        "details": {
            "token_mint_a": WSOL_MINT,
            "token_mint_b": USDC_MINT
        }
    });
    let v2 = json!({
        "event": "bot_collect_fees",
        "signature": sig2,
        "rebalance_session_id": session_id,
        "fee_payer_pubkey": owner,
        "details": {
            "token_mint_a": WSOL_MINT,
            "token_mint_b": USDC_MINT
        }
    });

    for (sig, v, lp_a, lp_b) in [
        (&sig1, &v1, 10_i64, 20_i64),
        (&sig2, &v2, 5_i64, 7_i64),
    ] {
        insert_pslr_row(&db, &session_id, sig, v, lp_a, lp_b)
            .await
            .expect("insert pslr");
        let o = apply_session_postings_from_lifecycle_row(&db, v, Some(lp_a), Some(lp_b))
            .await
            .expect("post");
        assert_eq!(o, SessionLifecyclePostingOutcome::Applied);
    }

    let gl = read_session_balances(&db, &session_id, Some(owner))
        .await
        .expect("read gl");
    let pslr = compute_session_balances_from_pslr(&db, &session_id)
        .await
        .expect("read pslr");
    assert!(gl_pslr_match(&gl, &pslr));

    let gl_map = balance_map(&gl);
    assert_eq!(gl_map.get(WSOL_MINT), Some(&15));
    assert_eq!(gl_map.get(USDC_MINT), Some(&27));
}
