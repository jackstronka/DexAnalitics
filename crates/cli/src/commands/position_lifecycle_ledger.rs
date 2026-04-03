//! Append-only JSONL ledger for Orca Whirlpool **position open / close** transactions.
//!
//! Each row records:
//! - **`tx_fee_lamports`**: Solana base fee from `meta.fee` (network fee only).
//! - **`fee_payer_*`**: pre/post balance and **net lamports delta** for the fee payer in that tx.
//!   - On **open**, delta is typically **negative** (fee + rent for new accounts + SOL leg of liquidity).
//!   - On **close**, delta is often **positive** (rent reclaim + returned SOL), minus `tx_fee_lamports`.
//!
//! Summing `fee_payer_net_lamports_delta` across open+close txs approximates **net SOL** movement from
//! those txs; **token** legs (USDC, etc.) are not converted to USD here — use mint fields + your prices.
//!
//! For a **swap + rebalance + open** sequence, set **`CLMM_REBALANCE_SESSION_ID`** (same value for each
//! step) so every row carries `rebalance_session_id`; sum costs per session (see `orca-swap` + bot rows).
//!
//! Default path: `data/ledger/orca_position_lifecycle.jsonl`  
//! Override: `CLMM_POSITION_LIFECYCLE_LEDGER_PATH`  
//! Legacy alias: `CLMM_POSITION_OPEN_LEDGER_PATH` (same as lifecycle path if lifecycle unset).

use anyhow::{Context, Result};
use clmm_lp_protocols::ledger::tx_lifecycle::{
    append_jsonl_line, enrich_tx_costs, rebalance_session_id_from_env,
};
use clmm_lp_protocols::orca::pool_reader::WhirlpoolState;
use clmm_lp_protocols::rpc::RpcProvider;
use serde::Serialize;
use serde_json::Value;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use tokio::time::{Duration, sleep};
use tracing::warn;

#[derive(Debug, Serialize)]
pub struct PositionLifecycleRecord {
    pub schema_version: u32,
    pub ts_utc: String,
    /// `cli` (this module) vs `orca_bot` (rebalance executor).
    pub source: &'static str,
    /// `position_open` | `position_close`
    pub event: &'static str,
    pub pool_address: String,
    pub mint_a: String,
    pub mint_b: String,
    pub mint_decimals_a: u8,
    pub mint_decimals_b: u8,
    pub token_max_a_raw: Option<String>,
    pub token_max_b_raw: Option<String>,
    pub token_max_a_ui: Option<f64>,
    pub token_max_b_ui: Option<f64>,
    /// `post - pre` for fee payer's SPL token balance of mint A (base units).
    /// Added for `position_close` ledger rows to make token refunds visible.
    pub token_a_net_delta_raw: Option<String>,
    /// Same as `token_a_net_delta_raw`, converted to UI units using `mint_decimals_a`.
    pub token_a_net_delta_ui: Option<f64>,
    /// Same as `token_b_net_delta_raw`, for mint B.
    pub token_b_net_delta_raw: Option<String>,
    /// Same as `token_b_net_delta_ui`, for mint B.
    pub token_b_net_delta_ui: Option<f64>,
    pub range_mode: Option<String>,
    pub tick_lower: Option<i32>,
    pub tick_upper: Option<i32>,
    pub range_width_pct: Option<f64>,
    pub slippage_bps: Option<u16>,
    pub position_pda: Option<String>,
    pub signature: String,
    pub tx_fee_lamports: u64,
    pub fee_payer_pubkey: String,
    pub fee_payer_pre_lamports: Option<u64>,
    pub fee_payer_post_lamports: Option<u64>,
    /// `post - pre` for fee payer (negative = net SOL left wallet in this tx).
    pub fee_payer_net_lamports_delta: Option<i64>,
    pub accounting_note: String,
    pub slot: Option<u64>,
    pub rpc_url: String,
    /// When `CLMM_REBALANCE_SESSION_ID` is set, ties this tx to swap / bot rows for total-cost sums.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rebalance_session_id: Option<String>,
}

fn raw_to_ui(raw: u64, decimals: u8) -> f64 {
    let scale = 10_f64.powi(i32::from(decimals));
    raw as f64 / scale
}

fn raw_i128_to_ui(raw: i128, decimals: u8) -> f64 {
    let scale = 10_f64.powi(i32::from(decimals));
    raw as f64 / scale
}

fn as_arr<'a>(v: &'a Value, k1: &str, k2: &str) -> &'a [Value] {
    v.get(k1)
        .and_then(|x| x.as_array())
        .or_else(|| v.get(k2).and_then(|x| x.as_array()))
        .map(|x| x.as_slice())
        .unwrap_or(&[])
}

fn token_amount_by_mint_owner(meta: &Value, mint: &str, owner: &str, key: &str) -> Option<u128> {
    let arr = as_arr(meta, key, key);
    let mut sum: u128 = 0;
    let mut found = false;

    for b in arr {
        let b_mint = b
            .get("mint")
            .and_then(|x| x.as_str())
            .or_else(|| b.get("tokenMint").and_then(|x| x.as_str()))
            .or_else(|| b.get("token_mint").and_then(|x| x.as_str()))?;
        if b_mint != mint {
            continue;
        }

        let b_owner = b
            .get("owner")
            .and_then(|x| x.as_str())
            .or_else(|| b.get("tokenOwner").and_then(|x| x.as_str()))
            .or_else(|| b.get("token_owner").and_then(|x| x.as_str()))?;
        if b_owner != owner {
            continue;
        }

        let amt = b
            .get("uiTokenAmount")
            .and_then(|x| x.get("amount"))
            .and_then(|x| {
                if let Some(s) = x.as_str() {
                    s.parse::<u128>().ok()
                } else {
                    x.as_u64().map(|n| n as u128)
                }
            })
            .or_else(|| {
                b.get("ui_token_amount")
                    .and_then(|x| x.get("amount"))
                    .and_then(|x| {
                        if let Some(s) = x.as_str() {
                            s.parse::<u128>().ok()
                        } else {
                            x.as_u64().map(|n| n as u128)
                        }
                    })
            });

        if let Some(amt) = amt {
            sum = sum.saturating_add(amt);
            found = true;
        }
    }

    found.then_some(sum)
}

fn u128_to_i128(x: u128) -> Option<i128> {
    i128::try_from(x).ok()
}

async fn fetch_tx_json_with_retry(provider: &RpcProvider, signature: &Signature) -> Result<Value> {
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 0u32..10 {
        if attempt > 0 {
            sleep(Duration::from_millis(80 * u64::from(attempt))).await;
        }
        match provider.get_transaction_json_parsed(signature).await {
            Ok(enc) => return serde_json::to_value(&enc).context("tx to json"),
            Err(e) => last_err = Some(e),
        }
    }

    Err(last_err
        .map(|e| anyhow::anyhow!(e))
        .unwrap_or_else(|| anyhow::anyhow!("get_transaction failed")))
}

async fn token_net_deltas_for_fee_payer_close(
    provider: &RpcProvider,
    signature: &Signature,
    fee_payer: &Pubkey,
    mint_a: &Pubkey,
    mint_b: &Pubkey,
    dec_a: u8,
    dec_b: u8,
) -> Result<(Option<String>, Option<f64>, Option<String>, Option<f64>)> {
    let tx_json = fetch_tx_json_with_retry(provider, signature).await?;
    let meta = tx_json
        .get("meta")
        .ok_or_else(|| anyhow::anyhow!("tx meta missing"))?;

    let owner_s = fee_payer.to_string();
    let mint_a_s = mint_a.to_string();
    let mint_b_s = mint_b.to_string();

    let pre_a = token_amount_by_mint_owner(meta, &mint_a_s, &owner_s, "preTokenBalances");
    let post_a = token_amount_by_mint_owner(meta, &mint_a_s, &owner_s, "postTokenBalances");
    let (token_a_net_delta_raw, token_a_net_delta_ui) = match (pre_a, post_a) {
        (None, None) => (None, None),
        (pre, post) => {
            let pre = pre.unwrap_or(0);
            let post = post.unwrap_or(0);
            let pre_i = u128_to_i128(pre).ok_or_else(|| anyhow::anyhow!("pre_a over i128"))?;
            let post_i = u128_to_i128(post).ok_or_else(|| anyhow::anyhow!("post_a over i128"))?;
            let delta = post_i - pre_i;
            (Some(delta.to_string()), Some(raw_i128_to_ui(delta, dec_a)))
        }
    };

    let pre_b = token_amount_by_mint_owner(meta, &mint_b_s, &owner_s, "preTokenBalances");
    let post_b = token_amount_by_mint_owner(meta, &mint_b_s, &owner_s, "postTokenBalances");
    let (token_b_net_delta_raw, token_b_net_delta_ui) = match (pre_b, post_b) {
        (None, None) => (None, None),
        (pre, post) => {
            let pre = pre.unwrap_or(0);
            let post = post.unwrap_or(0);
            let pre_i = u128_to_i128(pre).ok_or_else(|| anyhow::anyhow!("pre_b over i128"))?;
            let post_i = u128_to_i128(post).ok_or_else(|| anyhow::anyhow!("post_b over i128"))?;
            let delta = post_i - pre_i;
            (Some(delta.to_string()), Some(raw_i128_to_ui(delta, dec_b)))
        }
    };

    Ok((
        token_a_net_delta_raw,
        token_a_net_delta_ui,
        token_b_net_delta_raw,
        token_b_net_delta_ui,
    ))
}

async fn mint_decimals(provider: &RpcProvider, mint: &Pubkey) -> Result<u8> {
    let acc = provider
        .get_account(mint)
        .await
        .with_context(|| format!("get mint account {mint}"))?;
    acc.data
        .get(44)
        .copied()
        .ok_or_else(|| anyhow::anyhow!("mint account data too short"))
}

/// Best-effort append after successful `orca-position-open`.
pub async fn try_append_position_open_cost_ledger(
    provider: &RpcProvider,
    pool_state: &WhirlpoolState,
    amount_a: u64,
    amount_b: u64,
    slippage_bps: u16,
    signature: &Signature,
    range_mode: &str,
    tick_lower: Option<i32>,
    tick_upper: Option<i32>,
    range_width_pct: Option<f64>,
    position_pda: Option<String>,
    fee_payer: &Pubkey,
    source: &'static str,
) {
    if let Err(e) = append_open_inner(
        provider,
        pool_state,
        amount_a,
        amount_b,
        slippage_bps,
        signature,
        range_mode,
        tick_lower,
        tick_upper,
        range_width_pct,
        position_pda,
        fee_payer,
        source,
    )
    .await
    {
        warn!(error = %e, "position lifecycle ledger (open): append failed");
    }
}

async fn append_open_inner(
    provider: &RpcProvider,
    pool_state: &WhirlpoolState,
    amount_a: u64,
    amount_b: u64,
    slippage_bps: u16,
    signature: &Signature,
    range_mode: &str,
    tick_lower: Option<i32>,
    tick_upper: Option<i32>,
    range_width_pct: Option<f64>,
    position_pda: Option<String>,
    fee_payer: &Pubkey,
    source: &'static str,
) -> Result<()> {
    let dec_a = mint_decimals(provider, &pool_state.token_mint_a)
        .await
        .unwrap_or_else(|e| {
            warn!(mint_a = %pool_state.token_mint_a, err = %e, "mint decimals A failed; using 0");
            0
        });
    let dec_b = mint_decimals(provider, &pool_state.token_mint_b)
        .await
        .unwrap_or_else(|e| {
            warn!(mint_b = %pool_state.token_mint_b, err = %e, "mint decimals B failed; using 0");
            0
        });

    let (tx_fee, slot, pre, post, delta) = enrich_tx_costs(provider, signature, fee_payer).await;
    let rpc_url = provider.current_endpoint().await;

    let rec = PositionLifecycleRecord {
        schema_version: 2,
        ts_utc: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        source,
        event: "position_open",
        pool_address: pool_state.address.clone(),
        mint_a: pool_state.token_mint_a.to_string(),
        mint_b: pool_state.token_mint_b.to_string(),
        mint_decimals_a: dec_a,
        mint_decimals_b: dec_b,
        token_max_a_raw: Some(amount_a.to_string()),
        token_max_b_raw: Some(amount_b.to_string()),
        token_max_a_ui: Some(raw_to_ui(amount_a, dec_a)),
        token_max_b_ui: Some(raw_to_ui(amount_b, dec_b)),
        token_a_net_delta_raw: None,
        token_a_net_delta_ui: None,
        token_b_net_delta_raw: None,
        token_b_net_delta_ui: None,
        range_mode: Some(range_mode.to_string()),
        tick_lower,
        tick_upper,
        range_width_pct,
        slippage_bps: Some(slippage_bps),
        position_pda,
        signature: signature.to_string(),
        tx_fee_lamports: tx_fee,
        fee_payer_pubkey: fee_payer.to_string(),
        fee_payer_pre_lamports: pre,
        fee_payer_post_lamports: post,
        fee_payer_net_lamports_delta: delta,
        accounting_note: "tx_fee_lamports=network fee only; fee_payer_net_lamports_delta includes fee+rent+SOL deposited as liquidity. Token legs in mint_a/b (not USD).".to_string(),
        slot,
        rpc_url,
        rebalance_session_id: rebalance_session_id_from_env(),
    };

    append_jsonl_line(&rec)
}

/// Best-effort append after successful `orca-position-close`.
pub async fn try_append_position_close_ledger(
    provider: &RpcProvider,
    pool_state: &WhirlpoolState,
    position_pda: &Pubkey,
    fee_payer: &Pubkey,
    signature: &Signature,
) {
    if let Err(e) =
        append_close_inner(provider, pool_state, position_pda, fee_payer, signature).await
    {
        warn!(error = %e, "position lifecycle ledger (close): append failed");
    }
}

async fn append_close_inner(
    provider: &RpcProvider,
    pool_state: &WhirlpoolState,
    position_pda: &Pubkey,
    fee_payer: &Pubkey,
    signature: &Signature,
) -> Result<()> {
    let dec_a = mint_decimals(provider, &pool_state.token_mint_a)
        .await
        .unwrap_or(0);
    let dec_b = mint_decimals(provider, &pool_state.token_mint_b)
        .await
        .unwrap_or(0);

    let (tx_fee, slot, pre, post, delta) = enrich_tx_costs(provider, signature, fee_payer).await;
    let rpc_url = provider.current_endpoint().await;

    // Best-effort: token balances are not part of SOL fee delta, so compute token "refunds" from
    // parsed tx meta. This makes token inflow/outflow visible for close.
    let (token_a_net_delta_raw, token_a_net_delta_ui, token_b_net_delta_raw, token_b_net_delta_ui) =
        match token_net_deltas_for_fee_payer_close(
            provider,
            signature,
            fee_payer,
            &pool_state.token_mint_a,
            &pool_state.token_mint_b,
            dec_a,
            dec_b,
        )
        .await
        {
            Ok(v) => v,
            Err(e) => {
                warn!(err = %e, "position lifecycle ledger (close): token delta extraction failed");
                (None, None, None, None)
            }
        };

    let rec = PositionLifecycleRecord {
        schema_version: 2,
        ts_utc: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        source: "cli",
        event: "position_close",
        pool_address: pool_state.address.clone(),
        mint_a: pool_state.token_mint_a.to_string(),
        mint_b: pool_state.token_mint_b.to_string(),
        mint_decimals_a: dec_a,
        mint_decimals_b: dec_b,
        token_max_a_raw: None,
        token_max_b_raw: None,
        token_max_a_ui: None,
        token_max_b_ui: None,
        token_a_net_delta_raw,
        token_a_net_delta_ui,
        token_b_net_delta_raw,
        token_b_net_delta_ui,
        range_mode: None,
        tick_lower: None,
        tick_upper: None,
        range_width_pct: None,
        slippage_bps: None,
        position_pda: Some(position_pda.to_string()),
        signature: signature.to_string(),
        tx_fee_lamports: tx_fee,
        fee_payer_pubkey: fee_payer.to_string(),
        fee_payer_pre_lamports: pre,
        fee_payer_post_lamports: post,
        fee_payer_net_lamports_delta: delta,
        accounting_note: "On close, fee_payer_net_lamports_delta often positive (rent reclaim + SOL from liquidity) minus effects of tx_fee_lamports; token_a_net_delta_* and token_b_net_delta_* are best-effort deltas extracted from tx pre/postTokenBalances for fee_payer owner+mint.".to_string(),
        slot,
        rpc_url,
        rebalance_session_id: rebalance_session_id_from_env(),
    };

    append_jsonl_line(&rec)
}
