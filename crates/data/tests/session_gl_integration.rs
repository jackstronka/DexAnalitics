//! Postgres integration: lifecycle → SESSION / CHAIN / WALLET GL → read / PSLR / idempotency.
//!
//! Skips when `DATABASE_URL` is unset, unless `CLMM_REQUIRE_DB_TESTS=1` (CI) — then a missing or
//! unreachable database fails the test. Run against a disposable database (tests insert rows):
//! `DATABASE_URL=postgres://clmm_user:clmm_password@localhost:5432/clmm_lp_test cargo test -p clmm-lp-data --test session_gl_integration`

use clmm_lp_data::repositories::Database;
use clmm_lp_data::wallet_session::{
    SessionCapsSource, SessionLifecyclePostingOutcome, USDC_MINT, WSOL_MINT,
    apply_chain_postings_from_lifecycle_row, apply_session_postings_from_lifecycle_row,
    apply_wallet_mint_postings, apply_wallet_opening_import, chain_lifecycle_posting_event_id,
    compute_chain_balances_from_pslr, compute_session_balances_from_pslr, gl_pslr_match,
    lifecycle_posting_event_id, parse_raw_i128, read_chain_balances, read_session_balances,
    read_wallet_balances, resolve_session_mint_caps, session_lifecycle_posting_already_applied,
    wallet_opening_import_already_applied,
};
use serde_json::json;
use uuid::Uuid;

fn database_url() -> Option<String> {
    std::env::var("DATABASE_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// `migrate()` is not safe to run concurrently on a fresh database (BUG-20261001-01); tests in this
/// file run in parallel, so migrate once per test process.
static MIGRATED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

fn db_tests_required() -> bool {
    std::env::var("CLMM_REQUIRE_DB_TESTS").is_ok_and(|v| v.trim() == "1")
}

async fn test_db() -> Option<Database> {
    let required = db_tests_required();
    let Some(url) = database_url() else {
        assert!(
            !required,
            "CLMM_REQUIRE_DB_TESTS=1 but DATABASE_URL is unset"
        );
        return None;
    };
    let db = match Database::connect(&url).await {
        Ok(db) => db,
        Err(e) => {
            assert!(!required, "CLMM_REQUIRE_DB_TESTS=1 but connect failed: {e}");
            return None;
        }
    };
    if let Err(e) = MIGRATED.get_or_try_init(|| db.migrate()).await {
        assert!(!required, "CLMM_REQUIRE_DB_TESTS=1 but migrate failed: {e}");
        return None;
    }
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

async fn insert_chain_pslr_row(
    db: &Database,
    chain_session_id: &str,
    session_id: &str,
    signature: &str,
    raw: &serde_json::Value,
    lp_a: i64,
    lp_b: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO position_stream_ledger_rows (
            signature, ts_utc, source, event, rebalance_session_id, chain_session_id, raw_json,
            lp_collected_token_a_raw, lp_collected_token_b_raw
        )
        VALUES ($1, NOW(), 'integration_test', 'bot_close_position', $2, $3, $4, $5, $6)
        ON CONFLICT (signature) DO UPDATE SET
            rebalance_session_id = EXCLUDED.rebalance_session_id,
            chain_session_id = EXCLUDED.chain_session_id,
            raw_json = EXCLUDED.raw_json,
            lp_collected_token_a_raw = EXCLUDED.lp_collected_token_a_raw,
            lp_collected_token_b_raw = EXCLUDED.lp_collected_token_b_raw
        "#,
    )
    .bind(signature)
    .bind(session_id)
    .bind(chain_session_id)
    .bind(raw)
    .bind(lp_a)
    .bind(lp_b)
    .execute(db.pool())
    .await?;
    Ok(())
}

fn close_lifecycle_json_chain(
    chain_session_id: &str,
    session_id: &str,
    signature: &str,
) -> serde_json::Value {
    json!({
        "event": "bot_close_position",
        "signature": signature,
        "rebalance_session_id": session_id,
        "chain_session_id": chain_session_id,
        "fee_payer_pubkey": "Owner1111111111111111111111111111111111111111",
        "lp_collected_token_a_raw": 25_000,
        "lp_collected_token_b_raw": 0,
        "details": {
            "token_mint_a": WSOL_MINT,
            "token_mint_b": USDC_MINT,
            "close_amount_a_raw": 500_000_000u64,
            "close_amount_b_raw": 1_500_000u64
        }
    })
}

fn balance_map(
    rows: &[clmm_lp_data::wallet_session::SessionBalanceMint],
) -> std::collections::BTreeMap<String, i128> {
    rows.iter()
        .filter_map(|b| parse_raw_i128(&b.amount_raw).map(|v| (b.mint.clone(), v)))
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

    let outcome = apply_session_postings_from_lifecycle_row(&db, &v, Some(50_000), Some(0))
        .await
        .expect("post lifecycle");
    assert_eq!(outcome, SessionLifecyclePostingOutcome::Applied);

    let event_id = lifecycle_posting_event_id(&signature);
    assert!(
        session_lifecycle_posting_already_applied(&db, &event_id)
            .await
            .expect("idempotency check")
    );

    let again = apply_session_postings_from_lifecycle_row(&db, &v, Some(50_000), Some(0))
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

    for (sig, v, lp_a, lp_b) in [(&sig1, &v1, 10_i64, 20_i64), (&sig2, &v2, 5_i64, 7_i64)] {
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

#[tokio::test]
async fn chain_gl_lifecycle_posting_matches_pslr() {
    let Some(db) = test_db().await else {
        eprintln!("skip chain_gl_integration: DATABASE_URL unset or connect/migrate failed");
        return;
    };

    let chain_session_id = format!("itest-chain-{}", Uuid::new_v4());
    let session_id = format!("itest-sess-{}", Uuid::new_v4());
    let signature = format!("sig-chain-{}", Uuid::new_v4());
    let owner = "Owner1111111111111111111111111111111111111111";
    let v = close_lifecycle_json_chain(&chain_session_id, &session_id, &signature);

    insert_chain_pslr_row(
        &db,
        &chain_session_id,
        &session_id,
        &signature,
        &v,
        25_000,
        0,
    )
    .await
    .expect("insert chain pslr");

    let outcome = apply_chain_postings_from_lifecycle_row(&db, &v, Some(25_000), Some(0))
        .await
        .expect("post chain lifecycle");
    assert_eq!(outcome, SessionLifecyclePostingOutcome::Applied);

    let again = apply_chain_postings_from_lifecycle_row(&db, &v, Some(25_000), Some(0))
        .await
        .expect("post chain again");
    assert_eq!(again, SessionLifecyclePostingOutcome::SkippedAlready);

    let gl = read_chain_balances(&db, &chain_session_id, Some(owner))
        .await
        .expect("read chain gl");
    let pslr = compute_chain_balances_from_pslr(&db, &chain_session_id)
        .await
        .expect("read chain pslr");

    assert!(gl_pslr_match(&gl, &pslr), "gl={gl:?} pslr={pslr:?}");

    let gl_map = balance_map(&gl);
    assert_eq!(gl_map.get(WSOL_MINT), Some(&500_025_000));
    assert_eq!(gl_map.get(USDC_MINT), Some(&1_500_000));
}

#[tokio::test]
async fn wallet_gl_opening_import_and_journal_postings() {
    let Some(db) = test_db().await else {
        eprintln!("skip wallet_gl_integration: DATABASE_URL unset or connect/migrate failed");
        return;
    };

    let owner = format!("OwnerItest{}", Uuid::new_v4());
    let opening = vec![
        (WSOL_MINT.to_string(), 3_000_000i128),
        (USDC_MINT.to_string(), 900_000i128),
    ];

    let outcome = apply_wallet_opening_import(&db, &owner, &opening)
        .await
        .expect("opening import");
    assert_eq!(outcome, SessionLifecyclePostingOutcome::Applied);
    assert!(
        wallet_opening_import_already_applied(&db, &owner)
            .await
            .expect("opening idempotency flag")
    );

    let again = apply_wallet_opening_import(&db, &owner, &opening)
        .await
        .expect("opening import again");
    assert_eq!(again, SessionLifecyclePostingOutcome::SkippedAlready);

    apply_wallet_mint_postings(
        &db,
        &owner,
        "journal:transfer:test",
        "transfer_sol",
        &[(WSOL_MINT.to_string(), -500_000i128)],
    )
    .await
    .expect("journal posting");

    let gl = read_wallet_balances(&db, &owner)
        .await
        .expect("read wallet gl");
    let gl_map = balance_map(&gl);
    assert_eq!(gl_map.get(WSOL_MINT), Some(&2_500_000));
    assert_eq!(gl_map.get(USDC_MINT), Some(&900_000));
}

async fn gl_posting_count(db: &Database, event_id: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"SELECT COUNT(*)::bigint FROM wallet_gl_posting WHERE event_id = $1"#,
    )
    .bind(event_id)
    .fetch_one(db.pool())
    .await
    .expect("count postings")
}

/// C4: replaying the same lifecycle row must not change SESSION/CHAIN balances or posting count.
#[tokio::test]
async fn session_and_chain_gl_lifecycle_row_replay_does_not_change_balances() {
    let Some(db) = test_db().await else {
        eprintln!("skip session_gl_integration: DATABASE_URL unset or connect/migrate failed");
        return;
    };

    let owner = "Owner1111111111111111111111111111111111111111";

    let session_id = format!("itest-c4-sess-{}", Uuid::new_v4());
    let close_sig = format!("sig-c4-close-{}", Uuid::new_v4());
    let close = close_lifecycle_json(&session_id, &close_sig);
    let first = apply_session_postings_from_lifecycle_row(&db, &close, Some(50_000), Some(0))
        .await
        .expect("session close");
    assert_eq!(first, SessionLifecyclePostingOutcome::Applied);
    let session_after_first = balance_map(
        &read_session_balances(&db, &session_id, Some(owner))
            .await
            .expect("read session after first"),
    );
    assert!(!session_after_first.is_empty());
    let close_event_id = lifecycle_posting_event_id(&close_sig);
    let session_posts_first = gl_posting_count(&db, &close_event_id).await;
    assert!(session_posts_first > 0);

    let again = apply_session_postings_from_lifecycle_row(&db, &close, Some(50_000), Some(0))
        .await
        .expect("session close replay");
    assert_eq!(again, SessionLifecyclePostingOutcome::SkippedAlready);
    let session_after_replay = balance_map(
        &read_session_balances(&db, &session_id, Some(owner))
            .await
            .expect("read session after replay"),
    );
    assert_eq!(session_after_first, session_after_replay);
    assert_eq!(
        session_posts_first,
        gl_posting_count(&db, &close_event_id).await
    );

    let collect_sig = format!("sig-c4-collect-{}", Uuid::new_v4());
    let collect = json!({
        "event": "bot_collect_fees",
        "signature": collect_sig,
        "rebalance_session_id": session_id,
        "fee_payer_pubkey": owner,
        "details": {
            "token_mint_a": WSOL_MINT,
            "token_mint_b": USDC_MINT
        }
    });
    let collect_first =
        apply_session_postings_from_lifecycle_row(&db, &collect, Some(10), Some(20))
            .await
            .expect("session collect");
    assert_eq!(collect_first, SessionLifecyclePostingOutcome::Applied);
    let collect_after_first = balance_map(
        &read_session_balances(&db, &session_id, Some(owner))
            .await
            .expect("read session after collect"),
    );
    let collect_event_id = lifecycle_posting_event_id(&collect_sig);
    let collect_posts_first = gl_posting_count(&db, &collect_event_id).await;

    let collect_again =
        apply_session_postings_from_lifecycle_row(&db, &collect, Some(10), Some(20))
            .await
            .expect("session collect replay");
    assert_eq!(
        collect_again,
        SessionLifecyclePostingOutcome::SkippedAlready
    );
    let collect_after_replay = balance_map(
        &read_session_balances(&db, &session_id, Some(owner))
            .await
            .expect("read session after collect replay"),
    );
    assert_eq!(collect_after_first, collect_after_replay);
    assert_eq!(
        collect_posts_first,
        gl_posting_count(&db, &collect_event_id).await
    );

    let chain_session_id = format!("itest-c4-chain-{}", Uuid::new_v4());
    let chain_sess = format!("itest-c4-chain-sess-{}", Uuid::new_v4());
    let chain_sig = format!("sig-c4-chain-{}", Uuid::new_v4());
    let chain = close_lifecycle_json_chain(&chain_session_id, &chain_sess, &chain_sig);
    let chain_first = apply_chain_postings_from_lifecycle_row(&db, &chain, Some(25_000), Some(0))
        .await
        .expect("chain close");
    assert_eq!(chain_first, SessionLifecyclePostingOutcome::Applied);
    let chain_after_first = balance_map(
        &read_chain_balances(&db, &chain_session_id, Some(owner))
            .await
            .expect("read chain after first"),
    );
    assert!(!chain_after_first.is_empty());
    let chain_event_id = chain_lifecycle_posting_event_id(&chain_sig);
    let chain_posts_first = gl_posting_count(&db, &chain_event_id).await;
    assert!(chain_posts_first > 0);

    let chain_again = apply_chain_postings_from_lifecycle_row(&db, &chain, Some(25_000), Some(0))
        .await
        .expect("chain close replay");
    assert_eq!(chain_again, SessionLifecyclePostingOutcome::SkippedAlready);
    let chain_after_replay = balance_map(
        &read_chain_balances(&db, &chain_session_id, Some(owner))
            .await
            .expect("read chain after replay"),
    );
    assert_eq!(chain_after_first, chain_after_replay);
    assert_eq!(
        chain_posts_first,
        gl_posting_count(&db, &chain_event_id).await
    );
}
