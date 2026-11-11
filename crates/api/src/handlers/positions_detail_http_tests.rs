//! API-H5.1: hermetic typed HTTP for stream-pnl / lifecycle-summary / diagnostics envelopes.

use super::http_test_support::{get_json, test_router, test_state};
use crate::models::{
    PositionDiagnosticsResponse, PositionLifecycleSummaryResponse, PositionStreamPnLResponse,
};
use crate::test_env::EnvGuard;
use axum::http::StatusCode;
use rust_decimal::Decimal;
use solana_sdk::pubkey::Pubkey;
use std::io::Write;
use std::str::FromStr;

fn fixture_pda() -> String {
    Pubkey::new_unique().to_string()
}

/// API-H5.1: invalid address → 400 on all three detail routes.
#[tokio::test]
async fn detail_routes_reject_invalid_address() {
    let router = test_router(test_state());
    for path in [
        "/api/v1/positions/not-a-pubkey/stream-pnl",
        "/api/v1/positions/not-a-pubkey/lifecycle-summary",
        "/api/v1/positions/not-a-pubkey/diagnostics",
    ] {
        let (status, body) = get_json(router.clone(), path).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "path={path} body={body}");
        assert!(
            body.to_ascii_lowercase().contains("invalid"),
            "path={path} body={body}"
        );
    }
}

/// API-H5.1: diagnostics without monitor/strategies → typed empty envelope.
#[tokio::test]
async fn diagnostics_unknown_position_deserializes() {
    let pda = fixture_pda();
    let (status, body) = get_json(
        test_router(test_state()),
        &format!("/api/v1/positions/{pda}/diagnostics"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let resp: PositionDiagnosticsResponse =
        serde_json::from_str(&body).expect("PositionDiagnosticsResponse");
    assert_eq!(resp.address, pda);
    assert!(!resp.in_monitor);
    assert!(resp.monitor_in_range.is_none());
    assert!(resp.linked_strategies.is_empty());
}

/// API-H5.1: no DB → stream-pnl degraded envelope (zeros + note), not 500.
#[tokio::test]
async fn stream_pnl_no_db_deserializes_degraded_envelope() {
    let pda = fixture_pda();
    let (status, body) = get_json(
        test_router(test_state()),
        &format!("/api/v1/positions/{pda}/stream-pnl"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let resp: PositionStreamPnLResponse =
        serde_json::from_str(&body).expect("PositionStreamPnLResponse");
    assert_eq!(resp.position_address, pda);
    assert_eq!(resp.baseline_value_usd, Decimal::ZERO);
    assert_eq!(resp.net_pnl_usd, Decimal::ZERO);
    assert_eq!(resp.valuation_price_time_kind, "unavailable");
    assert_eq!(resp.economic_quality.as_deref(), Some("degraded"));
    assert_eq!(resp.end_nav_source.as_deref(), Some("missing"));
    let note = resp.note.unwrap_or_default();
    assert!(
        note.to_ascii_lowercase().contains("db"),
        "expected DB-disabled note, got {note}"
    );
}

/// API-H5.1: settlement mode same no-DB envelope class.
#[tokio::test]
async fn stream_pnl_settlement_mode_no_db_deserializes() {
    let pda = fixture_pda();
    let (status, body) = get_json(
        test_router(test_state()),
        &format!("/api/v1/positions/{pda}/stream-pnl?mode=settlement_v1"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let resp: PositionStreamPnLResponse =
        serde_json::from_str(&body).expect("PositionStreamPnLResponse");
    assert_eq!(resp.position_address, pda);
    assert_eq!(resp.economic_quality.as_deref(), Some("degraded"));
}

/// API-H5.1: missing lifecycle ledger → typed empty summary + missing-file note.
#[tokio::test]
async fn lifecycle_summary_missing_ledger_deserializes() {
    let mut env = EnvGuard::lock().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = dir.path().join("lifecycle-missing.jsonl");
    env.set("CLMM_POSITION_LIFECYCLE_LEDGER_READ_PATH", &ledger);
    env.set("CLMM_POSITION_LIFECYCLE_LEDGER_PATH", &ledger);
    env.remove("CLMM_POSITION_LIFECYCLE_USE_ENRICHED");
    assert!(!ledger.exists());

    let pda = fixture_pda();
    let (status, body) = get_json(
        test_router(test_state()),
        &format!("/api/v1/positions/{pda}/lifecycle-summary"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let resp: PositionLifecycleSummaryResponse =
        serde_json::from_str(&body).expect("PositionLifecycleSummaryResponse");
    assert_eq!(resp.position_address, pda);
    assert_eq!(resp.positions, vec![pda.clone()]);
    assert!(resp.sessions.is_empty());
    assert_eq!(resp.total_tx_fee_lamports, 0);
    assert_eq!(resp.collect_events, 0);
    assert!(resp.session_summaries.is_empty());
    let note = resp.note.unwrap_or_default();
    assert!(
        note.to_ascii_lowercase().contains("missing")
            || note.to_ascii_lowercase().contains("ledger"),
        "note={note}"
    );
}

/// API-H5.1: seeded lifecycle row for the PDA → fees/collect fields populated.
#[tokio::test]
async fn lifecycle_summary_seeded_ledger_aggregates() {
    let mut env = EnvGuard::lock().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = dir.path().join("lifecycle.jsonl");
    env.set("CLMM_POSITION_LIFECYCLE_LEDGER_READ_PATH", &ledger);
    env.set("CLMM_POSITION_LIFECYCLE_LEDGER_PATH", &ledger);
    env.remove("CLMM_POSITION_LIFECYCLE_USE_ENRICHED");

    let pda = fixture_pda();
    // Valid base58 unrelated PDA must not match.
    let other = Pubkey::new_unique().to_string();
    let mut f = std::fs::File::create(&ledger).expect("create ledger");
    writeln!(
        f,
        "{}",
        serde_json::json!({
            "source": "orca_bot",
            "rebalance_session_id": "sess-h5-life-1",
            "event": "bot_collect_fees",
            "ts_utc": "2026-01-01T00:00:00Z",
            "position_pubkey": pda,
            "tx_fee_lamports": 5000,
        })
    )
    .expect("write match row");
    writeln!(
        f,
        "{}",
        serde_json::json!({
            "source": "orca_bot",
            "rebalance_session_id": "sess-h5-other",
            "event": "bot_collect_fees",
            "ts_utc": "2026-01-01T00:00:01Z",
            "position_pubkey": other,
            "tx_fee_lamports": 99999,
        })
    )
    .expect("write other row");

    let (status, body) = get_json(
        test_router(test_state()),
        &format!("/api/v1/positions/{pda}/lifecycle-summary"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let resp: PositionLifecycleSummaryResponse =
        serde_json::from_str(&body).expect("PositionLifecycleSummaryResponse");
    assert_eq!(resp.position_address, pda);
    assert_eq!(resp.total_tx_fee_lamports, 5000);
    assert_eq!(resp.collect_events, 1);
    assert!(
        resp.session_summaries
            .iter()
            .any(|s| s.session_id == "sess-h5-life-1"),
        "session_summaries={:?}",
        resp.session_summaries
    );
    // Sanity: pubkey still parses after round-trip path.
    assert!(Pubkey::from_str(&resp.position_address).is_ok());
}
