use anyhow::Result;
use chrono::Utc;
use clmm_lp_protocols::events::{
    parse_meteora_swap_event_for_pool, parse_raydium_swap_event_for_pool,
    parse_traded_event_for_pool,
};
use clmm_lp_protocols::rpc::{RpcConfig, RpcProvider};
use futures::stream::{self, StreamExt};
use serde::Serialize;
use solana_client::pubsub_client::PubsubClient;
use solana_client::rpc_client::GetConfirmedSignaturesForAddress2Config;
use solana_client::rpc_config::{
    RpcTransactionConfig, RpcTransactionLogsConfig, RpcTransactionLogsFilter,
};
use solana_client::rpc_response::Response as RpcResponse;
use solana_client::rpc_response::RpcLogsResponse;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use solana_transaction_status_client_types::UiTransactionEncoding;
use spl_token::solana_program::program_pack::Pack;
use spl_token::state::Account as SplTokenAccount;
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::Mutex;
use tokio::time::{Duration, sleep, timeout};

#[derive(Debug, Clone, Copy)]
enum Proto {
    Orca,
    Raydium,
    Meteora,
}

impl Proto {
    fn dir(self) -> &'static str {
        match self {
            Proto::Orca => "orca",
            Proto::Raydium => "raydium",
            Proto::Meteora => "meteora",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "orca" => Some(Proto::Orca),
            "raydium" => Some(Proto::Raydium),
            "meteora" => Some(Proto::Meteora),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize)]
struct RawChainTx {
    ts_utc: String,
    protocol: String,
    pool_address: String,
    signature: String,
    slot: u64,
    block_time: Option<i64>,
    confirmation_status: Option<String>,
    err: Option<String>,
    source: String,
    schema_version: u8,
}

#[derive(Debug, Clone)]
struct PoolMeta {
    token_vault_a: String,
    token_vault_b: String,
    token_mint_a: String,
    token_mint_b: String,
    token_vault_owner_a: Option<String>,
    token_vault_owner_b: Option<String>,
}

fn is_debug_meteora_pool(pool: &str) -> bool {
    // Temporary targeted debug path for Meteora investigations.
    pool == "5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6"
}

#[derive(Debug, Serialize)]
struct DecodedSwapTx {
    ts_utc: String,
    protocol: String,
    pool_address: String,
    signature: String,
    slot: u64,
    block_time: Option<i64>,
    success: bool,
    tx_fee_lamports: Option<u64>,
    token_mint_a: String,
    token_mint_b: String,
    vault_a_delta_raw: Option<i128>,
    vault_b_delta_raw: Option<i128>,
    amount_in_raw: Option<u128>,
    amount_out_raw: Option<u128>,
    fee_amount_raw: Option<u128>,
    direction: Option<String>,
    tick_after: Option<i32>,
    sqrt_price_x64_after: Option<u128>,
    log_swap_mentions: u32,
    has_swap_log: bool,
    decode_status: String,
    source: String,
    schema_version: u8,
}

#[derive(Debug, Default)]
struct DecodeQuality {
    raw_rows: usize,
    decoded_rows: usize,
    ok_rows: usize,
    status_counts: BTreeMap<String, usize>,
    latest_block_time: Option<i64>,
}

#[derive(Debug, serde::Serialize)]
struct DecodeAuditRow {
    protocol: String,
    pool_address: String,
    raw_rows: usize,
    decoded_rows: usize,
    ok_rows: usize,
    ok_pct: f64,
    latest_block_time: Option<i64>,
    status_counts: BTreeMap<String, usize>,
}

fn parse_curated_pool_addrs(protocol: Proto) -> Result<Vec<String>> {
    let startup_path = std::path::Path::new("STARTUP.md");
    let content = std::fs::read_to_string(startup_path)?;
    let section_marker = match protocol {
        Proto::Orca => "**Orca",
        Proto::Raydium => "**Raydium",
        Proto::Meteora => "**Meteora",
    };
    let start = content.find(section_marker).ok_or_else(|| {
        anyhow::anyhow!("Could not find {} section in STARTUP.md", section_marker)
    })?;
    let rest = &content[start..];
    let mut addrs: Vec<String> = Vec::new();
    for line in rest.lines().skip(1) {
        let t = line.trim();
        if t.starts_with("**") && !t.contains(section_marker) {
            break;
        }
        if let Some(b) = t.find('`')
            && let Some(e_rel) = t[b + 1..].find('`')
        {
            let candidate = &t[b + 1..b + 1 + e_rel];
            if (32..=48).contains(&candidate.len()) && Pubkey::from_str(candidate).is_ok() {
                addrs.push(candidate.to_string());
            }
        }
    }
    addrs.sort();
    addrs.dedup();
    Ok(addrs)
}

fn existing_sigs(path: &PathBuf) -> HashSet<String> {
    let mut set = HashSet::new();
    let Ok(txt) = std::fs::read_to_string(path) else {
        return set;
    };
    for line in txt.lines().filter(|l| !l.trim().is_empty()) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line)
            && let Some(sig) = v.get("signature").and_then(|x| x.as_str())
        {
            set.insert(sig.to_string());
        }
    }
    set
}

fn latest_pool_meta(protocol: Proto, pool: &str) -> Option<PoolMeta> {
    let mut p = PathBuf::from("data");
    p.push("pool-snapshots");
    p.push(protocol.dir());
    p.push(pool);
    p.push("snapshots.jsonl");
    let txt = std::fs::read_to_string(p).ok()?;
    let mut last: Option<serde_json::Value> = None;
    for line in txt.lines().filter(|l| !l.trim().is_empty()) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            last = Some(v);
        }
    }
    let v = last?;
    Some(PoolMeta {
        token_vault_a: v.get("token_vault_a")?.as_str()?.to_string(),
        token_vault_b: v.get("token_vault_b")?.as_str()?.to_string(),
        token_mint_a: v.get("token_mint_a")?.as_str()?.to_string(),
        token_mint_b: v.get("token_mint_b")?.as_str()?.to_string(),
        token_vault_owner_a: v
            .get("token_vault_owner_a")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string()),
        token_vault_owner_b: v
            .get("token_vault_owner_b")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string()),
    })
}

async fn fill_meteora_token_vault_owners(
    rpc_provider: &RpcProvider,
    pool: &str,
    meta: &mut PoolMeta,
) -> Result<()> {
    // This is used as a robustness fallback when snapshot parsing can't retrieve
    // SPL token reserve accounts (e.g. RPC transient failures).
    if meta.token_vault_owner_a.is_some() && meta.token_vault_owner_b.is_some() {
        return Ok(());
    }

    if meta.token_vault_owner_a.is_none() {
        let pk = Pubkey::from_str(&meta.token_vault_a)?;
        if let Ok(Ok(acct)) = timeout(Duration::from_secs(4), rpc_provider.get_account(&pk)).await {
            if let Ok(spl) = SplTokenAccount::unpack(&acct.data) {
                meta.token_vault_owner_a = Some(spl.owner.to_string());
                if is_debug_meteora_pool(pool) {
                    eprintln!(
                        "[meteora-debug] owner-fill a ok pool={} vault_a={} owner_a={}",
                        pool, meta.token_vault_a, spl.owner
                    );
                }
            }
        } else if is_debug_meteora_pool(pool) {
            eprintln!(
                "[meteora-debug] owner-fill a failed pool={} vault_a={}",
                pool, meta.token_vault_a
            );
        }
    }
    if meta.token_vault_owner_b.is_none() {
        let pk = Pubkey::from_str(&meta.token_vault_b)?;
        if let Ok(Ok(acct)) = timeout(Duration::from_secs(4), rpc_provider.get_account(&pk)).await {
            if let Ok(spl) = SplTokenAccount::unpack(&acct.data) {
                meta.token_vault_owner_b = Some(spl.owner.to_string());
                if is_debug_meteora_pool(pool) {
                    eprintln!(
                        "[meteora-debug] owner-fill b ok pool={} vault_b={} owner_b={}",
                        pool, meta.token_vault_b, spl.owner
                    );
                }
            }
        } else if is_debug_meteora_pool(pool) {
            eprintln!(
                "[meteora-debug] owner-fill b failed pool={} vault_b={}",
                pool, meta.token_vault_b
            );
        }
    }

    if is_debug_meteora_pool(pool) {
        eprintln!(
            "[meteora-debug] owner-fill final pool={} has_owner_a={} has_owner_b={}",
            pool,
            meta.token_vault_owner_a.is_some(),
            meta.token_vault_owner_b.is_some()
        );
    }

    Ok(())
}

fn as_arr<'a>(v: &'a serde_json::Value, k1: &str, k2: &str) -> &'a [serde_json::Value] {
    v.get(k1)
        .and_then(|x| x.as_array())
        .or_else(|| v.get(k2).and_then(|x| x.as_array()))
        .map(|x| x.as_slice())
        .unwrap_or(&[])
}

fn token_amount_by_index(meta: &serde_json::Value, account_index: u64, key: &str) -> Option<u128> {
    let arr = as_arr(meta, key, key);
    for b in arr {
        let idx = b
            .get("accountIndex")
            .and_then(|x| x.as_u64())
            .or_else(|| b.get("account_index").and_then(|x| x.as_u64()))?;
        if idx != account_index {
            continue;
        }
        let amt = b
            .get("uiTokenAmount")
            .and_then(|x| x.get("amount"))
            .and_then(|x| x.as_str())
            .or_else(|| {
                b.get("ui_token_amount")
                    .and_then(|x| x.get("amount"))
                    .and_then(|x| x.as_str())
            })?;
        return amt.parse::<u128>().ok();
    }
    None
}

fn token_amount_by_mint_owner(
    meta: &serde_json::Value,
    mint: &str,
    owner: &str,
    key: &str,
) -> Option<u128> {
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
            .and_then(|x| x.as_str())
            .or_else(|| {
                b.get("ui_token_amount")
                    .and_then(|x| x.get("amount"))
                    .and_then(|x| x.as_str())
            })?
            .parse::<u128>()
            .ok()?;

        sum = sum.saturating_add(amt);
        found = true;
    }

    found.then_some(sum)
}

fn choose_largest_mint_delta(meta: &serde_json::Value, mint: &str) -> Option<i128> {
    largest_mint_deltas(meta)
        .into_iter()
        .find_map(|(m, d)| if m == mint { Some(d) } else { None })
}

fn largest_mint_deltas(meta: &serde_json::Value) -> Vec<(String, i128)> {
    let pre_arr = as_arr(meta, "preTokenBalances", "pre_token_balances");
    let post_arr = as_arr(meta, "postTokenBalances", "post_token_balances");

    fn parse_amt(b: &serde_json::Value) -> Option<u128> {
        b.get("uiTokenAmount")
            .and_then(|x| x.get("amount"))
            .and_then(|x| {
                if let Some(s) = x.as_str() {
                    s.parse::<u128>().ok()
                } else if let Some(n) = x.as_u64() {
                    Some(n as u128)
                } else {
                    None
                }
            })
            .or_else(|| {
                b.get("ui_token_amount")
                    .and_then(|x| x.get("amount"))
                    .and_then(|x| x.as_str()?.parse::<u128>().ok())
            })
    }

    fn parse_mint(b: &serde_json::Value) -> Option<&str> {
        b.get("mint")
            .and_then(|x| x.as_str())
            .or_else(|| b.get("tokenMint").and_then(|x| x.as_str()))
            .or_else(|| b.get("token_mint").and_then(|x| x.as_str()))
    }

    fn parse_idx(b: &serde_json::Value) -> Option<u64> {
        b.get("accountIndex")
            .and_then(|x| x.as_u64())
            .or_else(|| b.get("account_index").and_then(|x| x.as_u64()))
    }

    let mut pre_by_key: BTreeMap<(u64, String), u128> = BTreeMap::new();
    for b in pre_arr {
        let Some(mint) = parse_mint(&b) else { continue };
        let Some(idx) = parse_idx(&b) else { continue };
        let Some(amt) = parse_amt(&b) else { continue };
        pre_by_key.insert((idx, mint.to_string()), amt);
    }

    let mut post_by_key: BTreeMap<(u64, String), u128> = BTreeMap::new();
    for b in post_arr {
        let Some(post_mint) = parse_mint(&b) else {
            continue;
        };
        let Some(idx) = parse_idx(&b) else { continue };
        let Some(post_amt) = parse_amt(&b) else {
            continue;
        };
        post_by_key.insert((idx, post_mint.to_string()), post_amt);
    }

    let mut best_by_mint: BTreeMap<String, i128> = BTreeMap::new();
    // Compute deltas per (idx,mint), defaulting missing side to 0.
    for (k, pre_amt) in &pre_by_key {
        let post_amt = *post_by_key.get(k).unwrap_or(&0);
        let delta = post_amt as i128 - *pre_amt as i128;
        let e = best_by_mint.entry(k.1.clone()).or_insert(0);
        if delta.abs() > e.abs() {
            *e = delta;
        }
    }
    for (k, post_amt) in &post_by_key {
        if pre_by_key.contains_key(k) {
            continue;
        }
        let delta = *post_amt as i128; // pre assumed 0
        let e = best_by_mint.entry(k.1.clone()).or_insert(0);
        if delta.abs() > e.abs() {
            *e = delta;
        }
    }

    let mut out = best_by_mint.into_iter().collect::<Vec<_>>();
    out.sort_by(|a, b| b.1.abs().cmp(&a.1.abs()));
    out
}

fn debug_dump_token_balances(meta: &serde_json::Value, pool: &str, sig: &Signature) {
    let pre = as_arr(meta, "preTokenBalances", "pre_token_balances");
    let post = as_arr(meta, "postTokenBalances", "post_token_balances");
    eprintln!(
        "[meteora-debug] token-balances pool={} sig={} pre_len={} post_len={}",
        pool,
        sig,
        pre.len(),
        post.len()
    );

    for (label, arr) in [("pre", pre), ("post", post)] {
        for (i, row) in arr.iter().take(2).enumerate() {
            let account_index = row
                .get("accountIndex")
                .and_then(|x| x.as_u64())
                .or_else(|| row.get("account_index").and_then(|x| x.as_u64()));
            let mint = row
                .get("mint")
                .and_then(|x| x.as_str())
                .or_else(|| row.get("tokenMint").and_then(|x| x.as_str()))
                .or_else(|| row.get("token_mint").and_then(|x| x.as_str()));
            let owner = row
                .get("owner")
                .and_then(|x| x.as_str())
                .or_else(|| row.get("tokenOwner").and_then(|x| x.as_str()))
                .or_else(|| row.get("token_owner").and_then(|x| x.as_str()));
            let amount = row
                .get("uiTokenAmount")
                .and_then(|x| x.get("amount"))
                .and_then(|x| x.as_str())
                .or_else(|| {
                    row.get("ui_token_amount")
                        .and_then(|x| x.get("amount"))
                        .and_then(|x| x.as_str())
                });

            eprintln!(
                "[meteora-debug] token-balance-sample pool={} sig={} side={} idx={} account_index={:?} mint={:?} owner={:?} amount={:?} keys={:?}",
                pool,
                sig,
                label,
                i,
                account_index,
                mint,
                owner,
                amount,
                row.as_object()
                    .map(|o| o.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default()
            );
        }
    }
}

fn parse_amount_from_transfer_info(info: &serde_json::Value) -> Option<u128> {
    info.get("amount")
        .and_then(|x| {
            if let Some(s) = x.as_str() {
                s.parse::<u128>().ok()
            } else if let Some(n) = x.as_u64() {
                Some(n as u128)
            } else {
                None
            }
        })
        .or_else(|| {
            info.get("tokenAmount")
                .and_then(|x| x.get("amount"))
                .and_then(|x| {
                    if let Some(s) = x.as_str() {
                        s.parse::<u128>().ok()
                    } else if let Some(n) = x.as_u64() {
                        Some(n as u128)
                    } else {
                        None
                    }
                })
        })
}

fn infer_vault_deltas_from_inner_instructions(
    meta: &serde_json::Value,
    pool_meta: &PoolMeta,
) -> (Option<i128>, Option<i128>) {
    let mut da: i128 = 0;
    let mut db: i128 = 0;
    let mut seen_a = false;
    let mut seen_b = false;

    let inners = as_arr(meta, "innerInstructions", "inner_instructions");
    for inner in inners {
        let instructions = as_arr(inner, "instructions", "instructions");
        for ix in instructions {
            let parsed = ix.get("parsed");
            let Some(parsed) = parsed else { continue };
            let ix_type = parsed.get("type").and_then(|x| x.as_str()).unwrap_or("");
            if ix_type != "transfer" && ix_type != "transferChecked" {
                continue;
            }
            let Some(info) = parsed.get("info") else {
                continue;
            };
            let source = info.get("source").and_then(|x| x.as_str());
            let destination = info.get("destination").and_then(|x| x.as_str());
            let Some(amount) = parse_amount_from_transfer_info(info) else {
                continue;
            };

            // Track signed vault deltas directly from transfer direction:
            // destination vault => positive, source vault => negative.
            if source == Some(pool_meta.token_vault_a.as_str()) {
                da -= amount as i128;
                seen_a = true;
            }
            if destination == Some(pool_meta.token_vault_a.as_str()) {
                da += amount as i128;
                seen_a = true;
            }
            if source == Some(pool_meta.token_vault_b.as_str()) {
                db -= amount as i128;
                seen_b = true;
            }
            if destination == Some(pool_meta.token_vault_b.as_str()) {
                db += amount as i128;
                seen_b = true;
            }
        }
    }

    (seen_a.then_some(da), seen_b.then_some(db))
}

fn account_index_of(account_keys: &[serde_json::Value], key: &str) -> Option<u64> {
    let target = key.to_ascii_lowercase();
    account_keys.iter().enumerate().find_map(|(i, v)| {
        // `getTransaction(jsonParsed)` can encode pubkeys either as:
        // - strings: "..."
        // - objects: { "pubkey": "...", ... } (or sometimes `{ "pubKey": "..." }`)
        let pubkey = if let Some(s) = v.as_str() {
            Some(s.to_string())
        } else if let Some(s) = v.get("pubkey").and_then(|x| x.as_str()) {
            Some(s.to_string())
        } else if let Some(s) = v.get("pubKey").and_then(|x| x.as_str()) {
            Some(s.to_string())
        } else {
            None
        }?;
        if pubkey.to_ascii_lowercase() == target {
            Some(i as u64)
        } else {
            None
        }
    })
}

/// `getTransaction` JSON uses `EncodedConfirmedTransactionWithStatusMeta` with `#[serde(flatten)]`,
/// so `meta` / `blockTime` sit next to `transaction`, not inside it.
fn meta_from_tx_root(j: &serde_json::Value) -> Option<&serde_json::Value> {
    j.get("meta")
        .or_else(|| j.get("transaction").and_then(|t| t.get("meta")))
}

/// Parsed tx body: usually `{ "signatures": [...], "message": { ... } }`.
fn message_from_tx_body(tx: &serde_json::Value) -> Option<&serde_json::Value> {
    tx.get("message")
        .or_else(|| tx.get("transaction").and_then(|t| t.get("message")))
}

/// Static account keys from `message` (parsed = objects with `pubkey`, raw = strings).
fn static_account_keys_from_message(message: &serde_json::Value) -> Vec<serde_json::Value> {
    let Some(arr) = message
        .get("accountKeys")
        .or_else(|| message.get("account_keys"))
        .and_then(|x| x.as_array())
    else {
        return Vec::new();
    };
    arr.iter().cloned().collect()
}

/// Full key list for balance `accountIndex` resolution: static + writable loaded + readonly loaded.
fn append_loaded_address_pubkeys(keys: &mut Vec<serde_json::Value>, meta: &serde_json::Value) {
    let loaded = meta
        .get("loadedAddresses")
        .or_else(|| meta.get("loaded_addresses"));
    let Some(loaded) = loaded else {
        return;
    };
    if loaded.is_null() {
        return;
    }
    let Some(obj) = loaded.as_object() else {
        return;
    };
    for k in ["writable", "readonly"] {
        if let Some(arr) = obj.get(k).and_then(|x| x.as_array()) {
            for addr in arr {
                if let Some(s) = addr.as_str() {
                    keys.push(serde_json::Value::String(s.to_string()));
                    continue;
                }
                // Sometimes loaded address entries are objects like `{ "pubkey": "..." }`.
                // Be tolerant to key casing.
                let pk = addr
                    .get("pubkey")
                    .and_then(|x| x.as_str())
                    .or_else(|| addr.get("pubKey").and_then(|x| x.as_str()));
                if let Some(s) = pk {
                    keys.push(serde_json::Value::String(s.to_string()));
                }
            }
        }
    }
}

fn full_account_keys_for_tx(j: &serde_json::Value) -> Vec<serde_json::Value> {
    let Some(tx_body) = j.get("transaction") else {
        return Vec::new();
    };
    let Some(message) = message_from_tx_body(tx_body) else {
        return Vec::new();
    };
    let mut keys = static_account_keys_from_message(message);
    if let Some(meta) = meta_from_tx_root(j) {
        append_loaded_address_pubkeys(&mut keys, meta);
    }
    keys
}

fn parse_first_integer_after(s: &str, marker: &str) -> Option<i128> {
    let lower = s.to_ascii_lowercase();
    let marker_l = marker.to_ascii_lowercase();
    let pos = lower.find(&marker_l)?;
    let tail = &s[pos + marker.len()..];
    let mut started = false;
    let mut buf = String::new();
    for ch in tail.chars() {
        if !started {
            if ch == '-' || ch.is_ascii_digit() {
                started = true;
                buf.push(ch);
            }
            continue;
        }
        if ch.is_ascii_digit() {
            buf.push(ch);
        } else {
            break;
        }
    }
    if buf.is_empty() || buf == "-" {
        None
    } else {
        buf.parse::<i128>().ok()
    }
}

fn extract_from_logs(logs: &[String]) -> (Option<u128>, Option<i32>, Option<u128>, u32) {
    let mut fee_amount_raw: Option<u128> = None;
    let mut tick_after: Option<i32> = None;
    let mut sqrt_price_x64_after: Option<u128> = None;
    let mut swap_mentions: u32 = 0;

    for line in logs {
        let l = line.to_ascii_lowercase();
        if l.contains("swap") {
            swap_mentions += 1;
        }
        if fee_amount_raw.is_none() {
            for m in ["fee_amount", "fee amount", "fee:"] {
                if let Some(v) =
                    parse_first_integer_after(line, m).and_then(|x| u128::try_from(x).ok())
                {
                    fee_amount_raw = Some(v);
                    break;
                }
            }
        }
        if tick_after.is_none() {
            for m in ["tick_after", "tick after", "tick:"] {
                if let Some(v) =
                    parse_first_integer_after(line, m).and_then(|x| i32::try_from(x).ok())
                {
                    tick_after = Some(v);
                    break;
                }
            }
        }
        if sqrt_price_x64_after.is_none() {
            for m in ["sqrt_price_x64_after", "sqrt_price_x64", "sqrt price"] {
                if let Some(v) =
                    parse_first_integer_after(line, m).and_then(|x| u128::try_from(x).ok())
                {
                    sqrt_price_x64_after = Some(v);
                    break;
                }
            }
        }
    }
    (
        fee_amount_raw,
        tick_after,
        sqrt_price_x64_after,
        swap_mentions,
    )
}

async fn decode_one_signature(
    rpc: &RpcProvider,
    protocol: Proto,
    pool: &str,
    pool_meta: &PoolMeta,
    sig: &str,
) -> Result<DecodedSwapTx> {
    let sig = Signature::from_str(sig)?;
    let tx = rpc
        .get_transaction_with_config(
            &sig,
            RpcTransactionConfig {
                // `JsonParsed` is significantly heavier and can time out on public RPC endpoints.
                // We only need meta (token balances/logs) + account keys, so `Json` is sufficient.
                encoding: Some(UiTransactionEncoding::Json),
                commitment: None,
                max_supported_transaction_version: Some(0),
            },
        )
        .await?;
    let j = serde_json::to_value(&tx)?;
    let slot = j.get("slot").and_then(|x| x.as_u64()).unwrap_or(0);
    let block_time = j.get("blockTime").and_then(|x| x.as_i64());
    let meta = meta_from_tx_root(&j);
    // RPC uses `err: null` for success; missing `err` treated as success.
    let success = meta
        .and_then(|m| m.get("err"))
        .map(|e| e.is_null())
        .unwrap_or(true);
    let tx_fee = meta.and_then(|m| m.get("fee")).and_then(|x| x.as_u64());
    let logs: Vec<String> = meta
        .and_then(|m| m.get("logMessages").or_else(|| m.get("log_messages")))
        .and_then(|x| x.as_array())
        .map(|logs| {
            logs.iter()
                .filter_map(|x| x.as_str())
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut has_swap_log = logs
        .iter()
        .any(|s| s.contains("Swap") || s.contains("swap"));
    let (mut fee_amount_raw, mut tick_after, mut sqrt_price_x64_after, log_swap_mentions) =
        extract_from_logs(&logs);

    let traded_ev = if matches!(protocol, Proto::Orca) {
        parse_traded_event_for_pool(&logs, pool)
    } else {
        None
    };
    let raydium_swap_ev = if matches!(protocol, Proto::Raydium) {
        parse_raydium_swap_event_for_pool(&logs, pool)
    } else {
        None
    };
    let meteora_swap_ev = if matches!(protocol, Proto::Meteora) {
        parse_meteora_swap_event_for_pool(&logs, pool)
    } else {
        None
    };
    if traded_ev.is_some() || raydium_swap_ev.is_some() || meteora_swap_ev.is_some() {
        has_swap_log = true;
    }

    let account_keys = full_account_keys_for_tx(&j);
    let debug_meteora = matches!(protocol, Proto::Meteora) && is_debug_meteora_pool(pool);

    let mut va_delta: Option<i128> = None;
    let mut vb_delta: Option<i128> = None;
    let mut amount_in_raw: Option<u128> = None;
    let mut amount_out_raw: Option<u128> = None;
    let mut direction: Option<String> = None;
    let mut out_token_mint_a = pool_meta.token_mint_a.clone();
    let mut out_token_mint_b = pool_meta.token_mint_b.clone();
    let mut decode_status = if meta.is_some() {
        "partial".to_string()
    } else {
        "missing_meta".to_string()
    };

    if let Some(m) = meta {
        if let (Some(i_a), Some(i_b)) = (
            account_index_of(&account_keys, &pool_meta.token_vault_a),
            account_index_of(&account_keys, &pool_meta.token_vault_b),
        ) {
            let pre_a = token_amount_by_index(m, i_a, "preTokenBalances");
            let post_a = token_amount_by_index(m, i_a, "postTokenBalances");
            let pre_b = token_amount_by_index(m, i_b, "preTokenBalances");
            let post_b = token_amount_by_index(m, i_b, "postTokenBalances");
            if let (Some(pa), Some(qa), Some(pb), Some(qb)) = (pre_a, post_a, pre_b, post_b) {
                let da = qa as i128 - pa as i128;
                let db = qb as i128 - pb as i128;
                va_delta = Some(da);
                vb_delta = Some(db);
                if da > 0 && db < 0 {
                    amount_in_raw = Some(da as u128);
                    amount_out_raw = Some((-db) as u128);
                    direction = Some("a_to_b".to_string());
                    decode_status = "ok".to_string();
                } else if db > 0 && da < 0 {
                    amount_in_raw = Some(db as u128);
                    amount_out_raw = Some((-da) as u128);
                    direction = Some("b_to_a".to_string());
                    decode_status = "ok".to_string();
                } else if da == 0 && db == 0 {
                    decode_status = "no_vault_change".to_string();
                } else {
                    if has_swap_log {
                        // Loose inference: when vault deltas do not have the expected
                        // opposite-sign pattern, we still try to recover an "amount_in"
                        // from the dominant delta.
                        //
                        // This is intentionally NOT marked as `ok`; it is included only
                        // when backtest uses `--fee-swap-decode-status loose`.
                        let choose_da = da.abs() >= db.abs();
                        let chosen_delta = if choose_da { da } else { db };
                        let other_delta = if choose_da { db } else { da };

                        // In a clean swap, the input vault delta is usually positive.
                        // If chosen delta is negative, assume we picked the output vault.
                        let input_is_a = if chosen_delta >= 0 {
                            choose_da
                        } else {
                            !choose_da
                        };

                        amount_in_raw = Some(chosen_delta.unsigned_abs());
                        amount_out_raw = Some(other_delta.unsigned_abs());
                        direction = Some(if input_is_a {
                            "a_to_b".to_string()
                        } else {
                            "b_to_a".to_string()
                        });
                        decode_status = "loose_inferred_vault_delta".to_string();
                    } else {
                        decode_status = "ambiguous_vault_delta".to_string();
                    }
                }
            } else {
                decode_status = "missing_token_balances".to_string();
            }
        } else {
            // B2 Meteora: sometimes vault token accounts are present in logs/balances but
            // can't be resolved to `accountIndex` reliably. Fallback:
            // use token-mint + token-vault-owner matching inside pre/postTokenBalances.
            if matches!(protocol, Proto::Meteora) {
                // First fallback: infer vault deltas from SPL inner transfers touching pool vaults.
                let (inner_da, inner_db) = infer_vault_deltas_from_inner_instructions(m, pool_meta);
                if let (Some(da), Some(db)) = (inner_da, inner_db) {
                    va_delta = Some(da);
                    vb_delta = Some(db);
                    if da > 0 && db < 0 {
                        amount_in_raw = Some(da.unsigned_abs());
                        amount_out_raw = Some(db.unsigned_abs());
                        direction = Some("a_to_b".to_string());
                        decode_status = "ok".to_string();
                    } else if db > 0 && da < 0 {
                        amount_in_raw = Some(db.unsigned_abs());
                        amount_out_raw = Some(da.unsigned_abs());
                        direction = Some("b_to_a".to_string());
                        decode_status = "ok".to_string();
                    } else if has_swap_log {
                        let choose_da = da.abs() >= db.abs();
                        let chosen_delta = if choose_da { da } else { db };
                        let other_delta = if choose_da { db } else { da };
                        let input_is_a = chosen_delta >= 0;
                        amount_in_raw = Some(chosen_delta.unsigned_abs());
                        amount_out_raw = Some(other_delta.unsigned_abs());
                        direction = Some(if input_is_a {
                            "a_to_b".to_string()
                        } else {
                            "b_to_a".to_string()
                        });
                        decode_status = "loose_inferred_vault_delta".to_string();
                    } else {
                        decode_status = "ambiguous_vault_delta".to_string();
                    }
                    if debug_meteora {
                        eprintln!(
                            "[meteora-debug] inner-transfer fallback pool={} sig={} da={} db={} status={}",
                            pool, sig, da, db, decode_status
                        );
                    }
                } else if let (Some(owner_a), Some(owner_b)) = (
                    pool_meta.token_vault_owner_a.as_deref(),
                    pool_meta.token_vault_owner_b.as_deref(),
                ) {
                    if debug_meteora {
                        eprintln!(
                            "[meteora-debug] owner+mint fallback pool={} sig={} owner_a={} owner_b={}",
                            pool, sig, owner_a, owner_b
                        );
                    }
                    let pre_a = token_amount_by_mint_owner(
                        m,
                        &pool_meta.token_mint_a,
                        owner_a,
                        "preTokenBalances",
                    );
                    let post_a = token_amount_by_mint_owner(
                        m,
                        &pool_meta.token_mint_a,
                        owner_a,
                        "postTokenBalances",
                    );
                    let pre_b = token_amount_by_mint_owner(
                        m,
                        &pool_meta.token_mint_b,
                        owner_b,
                        "preTokenBalances",
                    );
                    let post_b = token_amount_by_mint_owner(
                        m,
                        &pool_meta.token_mint_b,
                        owner_b,
                        "postTokenBalances",
                    );

                    if let (Some(pa), Some(qa), Some(pb), Some(qb)) = (pre_a, post_a, pre_b, post_b)
                    {
                        let da = qa as i128 - pa as i128;
                        let db = qb as i128 - pb as i128;
                        va_delta = Some(da);
                        vb_delta = Some(db);

                        if da > 0 && db < 0 {
                            amount_in_raw = Some(da as u128);
                            amount_out_raw = Some((-db) as u128);
                            direction = Some("a_to_b".to_string());
                            decode_status = "ok".to_string();
                        } else if db > 0 && da < 0 {
                            amount_in_raw = Some(db as u128);
                            amount_out_raw = Some((-da) as u128);
                            direction = Some("b_to_a".to_string());
                            decode_status = "ok".to_string();
                        } else if da == 0 && db == 0 {
                            decode_status = "no_vault_change".to_string();
                        } else if has_swap_log {
                            // Loose inference, same idea as vault-index based version.
                            let choose_da = da.abs() >= db.abs();
                            let chosen_delta = if choose_da { da } else { db };
                            let other_delta = if choose_da { db } else { da };

                            let input_is_a = if chosen_delta >= 0 {
                                choose_da
                            } else {
                                !choose_da
                            };

                            amount_in_raw = Some(chosen_delta.unsigned_abs());
                            amount_out_raw = Some(other_delta.unsigned_abs());
                            direction = Some(if input_is_a {
                                "a_to_b".to_string()
                            } else {
                                "b_to_a".to_string()
                            });
                            decode_status = "loose_inferred_vault_delta".to_string();
                        } else {
                            decode_status = "ambiguous_vault_delta".to_string();
                        }
                    } else {
                        if debug_meteora {
                            debug_dump_token_balances(m, pool, &sig);
                            eprintln!(
                                "[meteora-debug] owner+mint missing balances pool={} sig={} pre_a={:?} post_a={:?} pre_b={:?} post_b={:?}",
                                pool, sig, pre_a, post_a, pre_b, post_b
                            );
                        }
                        decode_status = "missing_token_balances".to_string();
                    }
                } else {
                    // Without vault owners we can't match balances robustly.
                    // Mint-only fallback: choose the largest mint delta by `accountIndex`
                    // and infer direction from the sign pattern.
                    let mut da = choose_largest_mint_delta(m, &pool_meta.token_mint_a);
                    let mut db = choose_largest_mint_delta(m, &pool_meta.token_mint_b);
                    if da.is_none() || db.is_none() {
                        // Auto-correct mint mapping when snapshot mints differ from what tx reports.
                        let pairs = largest_mint_deltas(m);
                        let pos = pairs.iter().find(|(_, d)| *d > 0);
                        let neg = pairs.iter().find(|(_, d)| *d < 0);
                        if let (Some((mint_in, d_in)), Some((mint_out, d_out))) = (pos, neg) {
                            out_token_mint_a = mint_in.clone();
                            out_token_mint_b = mint_out.clone();
                            da = Some(*d_in);
                            db = Some(*d_out);
                            if debug_meteora {
                                eprintln!(
                                    "[meteora-debug] dynamic-mint-map pool={} sig={} mint_a={} da={} mint_b={} db={}",
                                    pool, sig, out_token_mint_a, d_in, out_token_mint_b, d_out
                                );
                            }
                        }
                    }
                    if debug_meteora {
                        eprintln!(
                            "[meteora-debug] mint-only fallback pool={} sig={} has_owner_a={} has_owner_b={} da={:?} db={:?}",
                            pool,
                            sig,
                            pool_meta.token_vault_owner_a.is_some(),
                            pool_meta.token_vault_owner_b.is_some(),
                            da,
                            db
                        );
                    }
                    if let (Some(da), Some(db)) = (da, db) {
                        va_delta = Some(da);
                        vb_delta = Some(db);
                        if da > 0 && db < 0 {
                            amount_in_raw = Some(da.unsigned_abs());
                            amount_out_raw = Some(db.unsigned_abs());
                            direction = Some("a_to_b".to_string());
                            decode_status = "ok".to_string();
                        } else if db > 0 && da < 0 {
                            amount_in_raw = Some(db.unsigned_abs());
                            amount_out_raw = Some(da.unsigned_abs());
                            direction = Some("b_to_a".to_string());
                            decode_status = "ok".to_string();
                        } else if has_swap_log {
                            let choose_da = da.abs() >= db.abs();
                            let chosen_delta = if choose_da { da } else { db };
                            let other_delta = if choose_da { db } else { da };

                            let input_is_a = chosen_delta >= 0;
                            amount_in_raw = Some(chosen_delta.unsigned_abs());
                            amount_out_raw = Some(other_delta.unsigned_abs());
                            direction = Some(if input_is_a {
                                "a_to_b".to_string()
                            } else {
                                "b_to_a".to_string()
                            });
                            decode_status = "loose_inferred_vault_delta".to_string();
                        } else {
                            decode_status = "ambiguous_vault_delta".to_string();
                        }
                    } else {
                        if debug_meteora {
                            debug_dump_token_balances(m, pool, &sig);
                            eprintln!(
                                "[meteora-debug] final missing_vault_indices pool={} sig={} token_mint_a={} token_mint_b={} has_swap_log={} log_swap_mentions={}",
                                pool,
                                sig,
                                pool_meta.token_mint_a,
                                pool_meta.token_mint_b,
                                has_swap_log,
                                log_swap_mentions
                            );
                        }
                        decode_status = "missing_vault_indices".to_string();
                    }
                }
            } else {
                decode_status = "missing_vault_indices".to_string();
            }
        }
    }

    // B2: Orca Whirlpool emits Anchor `Traded` in `Program data:` — prefer it for fee/sqrt/amounts
    // when vault-delta heuristics are ambiguous.
    if let Some(ref t) = traded_ev {
        fee_amount_raw = Some(t.lp_fee as u128);
        sqrt_price_x64_after = Some(t.post_sqrt_price);
        if amount_in_raw.is_none() {
            amount_in_raw = Some(t.input_amount as u128);
            amount_out_raw = Some(t.output_amount as u128);
            direction = Some(if t.a_to_b {
                "a_to_b".to_string()
            } else {
                "b_to_a".to_string()
            });
        }
        match decode_status.as_str() {
            "ok" => {}
            "missing_meta" | "partial" => {}
            _ if success => {
                decode_status = "ok_traded_event".to_string();
            }
            _ => {}
        }
    }

    // B2 (Raydium): Anchor `SwapEvent` in `Program data:` (same pattern as Orca).
    // Assumes snapshot `token_mint_a` / vault order matches pool token0 (mint0).
    if let Some(ref ev) = raydium_swap_ev {
        sqrt_price_x64_after = Some(ev.sqrt_price_x64);
        tick_after = Some(ev.tick);
        if amount_in_raw.is_none() {
            let (ain, aout) = if ev.zero_for_one {
                (ev.amount0, ev.amount1)
            } else {
                (ev.amount1, ev.amount0)
            };
            amount_in_raw = Some(ain as u128);
            amount_out_raw = Some(aout as u128);
            direction = Some(if ev.zero_for_one {
                "a_to_b".to_string()
            } else {
                "b_to_a".to_string()
            });
        }
        if fee_amount_raw.is_none() {
            fee_amount_raw = Some(ev.transfer_fee0.saturating_add(ev.transfer_fee1) as u128);
        }
        match decode_status.as_str() {
            "ok" => {}
            "missing_meta" | "partial" => {}
            _ if success => {
                decode_status = "ok_swap_event".to_string();
            }
            _ => {}
        }
    }

    // B2 (Meteora DLMM): Anchor `event:Swap` in `Program data:` (IDL name `Swap`; see `meteora_swap_event.rs`).
    // Snapshot `token_mint_a` / `token_mint_b` follow pool token X / Y (same as LB pair reserves).
    if let Some(ref ev) = meteora_swap_ev {
        if amount_in_raw.is_none() {
            amount_in_raw = Some(ev.amount_in as u128);
            amount_out_raw = Some(ev.amount_out as u128);
            direction = Some(if ev.swap_for_y {
                "a_to_b".to_string()
            } else {
                "b_to_a".to_string()
            });
        }
        if fee_amount_raw.is_none() {
            fee_amount_raw = Some(ev.fee.saturating_add(ev.protocol_fee) as u128);
        }
        match decode_status.as_str() {
            "ok" => {}
            "missing_meta" | "partial" => {}
            _ if success => {
                decode_status = "ok_swap_event".to_string();
            }
            _ => {}
        }
    }

    Ok(DecodedSwapTx {
        ts_utc: Utc::now().to_rfc3339(),
        protocol: protocol.dir().to_string(),
        pool_address: pool.to_string(),
        signature: sig.to_string(),
        slot,
        block_time,
        success,
        tx_fee_lamports: tx_fee,
        token_mint_a: out_token_mint_a,
        token_mint_b: out_token_mint_b,
        vault_a_delta_raw: va_delta,
        vault_b_delta_raw: vb_delta,
        amount_in_raw,
        amount_out_raw,
        fee_amount_raw,
        direction,
        tick_after,
        sqrt_price_x64_after,
        log_swap_mentions,
        has_swap_log,
        decode_status,
        source: "rpc:getTransaction(jsonParsed)".to_string(),
        schema_version: 3,
    })
}

async fn decode_one_signature_with_retry(
    rpc: &RpcProvider,
    protocol: Proto,
    pool: &str,
    pool_meta: &PoolMeta,
    sig: &str,
    decode_timeout_secs: u64,
    decode_retries: usize,
    decode_jitter_ms: u64,
) -> Result<DecodedSwapTx> {
    const RETRY_BACKOFF_MS: u64 = 700;

    if decode_jitter_ms > 0 {
        let ms = rand::random::<u64>() % decode_jitter_ms;
        if ms > 0 {
            sleep(Duration::from_millis(ms)).await;
        }
    }

    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 0..=decode_retries {
        let fut = decode_one_signature(rpc, protocol, pool, pool_meta, sig);
        match timeout(Duration::from_secs(decode_timeout_secs), fut).await {
            Ok(Ok(row)) => return Ok(row),
            Ok(Err(e)) => {
                last_err = Some(e);
            }
            Err(_) => {
                last_err = Some(anyhow::anyhow!(
                    "decode timeout after {}s for signature {}",
                    decode_timeout_secs,
                    sig
                ));
            }
        }
        if attempt < decode_retries {
            sleep(Duration::from_millis(
                RETRY_BACKOFF_MS * (attempt as u64 + 1),
            ))
            .await;
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("unknown decode error")))
}

async fn sync_one_pool(
    rpc: &RpcProvider,
    protocol: Proto,
    pool: &str,
    max_signatures: usize,
    max_pages: usize,
) -> Result<(usize, usize)> {
    let pubkey = Pubkey::from_str(pool)?;
    const PAGE_RETRIES: usize = 3;
    const PAGE_BACKOFF_MS: u64 = 600;
    let mut before: Option<Signature> = None;
    let mut rows = Vec::new();

    for page in 0..max_pages.max(1) {
        if rows.len() >= max_signatures {
            break;
        }
        let remaining = max_signatures.saturating_sub(rows.len());
        let page_limit = remaining.min(1_000);
        let mut page_rows = None;
        let mut last_err: Option<anyhow::Error> = None;
        for attempt in 0..=PAGE_RETRIES {
            let cfg = GetConfirmedSignaturesForAddress2Config {
                before,
                until: None,
                limit: Some(page_limit),
                commitment: None,
            };
            match rpc
                .get_signatures_for_address_with_config(&pubkey, cfg)
                .await
            {
                Ok(v) => {
                    page_rows = Some(v);
                    break;
                }
                Err(e) => {
                    last_err = Some(e.into());
                    if attempt < PAGE_RETRIES {
                        sleep(Duration::from_millis(
                            PAGE_BACKOFF_MS * (attempt as u64 + 1),
                        ))
                        .await;
                    }
                }
            }
        }

        let Some(mut batch) = page_rows else {
            return Err(last_err.unwrap_or_else(|| {
                anyhow::anyhow!("failed to fetch signatures page for {}", pool)
            }));
        };
        if batch.is_empty() {
            break;
        }
        before = batch
            .last()
            .and_then(|r| Signature::from_str(&r.signature).ok());
        rows.append(&mut batch);
        if before.is_none() {
            break;
        }
        if page + 1 >= max_pages.max(1) {
            break;
        }
    }

    let mut dir = PathBuf::from("data");
    dir.push("swaps");
    dir.push(protocol.dir());
    dir.push(pool);
    std::fs::create_dir_all(&dir)?;
    let mut path = dir;
    path.push("swaps.jsonl");

    let mut known = existing_sigs(&path);
    let mut appended = 0usize;
    let mut seen = 0usize;

    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;

    for r in rows {
        seen += 1;
        if known.contains(&r.signature) {
            continue;
        }
        let obj = RawChainTx {
            ts_utc: Utc::now().to_rfc3339(),
            protocol: protocol.dir().to_string(),
            pool_address: pool.to_string(),
            signature: r.signature.clone(),
            slot: r.slot,
            block_time: r.block_time,
            confirmation_status: r.confirmation_status.map(|s| format!("{s:?}")),
            err: r.err.map(|e| format!("{e:?}")),
            source: "rpc:getSignaturesForAddress".to_string(),
            schema_version: 1,
        };
        let line = serde_json::to_string(&obj)? + "\n";
        use std::io::Write;
        out.write_all(line.as_bytes())?;
        known.insert(obj.signature);
        appended += 1;
    }

    Ok((seen, appended))
}

fn ws_url_from_rpc_url(rpc_url: &str) -> String {
    if let Some(rest) = rpc_url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = rpc_url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        rpc_url.to_string()
    }
}

pub fn subscribe_mentions_to_raw(
    protocol: &str,
    pool_address: &str,
    mentions: &Option<String>,
    mentions_preset: &Option<String>,
    max_events: usize,
    idle_timeout_secs: u64,
) -> Result<()> {
    let proto = Proto::parse(protocol).ok_or_else(|| {
        anyhow::anyhow!("invalid protocol '{protocol}', expected: orca|raydium|meteora")
    })?;
    let _pool_pubkey = Pubkey::from_str(pool_address)
        .map_err(|e| anyhow::anyhow!("invalid --pool-address '{}': {e}", pool_address))?;
    let resolved_mentions = if let Some(v) = mentions.as_ref() {
        v.clone()
    } else if let Some(preset) = mentions_preset.as_ref() {
        match preset.trim().to_ascii_lowercase().as_str() {
            "orca" => "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc".to_string(),
            "raydium" => "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK".to_string(),
            "meteora" => "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo".to_string(),
            _ => {
                return Err(anyhow::anyhow!(
                    "invalid --mentions-preset '{}', expected: orca|raydium|meteora",
                    preset
                ));
            }
        }
    } else {
        return Err(anyhow::anyhow!(
            "missing mentions filter: provide --mentions <PUBKEY> or --mentions-preset <orca|raydium|meteora>"
        ));
    };
    let mention_pubkey = Pubkey::from_str(&resolved_mentions)
        .map_err(|e| anyhow::anyhow!("invalid mentions pubkey '{}': {e}", resolved_mentions))?;

    let rpc_cfg = RpcConfig::default();
    let ws_url = ws_url_from_rpc_url(&rpc_cfg.primary_url);
    let logs_cfg = RpcTransactionLogsConfig { commitment: None };
    let filter = RpcTransactionLogsFilter::Mentions(vec![mention_pubkey.to_string()]);
    let (mut client, receiver) = PubsubClient::logs_subscribe(&ws_url, filter, logs_cfg)?;

    let mut dir = PathBuf::from("data");
    dir.push("swaps");
    dir.push(proto.dir());
    dir.push(pool_address);
    std::fs::create_dir_all(&dir)?;
    let mut path = dir;
    path.push("swaps.jsonl");
    let mut known = existing_sigs(&path);
    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;

    let mut seen = 0usize;
    let mut appended = 0usize;
    println!(
        "🔌 logs subscribe: protocol={} pool={} mentions={} ws={} max_events={} idle_timeout={}s",
        proto.dir(),
        pool_address,
        resolved_mentions,
        ws_url,
        max_events,
        idle_timeout_secs
    );
    println!("ℹ️ streaming raw signatures into {}", path.display());

    while seen < max_events {
        let msg: RpcResponse<RpcLogsResponse> =
            match receiver.recv_timeout(Duration::from_secs(idle_timeout_secs.max(1))) {
                Ok(v) => v,
                Err(e) => {
                    // Pubsub uses crossbeam receiver; timeout error type is not from std::sync::mpsc.
                    // Keep matching by semantic message to avoid version-specific type imports.
                    if e.to_string().to_ascii_lowercase().contains("timed out") {
                        println!(
                            "⏹️ stop subscribe after {}s idle (seen={} appended={})",
                            idle_timeout_secs, seen, appended
                        );
                        break;
                    }
                    return Err(anyhow::anyhow!("logs subscribe recv failed: {e}"));
                }
            };

        seen += 1;
        let sig = msg.value.signature.clone();
        if known.contains(&sig) {
            continue;
        }
        let obj = RawChainTx {
            ts_utc: Utc::now().to_rfc3339(),
            protocol: proto.dir().to_string(),
            pool_address: pool_address.to_string(),
            signature: sig.clone(),
            slot: msg.context.slot,
            block_time: None,
            confirmation_status: None,
            err: msg.value.err.map(|e| format!("{e:?}")),
            source: "rpc:logsSubscribe(mentions)".to_string(),
            schema_version: 1,
        };
        let line = serde_json::to_string(&obj)? + "\n";
        use std::io::Write;
        out.write_all(line.as_bytes())?;
        known.insert(sig);
        appended += 1;
    }

    let _ = client.shutdown();
    println!(
        "📌 logs subscribe summary: seen={} appended={} source=logsSubscribe(mentions)",
        seen, appended
    );
    Ok(())
}

pub async fn sync_curated_all_robust(
    limit: Option<usize>,
    max_signatures: usize,
    max_pages: usize,
) -> Result<()> {
    let rpc = RpcProvider::mainnet();
    let plans: Vec<(Proto, Vec<String>)> = vec![
        (Proto::Orca, parse_curated_pool_addrs(Proto::Orca)?),
        (Proto::Raydium, parse_curated_pool_addrs(Proto::Raydium)?),
        (Proto::Meteora, parse_curated_pool_addrs(Proto::Meteora)?),
    ];

    let mut total_seen = 0usize;
    let mut total_new = 0usize;
    for (proto, mut pools) in plans {
        if let Some(l) = limit {
            pools.truncate(l);
        }
        for pool in pools {
            let (seen, new_rows) =
                sync_one_pool(&rpc, proto, &pool, max_signatures, max_pages).await?;
            total_seen += seen;
            total_new += new_rows;
            println!(
                "✅ swaps sync {} {}: seen={} appended={} (pages<={})",
                proto.dir(),
                pool,
                seen,
                new_rows,
                max_pages.max(1)
            );
        }
    }

    println!(
        "📌 swaps sync summary: seen={} appended={} (source=getSignaturesForAddress paged)",
        total_seen, total_new
    );
    Ok(())
}

pub async fn enrich_curated_all(
    limit: Option<usize>,
    max_decode: usize,
    decode_timeout_secs: u64,
    decode_retries: usize,
    decode_concurrency: usize,
    decode_jitter_ms: u64,
    refresh_decoded: bool,
) -> Result<()> {
    // Env overrides CLI when set (e.g. CI / shared runners). Bounded to avoid unbounded fan-out.
    let decode_inflight = std::env::var("CLMM_ENRICH_DECODE_INFLIGHT")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .map(|n| n.clamp(1, 32))
        .unwrap_or_else(|| decode_concurrency.max(1).min(32));
    let decode_jitter_ms = std::env::var("CLMM_ENRICH_DECODE_JITTER_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(decode_jitter_ms);

    // Use a tighter RPC timeout for swap decoding so the command remains responsive.
    // The outer `decode_timeout_secs` still guards the full decode path; this mainly
    // prevents individual RPC endpoints from hanging too long before failover.
    let rpc_timeout = Duration::from_secs(decode_timeout_secs.clamp(4, 15));
    let rpc_provider = RpcProvider::new(
        RpcConfig::default()
            .with_timeout(rpc_timeout)
            .with_max_retries(decode_retries.min(3) as u32),
    );
    let rpc = Arc::new(rpc_provider);
    let plans: Vec<(Proto, Vec<String>)> = vec![
        (Proto::Orca, parse_curated_pool_addrs(Proto::Orca)?),
        (Proto::Raydium, parse_curated_pool_addrs(Proto::Raydium)?),
        (Proto::Meteora, parse_curated_pool_addrs(Proto::Meteora)?),
    ];
    let mut decoded_total = 0usize;
    let mut skipped_total = 0usize;

    for (proto, mut pools) in plans {
        if let Some(l) = limit {
            pools.truncate(l);
        }
        for pool in pools {
            let Some(mut meta) = latest_pool_meta(proto, &pool) else {
                println!(
                    "⚠️ skip enrich {} {}: missing snapshot meta",
                    proto.dir(),
                    pool
                );
                continue;
            };

            // For Meteora, decode fallback may need token-vault owners. If snapshots
            // couldn't fetch SPL token accounts (transient RPC issues), fill owners
            // once per pool here.
            if matches!(proto, Proto::Meteora) {
                let _ = fill_meteora_token_vault_owners(rpc.as_ref(), &pool, &mut meta).await;
            }
            let mut raw_path = PathBuf::from("data");
            raw_path.push("swaps");
            raw_path.push(proto.dir());
            raw_path.push(&pool);
            raw_path.push("swaps.jsonl");
            if !raw_path.exists() {
                println!(
                    "⚠️ skip enrich {} {}: missing raw swaps file",
                    proto.dir(),
                    pool
                );
                continue;
            }
            let mut out_path = raw_path.clone();
            out_path.set_file_name("decoded_swaps.jsonl");
            if refresh_decoded {
                let _ = std::fs::remove_file(&out_path);
            }
            let known = existing_sigs(&out_path);
            let txt = std::fs::read_to_string(&raw_path)?;
            let mut to_decode: Vec<String> = Vec::new();
            // Focus decode on recent history to avoid archive-missing transactions.
            // The raw swaps file includes `block_time` from getSignaturesForAddress.
            let min_block_time = Utc::now().timestamp().saturating_sub(72 * 3600);
            // Decode newest first: raw swaps file is append-only, so the newest signatures
            // are near the end. This makes "last 24h/48h" audits meaningful quickly.
            let lines: Vec<&str> = txt.lines().filter(|l| !l.trim().is_empty()).collect();
            for line in lines.iter().rev() {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                    continue;
                };
                if let Some(bt) = v.get("block_time").and_then(|x| x.as_i64()) {
                    if bt > 0 && bt < min_block_time {
                        continue;
                    }
                }
                let Some(sig) = v.get("signature").and_then(|x| x.as_str()) else {
                    continue;
                };
                if known.contains(sig) {
                    continue;
                }
                to_decode.push(sig.to_string());
                if to_decode.len() >= max_decode {
                    break;
                }
            }

            let out = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&out_path)?;
            let mut ok = 0usize;
            let mut skipped = 0usize;
            let mut retried = 0usize;
            println!(
                "🔎 swaps enrich {} {}: decoding {} newest signatures (timeout={}s retries={} inflight={} jitter_ms={})",
                proto.dir(),
                pool,
                to_decode.len(),
                decode_timeout_secs,
                decode_retries,
                decode_inflight,
                decode_jitter_ms
            );

            let meta_arc = Arc::new(meta);
            let pool_owned = pool.clone();
            let out = Arc::new(Mutex::new(out));
            let mut stream = stream::iter(to_decode.into_iter().map(|sig| {
                let rpc = rpc.clone();
                let meta_arc = meta_arc.clone();
                let pool_owned = pool_owned.clone();
                async move {
                    let res = decode_one_signature_with_retry(
                        rpc.as_ref(),
                        proto,
                        &pool_owned,
                        meta_arc.as_ref(),
                        &sig,
                        decode_timeout_secs,
                        decode_retries,
                        decode_jitter_ms,
                    )
                    .await;
                    (sig, res)
                }
            }))
            .buffer_unordered(decode_inflight);

            let mut processed = 0usize;
            while let Some((sig, decode_res)) = stream.next().await {
                processed += 1;
                match decode_res {
                    Ok(row) => {
                        let line = serde_json::to_string(&row)? + "\n";
                        use std::io::Write;
                        let mut g = out.lock().await;
                        g.write_all(line.as_bytes())?;
                        ok += 1;
                    }
                    Err(e) => {
                        if e.to_string().contains("timeout") {
                            retried += 1;
                        }
                        skipped += 1;
                        if skipped <= 3 {
                            println!(
                                "⚠️ decode failed {} {} sig={} err={}",
                                proto.dir(),
                                pool,
                                sig,
                                e
                            );
                        }
                    }
                }
                if processed % 10 == 0 {
                    println!(
                        "… progress {} {}: decoded_ok={} skipped={}",
                        proto.dir(),
                        pool,
                        ok,
                        skipped
                    );
                }
            }
            decoded_total += ok;
            skipped_total += skipped;
            println!(
                "✅ swaps enrich {} {}: decoded={} skipped={} timeouts/retry-fail={} out={}",
                proto.dir(),
                pool,
                ok,
                skipped,
                retried,
                out_path.display()
            );
        }
    }

    println!(
        "📌 swaps enrich summary: decoded={} skipped={}",
        decoded_total, skipped_total
    );
    Ok(())
}

fn count_jsonl_rows(path: &std::path::Path) -> usize {
    std::fs::read_to_string(path)
        .ok()
        .map(|txt| txt.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

fn load_decode_quality(path: &std::path::Path) -> DecodeQuality {
    let mut q = DecodeQuality::default();
    let Ok(txt) = std::fs::read_to_string(path) else {
        return q;
    };
    for line in txt.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        q.decoded_rows += 1;
        if matches!(
            v.get("decode_status").and_then(|x| x.as_str()),
            Some("ok") | Some("ok_traded_event") | Some("ok_swap_event")
        ) {
            q.ok_rows += 1;
        }
        if let Some(s) = v.get("decode_status").and_then(|x| x.as_str()) {
            *q.status_counts.entry(s.to_string()).or_insert(0) += 1;
        }
        if let Some(bt) = v.get("block_time").and_then(|x| x.as_i64()) {
            q.latest_block_time = Some(q.latest_block_time.map(|x| x.max(bt)).unwrap_or(bt));
        }
    }
    q
}

pub fn decode_audit_curated_all(limit: Option<usize>, save_report: bool) -> Result<()> {
    let plans: Vec<(Proto, Vec<String>)> = vec![
        (Proto::Orca, parse_curated_pool_addrs(Proto::Orca)?),
        (Proto::Raydium, parse_curated_pool_addrs(Proto::Raydium)?),
        (Proto::Meteora, parse_curated_pool_addrs(Proto::Meteora)?),
    ];
    let mut rows: Vec<DecodeAuditRow> = Vec::new();
    let mut total_raw = 0usize;
    let mut total_decoded = 0usize;
    let mut total_ok = 0usize;
    for (proto, mut pools) in plans {
        if let Some(l) = limit {
            pools.truncate(l);
        }
        for pool in pools {
            let root = std::path::Path::new("data")
                .join("swaps")
                .join(proto.dir())
                .join(&pool);
            let raw = root.join("swaps.jsonl");
            let dec = root.join("decoded_swaps.jsonl");
            let raw_rows = count_jsonl_rows(&raw);
            let mut quality = load_decode_quality(&dec);
            quality.raw_rows = raw_rows;
            total_raw += quality.raw_rows;
            total_decoded += quality.decoded_rows;
            total_ok += quality.ok_rows;
            let ok_pct = if quality.decoded_rows == 0 {
                0.0
            } else {
                (quality.ok_rows as f64) * 100.0 / (quality.decoded_rows as f64)
            };
            rows.push(DecodeAuditRow {
                protocol: proto.dir().to_string(),
                pool_address: pool,
                raw_rows: quality.raw_rows,
                decoded_rows: quality.decoded_rows,
                ok_rows: quality.ok_rows,
                ok_pct,
                latest_block_time: quality.latest_block_time,
                status_counts: quality.status_counts,
            });
        }
    }
    rows.sort_by(|a, b| {
        a.ok_pct
            .partial_cmp(&b.ok_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for r in &rows {
        println!(
            "📈 decode audit {} {} raw={} decoded={} ok={} ({:.1}%) latest_block_time={:?} statuses={:?}",
            r.protocol,
            r.pool_address,
            r.raw_rows,
            r.decoded_rows,
            r.ok_rows,
            r.ok_pct,
            r.latest_block_time,
            r.status_counts
        );
    }
    let total_ok_pct = if total_decoded == 0 {
        0.0
    } else {
        (total_ok as f64) * 100.0 / (total_decoded as f64)
    };
    let mut global_status: BTreeMap<String, usize> = BTreeMap::new();
    for r in &rows {
        for (k, c) in &r.status_counts {
            *global_status.entry(k.clone()).or_insert(0) += c;
        }
    }
    println!(
        "📌 decode audit summary: pools={} raw={} decoded={} ok={} ({:.1}%)",
        rows.len(),
        total_raw,
        total_decoded,
        total_ok,
        total_ok_pct
    );
    if !global_status.is_empty() && total_decoded > 0 {
        print!("📌 decode_status global: ");
        let mut first = true;
        for (st, n) in &global_status {
            let pct = (*n as f64) * 100.0 / (total_decoded as f64);
            if !first {
                print!(", ");
            }
            first = false;
            print!("{}={} ({:.1}%)", st, n, pct);
        }
        println!();
    }
    let global_status_pct: BTreeMap<String, f64> = global_status
        .iter()
        .filter_map(|(k, n)| {
            if total_decoded == 0 {
                None
            } else {
                Some((k.clone(), (*n as f64) * 100.0 / (total_decoded as f64)))
            }
        })
        .collect();
    if save_report {
        let ts = Utc::now().format("%Y%m%d_%H%M%S");
        let out_dir = std::path::Path::new("data").join("reports");
        std::fs::create_dir_all(&out_dir)?;
        let out = out_dir.join(format!("decode_audit_{}.json", ts));
        let body = serde_json::json!({
            "ts_utc": Utc::now().to_rfc3339(),
            "total_raw_rows": total_raw,
            "total_decoded_rows": total_decoded,
            "total_ok_rows": total_ok,
            "total_ok_pct": total_ok_pct,
            "global_status_counts": global_status,
            "global_status_pct": global_status_pct,
            "rows": rows,
        });
        std::fs::write(&out, serde_json::to_string_pretty(&body)?)?;
        println!("📝 decode audit report saved: {}", out.display());
    }
    Ok(())
}

fn age_minutes(path: &std::path::Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let now = SystemTime::now();
    let d = now.duration_since(modified).ok()?;
    Some((d.as_secs() / 60) as i64)
}

pub fn health_check_curated_all(
    max_age_minutes: i64,
    min_decode_ok_pct: f64,
    fail_on_alert: bool,
) -> Result<()> {
    let plans: Vec<(Proto, Vec<String>)> = vec![
        (Proto::Orca, parse_curated_pool_addrs(Proto::Orca)?),
        (Proto::Raydium, parse_curated_pool_addrs(Proto::Raydium)?),
        (Proto::Meteora, parse_curated_pool_addrs(Proto::Meteora)?),
    ];
    let mut alerts: Vec<String> = Vec::new();
    for (proto, pools) in plans {
        for pool in pools {
            let base = std::path::Path::new("data")
                .join("swaps")
                .join(proto.dir())
                .join(&pool);
            let raw = base.join("swaps.jsonl");
            let dec = base.join("decoded_swaps.jsonl");
            let snap = std::path::Path::new("data")
                .join("pool-snapshots")
                .join(proto.dir())
                .join(&pool)
                .join("snapshots.jsonl");
            let raw_age = age_minutes(&raw);
            let dec_age = age_minutes(&dec);
            let snap_age = age_minutes(&snap);
            let quality = load_decode_quality(&dec);
            let ok_pct = if quality.decoded_rows == 0 {
                0.0
            } else {
                (quality.ok_rows as f64) * 100.0 / (quality.decoded_rows as f64)
            };
            let mut row_alerts: Vec<String> = Vec::new();
            if raw_age.map(|x| x > max_age_minutes).unwrap_or(true) {
                row_alerts.push(format!("raw_stale({:?}m)", raw_age));
            }
            if dec_age.map(|x| x > max_age_minutes).unwrap_or(true) {
                row_alerts.push(format!("decoded_stale({:?}m)", dec_age));
            }
            if snap_age.map(|x| x > max_age_minutes).unwrap_or(true) {
                row_alerts.push(format!("snapshot_stale({:?}m)", snap_age));
            }
            if quality.decoded_rows > 0 && ok_pct < min_decode_ok_pct {
                row_alerts.push(format!("decode_ok_pct_low({:.1}%)", ok_pct));
            }
            if row_alerts.is_empty() {
                println!(
                    "✅ health {} {} raw_age={:?}m decoded_age={:?}m snapshot_age={:?}m ok_pct={:.1}%",
                    proto.dir(),
                    pool,
                    raw_age,
                    dec_age,
                    snap_age,
                    ok_pct
                );
            } else {
                let msg = format!("{} {} => {}", proto.dir(), pool, row_alerts.join(", "));
                println!("⚠️ health alert {}", msg);
                alerts.push(msg);
            }
        }
    }
    println!("📌 health summary: alerts={}", alerts.len());
    if !alerts.is_empty() {
        let ts = Utc::now().format("%Y%m%d_%H%M%S");
        let out_dir = std::path::Path::new("data").join("reports");
        std::fs::create_dir_all(&out_dir)?;
        let out = out_dir.join(format!("health_alerts_{}.json", ts));
        let body = serde_json::json!({
            "ts_utc": Utc::now().to_rfc3339(),
            "max_age_minutes": max_age_minutes,
            "min_decode_ok_pct": min_decode_ok_pct,
            "alerts": alerts,
        });
        std::fs::write(&out, serde_json::to_string_pretty(&body)?)?;
        println!("📝 health report saved: {}", out.display());
        if fail_on_alert {
            anyhow::bail!(
                "health check failed with {} alerts",
                body["alerts"].as_array().map(|a| a.len()).unwrap_or(0)
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_from_tx_root_reads_flattened_meta() {
        let j = serde_json::json!({
            "slot": 1,
            "transaction": { "signatures": [], "message": { "accountKeys": [] } },
            "meta": { "err": null, "fee": 5000 }
        });
        let m = meta_from_tx_root(&j);
        assert!(m.is_some());
        assert!(m.unwrap().get("fee").and_then(|x| x.as_u64()) == Some(5000));
    }

    #[test]
    fn full_account_keys_appends_loaded_addresses() {
        let j = serde_json::json!({
            "transaction": {
                "signatures": ["sig"],
                "message": {
                    "accountKeys": [
                        {"pubkey": "Static111", "writable": true, "signer": true}
                    ]
                }
            },
            "meta": {
                "loadedAddresses": {
                    "writable": ["WritableLoaded"],
                    "readonly": ["ReadonlyLoaded"]
                }
            }
        });
        let keys = full_account_keys_for_tx(&j);
        assert_eq!(keys.len(), 3);
        assert!(account_index_of(&keys, "WritableLoaded").is_some());
        assert!(account_index_of(&keys, "ReadonlyLoaded").is_some());
    }
}
