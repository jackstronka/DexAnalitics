//! Phase C: mirror wallet journal deltas from lifecycle rows (close/collect paths).

use crate::models::WalletLedgerDelta;
use crate::services::wallet_ledger::decode_status;
use clmm_lp_data::wallet_session::{
    find_lifecycle_row_by_signature, session_mint_deltas_from_lifecycle_json,
};
use clmm_lp_protocols::ledger::tx_lifecycle::ledger_read_path;

fn default_mint_decimals(mint: &str) -> u8 {
    if mint == clmm_lp_data::wallet_session::WSOL_MINT {
        9
    } else if mint == clmm_lp_data::wallet_session::USDC_MINT {
        6
    } else {
        9
    }
}

fn postings_to_ledger_deltas(postings: &[(String, i128)]) -> Vec<WalletLedgerDelta> {
    postings
        .iter()
        .filter(|(_, d)| *d != 0)
        .map(|(mint, delta)| WalletLedgerDelta {
            mint: mint.clone(),
            decimals: default_mint_decimals(mint),
            raw_delta_i128: delta.to_string(),
        })
        .collect()
}

/// Best-effort: read lifecycle tail for `signature` and build journal deltas (close/collect/swap).
#[must_use]
pub fn journal_mirror_from_lifecycle_signature(
    signature: &str,
) -> Option<(Vec<WalletLedgerDelta>, &'static str)> {
    let (v, lp_a, lp_b) = find_lifecycle_row_by_signature(ledger_read_path(), signature, 1_000)?;
    let (_, _, _, postings) = session_mint_deltas_from_lifecycle_json(&v, lp_a, lp_b)?;
    let deltas = postings_to_ledger_deltas(&postings);
    if deltas.is_empty() {
        return None;
    }
    Some((deltas, decode_status::LIFECYCLE_MIRROR))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clmm_lp_data::wallet_session::{USDC_MINT, WSOL_MINT};
    use std::io::Write;
    use std::str::FromStr;

    #[test]
    fn journal_mirror_reads_close_row_from_temp_lifecycle() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("lifecycle.jsonl");
        let line = serde_json::json!({
            "event": "bot_close_position",
            "signature": "sig-close-mirror-test",
            "rebalance_session_id": "sess-1",
            "details": {
                "token_mint_a": WSOL_MINT,
                "token_mint_b": USDC_MINT,
                "close_amount_a_raw": 1_000u64,
                "close_amount_b_raw": 500u64
            }
        });
        let mut f = std::fs::File::create(&path).expect("create");
        writeln!(f, "{line}").expect("write");

        let (v, lp_a, lp_b) =
            find_lifecycle_row_by_signature(&path, "sig-close-mirror-test", 10).expect("row");
        assert!(lp_a.is_none());
        let (_, _, _, postings) =
            session_mint_deltas_from_lifecycle_json(&v, lp_a, lp_b).expect("deltas");
        let ledger = postings_to_ledger_deltas(&postings);
        assert_eq!(ledger.len(), 2);
        assert!(i128::from_str(&ledger[0].raw_delta_i128).unwrap_or(0) > 0);
    }
}
