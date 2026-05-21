//! Logical SESSION portfolio: lifecycle deltas, GL/PSLR read, caps for executor reopen.

use crate::repositories::Database;
use num_traits::ToPrimitive;
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::Row;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
pub const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionCapsSource {
    Gl,
    PslrFallback,
    ReconciledMin,
    LifecycleFile,
    Empty,
}

/// Per-mint spendable cap for one rebalance session (logical sub-wallet).
#[derive(Debug, Clone)]
pub struct SessionMintCaps {
    pub session_id: String,
    pub caps_by_mint: BTreeMap<String, u64>,
    pub source: SessionCapsSource,
}

impl SessionMintCaps {
    pub fn empty(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            caps_by_mint: BTreeMap::new(),
            source: SessionCapsSource::Empty,
        }
    }

    pub fn cap_u64_for_mint(&self, mint: &str) -> u64 {
        self.caps_by_mint
            .get(mint.trim())
            .copied()
            .unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.caps_by_mint.is_empty() || self.caps_by_mint.values().all(|&v| v == 0)
    }
}

#[derive(Debug, Clone)]
pub struct SessionBalanceMint {
    pub mint: String,
    pub amount_raw: String,
}

/// Cycle-start reference: first **open** in the session (capital deployed into LP).
#[derive(Debug, Clone)]
pub struct SessionOpenStartSnapshot {
    pub ts_utc: Option<String>,
    pub signature: String,
    pub position_pubkey: Option<String>,
    pub event: String,
    pub deployed_balances: Vec<SessionBalanceMint>,
    pub value_usd: Option<f64>,
    pub value_usd_source: String,
    pub pre_open_balances: Vec<SessionBalanceMint>,
    pub pre_open_value_usd: Option<f64>,
    pub price_by_mint: BTreeMap<String, f64>,
    /// `details` | `pool_address` | `incomplete` — how open-row pool mints were resolved.
    pub mint_resolution: String,
}

#[derive(Debug, Clone)]
pub struct SessionMetricsResolved {
    pub open_start: Option<SessionOpenStartSnapshot>,
    pub current_value_usd: Option<f64>,
    pub delta_vs_pre_open_usd: Option<f64>,
    /// False when any pre-open close/open row lacked resolvable pool mints (legacy rows).
    pub metrics_trusted: bool,
}

pub fn session_account_code(session_id: &str) -> String {
    format!("SESSION:{session_id}")
}

/// Idempotent GL posting key. Must fit `wallet_gl_posting.event_id` (VARCHAR 128 after migration 012).
pub fn lifecycle_posting_event_id(signature: &str) -> String {
    format!("lifecycle:{}", signature.trim())
}

/// Max length for `lifecycle:{signature}` (Solana sig ~88 + prefix).
pub const LIFECYCLE_POSTING_EVENT_ID_MAX_LEN: usize = 128;

pub fn parse_raw_i128(s: &str) -> Option<i128> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    t.parse::<i128>().ok()
}

pub fn format_raw_i128(v: i128) -> String {
    v.to_string()
}

fn raw_i128_to_spend_cap(v: i128) -> u64 {
    if v <= 0 {
        0
    } else {
        v.min(u64::MAX as i128) as u64
    }
}

fn caps_map_from_balance_rows(rows: &[(String, String)]) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for (mint, raw) in rows {
        let Some(v) = parse_raw_i128(raw) else {
            continue;
        };
        let cap = raw_i128_to_spend_cap(v);
        if cap > 0 {
            out.insert(mint.clone(), cap);
        }
    }
    out
}

fn merge_min_caps(gl: &BTreeMap<String, u64>, pslr: &BTreeMap<String, u64>) -> BTreeMap<String, u64> {
    let mut all: BTreeMap<String, ()> = BTreeMap::new();
    for m in gl.keys() {
        all.insert(m.clone(), ());
    }
    for m in pslr.keys() {
        all.insert(m.clone(), ());
    }
    let mut out = BTreeMap::new();
    for mint in all.keys() {
        let g = gl.get(mint).copied().unwrap_or(0);
        let p = pslr.get(mint).copied().unwrap_or(0);
        let cap = match (g, p) {
            (0, 0) => 0,
            (0, p) => p,
            (g, 0) => g,
            (g, p) => g.min(p),
        };
        if cap > 0 {
            out.insert(mint.clone(), cap);
        }
    }
    out
}

fn dec_from_json(v: &Value) -> Option<Decimal> {
    match v {
        Value::String(s) => Decimal::from_str(s.trim()).ok(),
        Value::Number(n) => n
            .as_f64()
            .and_then(Decimal::from_f64_retain)
            .or_else(|| n.as_u64().map(Decimal::from)),
        _ => None,
    }
}

fn parse_u64_json(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_i64().and_then(|x| (x >= 0).then_some(x as u64)))
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

fn default_mint_decimals(mint: &str) -> u8 {
    if mint == WSOL_MINT {
        9
    } else if mint == USDC_MINT {
        6
    } else {
        9
    }
}

fn ui_decimal_to_raw_i128(ui: Decimal, decimals: u8) -> Option<i128> {
    if ui == Decimal::ZERO {
        return Some(0);
    }
    let scale = Decimal::from(10i128.pow(u32::from(decimals)));
    let raw = (ui * scale).trunc();
    raw.to_i128()
}

/// How pool leg mints were resolved for SESSION postings (UI / diagnostics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolMintResolveSource {
    /// `details.token_mint_a` / `token_mint_b`.
    Details,
    /// `pool_address` on the lifecycle row → curated pool table.
    PoolAddress,
    /// Unresolved — principal legs for close/open/collect are skipped (no fee_payer key-order guess).
    Unresolved,
}

impl PoolMintResolveSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Details => "details",
            Self::PoolAddress => "pool_address",
            Self::Unresolved => "incomplete",
        }
    }
}

/// Curated pools — keep in sync with `migrations/009_wallet_gl_curated_tokens_and_pools.sql`.
fn known_pool_leg_mints(pool_address: &str) -> Option<(&'static str, &'static str)> {
    match pool_address.trim() {
        "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE" => Some((WSOL_MINT, USDC_MINT)),
        "HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF" => {
            Some((WSOL_MINT, "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs"))
        }
        "HxA6SKW5qA4o12fjVgTpXdq2YnZ5Zv1s7SB4FFomsyLM" => {
            Some(("cbbtcf3aa214zXHbiAZQwf4122FBYbraNdFqgw4iMij", USDC_MINT))
        }
        "4v8ufj8Hj7UvFgtofQJAtzUud5xomwZfEqfCTHZ4wM72" => Some((
            "cbbtcf3aa214zXHbiAZQwf4122FBYbraNdFqgw4iMij",
            "3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh",
        )),
        "3nMFwZXwY1s1M5s8vYAHqd4wGs4iSxXE4LRoUMMYqEgF" => {
            Some((WSOL_MINT, "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB"))
        }
        "HTvjzsfX3yU6BUodCjZ5vZkUrAxMDTrBs3CJaq43ashR"
        | "5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6"
        | "BGm1tav58oGcsQJehL9WXBFXF7D27vZsKefj4xJKD5Y" => Some((WSOL_MINT, USDC_MINT)),
        _ => None,
    }
}

fn pool_address_from_row(v: &Value, details: Option<&serde_json::Map<String, Value>>) -> Option<String> {
    v.get("pool_address")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            details?
                .get("pool")
                .and_then(|x| x.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
        })
}

/// Resolve Whirlpool pool leg mints. Never infer leg A/B from `fee_payer_token_deltas` key order
/// (that mis-maps SOL lamports onto USDC when `close_amount_*_raw` is present).
pub fn pool_mints_from_lifecycle_row(
    v: &Value,
    details: Option<&serde_json::Map<String, Value>>,
) -> (Option<String>, Option<String>, PoolMintResolveSource) {
    let mint_a = details
        .and_then(|d| d.get("token_mint_a"))
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    let mint_b = details
        .and_then(|d| d.get("token_mint_b"))
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    if mint_a.is_some() && mint_b.is_some() {
        return (mint_a, mint_b, PoolMintResolveSource::Details);
    }
    if let Some(pool) = pool_address_from_row(v, details)
        && let Some((ma, mb)) = known_pool_leg_mints(&pool)
    {
        return (
            mint_a.or_else(|| Some(ma.to_string())),
            mint_b.or_else(|| Some(mb.to_string())),
            PoolMintResolveSource::PoolAddress,
        );
    }
    (mint_a, mint_b, PoolMintResolveSource::Unresolved)
}

fn push_raw_delta(out: &mut Vec<(String, i128)>, mint: &str, delta: i128) {
    if mint.trim().is_empty() || delta == 0 {
        return;
    }
    out.push((mint.trim().to_string(), delta));
}

fn push_positive_u64(out: &mut Vec<(String, i128)>, mint: &str, amount: u64) {
    if amount > 0 {
        push_raw_delta(out, mint, i128::from(amount));
    }
}

fn is_lifecycle_close_event(ev: &str) -> bool {
    matches!(ev, "bot_close_position" | "position_close")
}

pub fn is_lifecycle_open_event(ev: &str) -> bool {
    matches!(
        ev,
        "bot_open_position" | "bot_open_position_full_range" | "position_open"
    )
}

fn json_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn i128_map_to_balance_mints(map: &BTreeMap<String, i128>) -> Vec<SessionBalanceMint> {
    map.iter()
        .filter(|(_, v)| **v != 0)
        .map(|(mint, v)| SessionBalanceMint {
            mint: mint.clone(),
            amount_raw: format_raw_i128(*v),
        })
        .collect()
}

fn raw_to_ui_f64(raw: &str, decimals: u8) -> Option<f64> {
    let t = raw.trim();
    if t.is_empty() {
        return Some(0.0);
    }
    let n = t.parse::<i128>().ok()?;
    let neg = n < 0;
    let abs = if neg { -n } else { n };
    let base = 10f64.powi(i32::from(decimals));
    let ui = (abs as f64) / base;
    Some(if neg { -ui } else { ui })
}

fn value_usd_for_balance_mints(
    balances: &[SessionBalanceMint],
    price_by_mint: &BTreeMap<String, f64>,
) -> Option<f64> {
    let mut total = 0.0f64;
    let mut priced_any = false;
    for b in balances {
        let Some(price) = price_by_mint.get(b.mint.trim()) else {
            continue;
        };
        if !price.is_finite() {
            continue;
        }
        let Some(ui) = raw_to_ui_f64(&b.amount_raw, default_mint_decimals(&b.mint)) else {
            continue;
        };
        total += ui * price;
        priced_any = true;
    }
    priced_any.then_some(total)
}

fn open_usd_from_details(details: &serde_json::Map<String, Value>) -> (Option<f64>, &'static str) {
    for (key, src) in [
        ("open_quote_estimated_value_usd", "open_quote_estimated_value_usd"),
        ("open_target_usd", "open_target_usd"),
        ("open_prev_end_value_usd", "open_prev_end_value_usd"),
    ] {
        let Some(v) = details.get(key) else {
            continue;
        };
        let Some(f) = json_f64(v).filter(|x| x.is_finite() && *x > 0.0) else {
            continue;
        };
        return (Some(f), src);
    }
    (None, "unknown")
}

fn deployed_usd_from_open_details(
    details: &serde_json::Map<String, Value>,
    mint_a: Option<&str>,
    mint_b: Option<&str>,
    amount_a_raw: u64,
    amount_b_raw: u64,
) -> (Option<f64>, String) {
    if let (Some(v), src) = open_usd_from_details(details) {
        return (Some(v), src.to_string());
    }
    let pa = details.get("event_price_a_usd").and_then(json_f64);
    let pb = details.get("event_price_b_usd").and_then(json_f64);
    let (Some(pa), Some(pb)) = (pa, pb) else {
        return (None, "unknown".to_string());
    };
    if !(pa.is_finite() && pb.is_finite()) {
        return (None, "unknown".to_string());
    }
    let mut total = 0.0f64;
    if let Some(ma) = mint_a.filter(|s| !s.is_empty()) {
        total += (amount_a_raw as f64 / 10f64.powi(i32::from(default_mint_decimals(ma)))) * pa;
    }
    if let Some(mb) = mint_b.filter(|s| !s.is_empty()) {
        total += (amount_b_raw as f64 / 10f64.powi(i32::from(default_mint_decimals(mb)))) * pb;
    }
    if total > 0.0 && total.is_finite() {
        (Some(total), "computed_event_prices".to_string())
    } else {
        (None, "unknown".to_string())
    }
}

fn price_map_from_open_details(
    details: &serde_json::Map<String, Value>,
    mint_a: Option<&str>,
    mint_b: Option<&str>,
) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    if let (Some(ma), Some(pa)) = (
        mint_a.filter(|s| !s.is_empty()),
        details.get("event_price_a_usd").and_then(json_f64),
    ) && pa.is_finite()
    {
        out.insert(ma.to_string(), pa);
    }
    if let (Some(mb), Some(pb)) = (
        mint_b.filter(|s| !s.is_empty()),
        details.get("event_price_b_usd").and_then(json_f64),
    ) && pb.is_finite()
    {
        out.insert(mb.to_string(), pb);
    }
    out
}

fn build_open_start_from_row(
    v: &Value,
    pre_open: &BTreeMap<String, i128>,
) -> Option<SessionOpenStartSnapshot> {
    let event = v.get("event").and_then(|x| x.as_str()).map(str::trim)?;
    if !is_lifecycle_open_event(event) {
        return None;
    }
    let signature = v
        .get("signature")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    let details = v.get("details").and_then(|d| d.as_object());
    let token_deltas = v.get("fee_payer_token_deltas").and_then(|d| d.as_object());
    let (mint_a, mint_b, mint_resolution) = pool_mints_from_lifecycle_row(v, details);

    let mut deployed: Vec<(String, i128)> = Vec::new();
    if let (Some(ma), Some(mb)) = (&mint_a, &mint_b) {
        let open_a = details
            .and_then(|d| d.get("open_amount_a_raw"))
            .and_then(parse_u64_json);
        let open_b = details
            .and_then(|d| d.get("open_amount_b_raw"))
            .and_then(parse_u64_json);
        if let Some(a) = open_a.filter(|&x| x > 0) {
            push_raw_delta(&mut deployed, ma, i128::from(a));
        }
        if let Some(b) = open_b.filter(|&x| x > 0) {
            push_raw_delta(&mut deployed, mb, i128::from(b));
        }
    }
    if deployed.is_empty()
        && let Some(obj) = token_deltas
    {
        for (mint, dv) in obj {
            let Some(ui) = dec_from_json(dv) else {
                continue;
            };
            if ui >= Decimal::ZERO {
                continue;
            }
            let dec = default_mint_decimals(mint);
            if let Some(raw) = ui_decimal_to_raw_i128(-ui, dec) {
                push_raw_delta(&mut deployed, mint, raw);
            }
        }
    }
    if deployed.is_empty() {
        return None;
    }

    let details_map = details?;
    let amount_a = details_map
        .get("open_amount_a_raw")
        .and_then(parse_u64_json)
        .unwrap_or(0);
    let amount_b = details_map
        .get("open_amount_b_raw")
        .and_then(parse_u64_json)
        .unwrap_or(0);
    let (value_usd, value_usd_source) = deployed_usd_from_open_details(
        details_map,
        mint_a.as_deref(),
        mint_b.as_deref(),
        amount_a,
        amount_b,
    );
    let price_by_mint = price_map_from_open_details(details_map, mint_a.as_deref(), mint_b.as_deref());
    let pre_open_balances = i128_map_to_balance_mints(pre_open);
    let pre_open_value_usd = value_usd_for_balance_mints(&pre_open_balances, &price_by_mint);

    Some(SessionOpenStartSnapshot {
        ts_utc: v
            .get("ts_utc")
            .and_then(|x| x.as_str())
            .map(ToString::to_string),
        signature,
        position_pubkey: v
            .get("position_pubkey")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string),
        event: event.to_string(),
        deployed_balances: deployed
            .into_iter()
            .map(|(mint, amount_raw)| SessionBalanceMint {
                mint,
                amount_raw: format_raw_i128(amount_raw),
            })
            .collect(),
        value_usd,
        value_usd_source,
        pre_open_balances,
        pre_open_value_usd,
        price_by_mint,
        mint_resolution: mint_resolution.as_str().to_string(),
    })
}

/// First open row in session order → cycle-start reference (deployed capital + pre-open inventory).
pub fn compute_session_open_start_from_lifecycle_rows(
    rows: impl IntoIterator<Item = (Value, Option<i64>, Option<i64>)>,
    session_id: &str,
) -> Option<SessionOpenStartSnapshot> {
    let sid = session_id.trim();
    if sid.is_empty() {
        return None;
    }
    let mut cumulative: BTreeMap<String, i128> = BTreeMap::new();
    let mut open_start: Option<SessionOpenStartSnapshot> = None;

    for (raw, lp_a, lp_b) in rows {
        let row_sid = raw
            .get("rebalance_session_id")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .unwrap_or("");
        if row_sid != sid {
            continue;
        }
        let event = raw
            .get("event")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .unwrap_or("");

        if open_start.is_none() && is_lifecycle_open_event(event) {
            open_start = build_open_start_from_row(&raw, &cumulative);
        }

        let Some((_, _, _, postings)) =
            session_mint_deltas_from_lifecycle_json(&raw, lp_a, lp_b)
        else {
            continue;
        };
        for (mint, delta) in postings {
            let e = cumulative.entry(mint).or_insert(0);
            *e = e.saturating_add(delta);
        }
    }

    open_start
}

/// True when every close/open/collect row in the session can map pool legs (details or `pool_address`).
pub fn session_principal_mints_trusted<'a>(
    rows: impl IntoIterator<Item = &'a (Value, Option<i64>, Option<i64>)>,
    session_id: &str,
) -> bool {
    let sid = session_id.trim();
    if sid.is_empty() {
        return false;
    }
    for (raw, _, _) in rows.into_iter() {
        let row_sid = raw
            .get("rebalance_session_id")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .unwrap_or("");
        if row_sid != sid {
            continue;
        }
        let event = raw
            .get("event")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .unwrap_or("");
        if !is_lifecycle_close_event(event)
            && !is_lifecycle_open_event(event)
            && !is_lifecycle_collect_event(event)
        {
            continue;
        }
        let details = raw.get("details").and_then(|d| d.as_object());
        let (_, _, src) = pool_mints_from_lifecycle_row(&raw, details);
        if src == PoolMintResolveSource::Unresolved {
            return false;
        }
    }
    true
}

pub fn resolve_session_metrics_from_open_start(
    open_start: &SessionOpenStartSnapshot,
    current_balances: &[SessionBalanceMint],
    metrics_trusted: bool,
) -> SessionMetricsResolved {
    let current_value_usd = value_usd_for_balance_mints(current_balances, &open_start.price_by_mint);
    let delta_vs_pre_open_usd = match (current_value_usd, open_start.pre_open_value_usd) {
        (Some(cur), Some(pre)) if cur.is_finite() && pre.is_finite() => Some(cur - pre),
        _ => None,
    };
    SessionMetricsResolved {
        open_start: Some(open_start.clone()),
        current_value_usd,
        delta_vs_pre_open_usd,
        metrics_trusted,
    }
}

pub async fn compute_session_metrics_from_pslr(
    db: &Database,
    session_id: &str,
    current_balances: &[SessionBalanceMint],
) -> Result<SessionMetricsResolved, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT raw_json, lp_collected_token_a_raw, lp_collected_token_b_raw
        FROM position_stream_ledger_rows
        WHERE rebalance_session_id = $1
        ORDER BY ts_utc ASC NULLS LAST, signature ASC NULLS LAST
        "#,
    )
    .bind(session_id)
    .fetch_all(db.pool())
    .await?;

    let agg: Vec<(Value, Option<i64>, Option<i64>)> = rows
        .iter()
        .map(|r| {
            let raw: Value = r.get("raw_json");
            let lp_a: Option<i64> = r.try_get("lp_collected_token_a_raw").ok().flatten();
            let lp_b: Option<i64> = r.try_get("lp_collected_token_b_raw").ok().flatten();
            (raw, lp_a, lp_b)
        })
        .collect();

    let trusted = session_principal_mints_trusted(&agg, session_id);
    let open_start = compute_session_open_start_from_lifecycle_rows(agg, session_id);
    Ok(match open_start {
        Some(ref snap) => {
            resolve_session_metrics_from_open_start(snap, current_balances, trusted)
        }
        None => SessionMetricsResolved {
            open_start: None,
            current_value_usd: None,
            delta_vs_pre_open_usd: None,
            metrics_trusted: trusted,
        },
    })
}

fn is_lifecycle_collect_event(ev: &str) -> bool {
    ev == "bot_collect_fees"
}

fn is_lifecycle_swap_event(ev: &str) -> bool {
    matches!(ev, "cli_swap" | "bot_swap_exact_in" | "bot_swap" | "bot_orca_tx")
}

/// Build SESSION mint deltas from one lifecycle JSONL row.
pub fn session_mint_deltas_from_lifecycle_json(
    v: &Value,
    lp_collected_a_raw: Option<i64>,
    lp_collected_b_raw: Option<i64>,
) -> Option<(String, String, String, Vec<(String, i128)>)> {
    let session_id = v
        .get("rebalance_session_id")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    let event = v
        .get("event")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    let signature = v
        .get("signature")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();

    let details = v.get("details").and_then(|d| d.as_object());
    let token_deltas = v
        .get("fee_payer_token_deltas")
        .and_then(|d| d.as_object());
    let (mint_a, mint_b, _resolve_src) = pool_mints_from_lifecycle_row(v, details);
    let principal_events = is_lifecycle_close_event(&event)
        || is_lifecycle_open_event(&event)
        || is_lifecycle_collect_event(&event);
    let pool_mints_ok = mint_a.is_some() && mint_b.is_some();

    let lp_a = lp_collected_a_raw
        .or_else(|| {
            v.get("lp_collected_token_a_raw")
                .and_then(parse_u64_json)
                .map(|n| n as i64)
        })
        .or_else(|| {
            details
                .and_then(|d| d.get("lp_collected_token_a_raw"))
                .and_then(parse_u64_json)
                .map(|n| n as i64)
        });
    let lp_b = lp_collected_b_raw
        .or_else(|| {
            v.get("lp_collected_token_b_raw")
                .and_then(parse_u64_json)
                .map(|n| n as i64)
        })
        .or_else(|| {
            details
                .and_then(|d| d.get("lp_collected_token_b_raw"))
                .and_then(parse_u64_json)
                .map(|n| n as i64)
        });

    let mut out: Vec<(String, i128)> = Vec::new();

    if is_lifecycle_close_event(&event) {
        if pool_mints_ok {
            let ma = mint_a.as_ref().expect("pool_mints_ok");
            let mb = mint_b.as_ref().expect("pool_mints_ok");
            let close_a = details
                .and_then(|d| d.get("close_amount_a_raw"))
                .and_then(parse_u64_json);
            let close_b = details
                .and_then(|d| d.get("close_amount_b_raw"))
                .and_then(parse_u64_json);
            if let Some(a) = close_a {
                push_positive_u64(&mut out, ma, a);
            }
            if let Some(b) = close_b {
                push_positive_u64(&mut out, mb, b);
            }
            if let Some(a) = lp_a.filter(|&x| x > 0) {
                push_raw_delta(&mut out, ma, a as i128);
            }
            if let Some(b) = lp_b.filter(|&x| x > 0) {
                push_raw_delta(&mut out, mb, b as i128);
            }
        }
        if out.is_empty()
            && !principal_events
            && let Some(obj) = token_deltas
        {
            for (mint, dv) in obj {
                let Some(ui) = dec_from_json(dv) else {
                    continue;
                };
                if ui <= Decimal::ZERO {
                    continue;
                }
                let dec = default_mint_decimals(mint);
                if let Some(raw) = ui_decimal_to_raw_i128(ui, dec) {
                    push_raw_delta(&mut out, mint, raw);
                }
            }
        }
    } else if is_lifecycle_collect_event(&event) {
        if pool_mints_ok {
            let ma = mint_a.as_ref().expect("pool_mints_ok");
            let mb = mint_b.as_ref().expect("pool_mints_ok");
            if let Some(a) = lp_a.filter(|&x| x > 0) {
                push_raw_delta(&mut out, ma, a as i128);
            }
            if let Some(b) = lp_b.filter(|&x| x > 0) {
                push_raw_delta(&mut out, mb, b as i128);
            }
        }
        if out.is_empty()
            && let Some(obj) = token_deltas
        {
            for (mint, dv) in obj {
                let Some(ui) = dec_from_json(dv) else {
                    continue;
                };
                if ui <= Decimal::ZERO {
                    continue;
                }
                let dec = default_mint_decimals(mint);
                if let Some(raw) = ui_decimal_to_raw_i128(ui, dec) {
                    push_raw_delta(&mut out, mint, raw);
                }
            }
        }
    } else if is_lifecycle_open_event(&event) {
        if pool_mints_ok {
            let ma = mint_a.as_ref().expect("pool_mints_ok");
            let mb = mint_b.as_ref().expect("pool_mints_ok");
            let open_a = details
                .and_then(|d| d.get("open_amount_a_raw"))
                .and_then(parse_u64_json);
            let open_b = details
                .and_then(|d| d.get("open_amount_b_raw"))
                .and_then(parse_u64_json);
            if let Some(a) = open_a.filter(|&x| x > 0) {
                push_raw_delta(&mut out, ma, -(a as i128));
            }
            if let Some(b) = open_b.filter(|&x| x > 0) {
                push_raw_delta(&mut out, mb, -(b as i128));
            }
        }
        if out.is_empty()
            && let Some(obj) = token_deltas
        {
            for (mint, dv) in obj {
                let Some(ui) = dec_from_json(dv) else {
                    continue;
                };
                if ui == Decimal::ZERO {
                    continue;
                }
                let dec = default_mint_decimals(mint);
                if let Some(raw) = ui_decimal_to_raw_i128(ui, dec) {
                    push_raw_delta(&mut out, mint, raw);
                }
            }
        }
    } else if is_lifecycle_swap_event(&event) {
        if let Some(obj) = token_deltas {
            for (mint, dv) in obj {
                let Some(ui) = dec_from_json(dv) else {
                    continue;
                };
                if ui == Decimal::ZERO {
                    continue;
                }
                let dec = default_mint_decimals(mint);
                if let Some(raw) = ui_decimal_to_raw_i128(ui, dec) {
                    push_raw_delta(&mut out, mint, raw);
                }
            }
        }
    } else {
        return None;
    }

    if out.is_empty() {
        None
    } else {
        Some((session_id, signature, event, out))
    }
}

/// Aggregate signed raw balances for one session from lifecycle JSON values.
pub fn aggregate_session_sums_from_lifecycle_rows(
    rows: impl IntoIterator<Item = (Value, Option<i64>, Option<i64>)>,
    session_id: &str,
) -> BTreeMap<String, i128> {
    let sid = session_id.trim();
    let mut sums: BTreeMap<String, i128> = BTreeMap::new();
    for (raw, lp_a, lp_b) in rows {
        let Some((row_sid, _, _, postings)) =
            session_mint_deltas_from_lifecycle_json(&raw, lp_a, lp_b)
        else {
            continue;
        };
        if row_sid.trim() != sid {
            continue;
        }
        for (mint, delta) in postings {
            let e = sums.entry(mint).or_insert(0);
            *e = e.saturating_add(delta);
        }
    }
    sums
}

fn sums_to_spend_caps(sums: BTreeMap<String, i128>) -> BTreeMap<String, u64> {
    sums.into_iter()
        .filter_map(|(mint, v)| {
            let cap = raw_i128_to_spend_cap(v);
            (cap > 0).then_some((mint, cap))
        })
        .collect()
}

/// Scan lifecycle JSONL file and build spend caps for `session_id`.
pub fn caps_from_lifecycle_jsonl_path(
    session_id: &str,
    path: impl AsRef<Path>,
) -> SessionMintCaps {
    let sid = session_id.trim().to_string();
    let file = match File::open(path.as_ref()) {
        Ok(f) => f,
        Err(_) => return SessionMintCaps::empty(sid),
    };
    let reader = BufReader::new(file);
    let mut rows: Vec<(Value, Option<i64>, Option<i64>)> = Vec::new();
    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let lp_a = v
            .get("lp_collected_token_a_raw")
            .and_then(parse_u64_json)
            .map(|n| n as i64);
        let lp_b = v
            .get("lp_collected_token_b_raw")
            .and_then(parse_u64_json)
            .map(|n| n as i64);
        rows.push((v, lp_a, lp_b));
    }
    let sums = aggregate_session_sums_from_lifecycle_rows(rows, &sid);
    let caps = sums_to_spend_caps(sums);
    let source = if caps.is_empty() {
        SessionCapsSource::Empty
    } else {
        SessionCapsSource::LifecycleFile
    };
    SessionMintCaps {
        session_id: sid,
        caps_by_mint: caps,
        source,
    }
}

pub fn default_lifecycle_ledger_path() -> PathBuf {
    std::env::var("CLMM_POSITION_LIFECYCLE_LEDGER_PATH")
        .ok()
        .or_else(|| std::env::var("CLMM_POSITION_OPEN_LEDGER_PATH").ok())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/ledger/orca_position_lifecycle.jsonl"))
}

pub async fn read_session_balances(
    db: &Database,
    session_id: &str,
    owner: Option<&str>,
) -> Result<Vec<SessionBalanceMint>, sqlx::Error> {
    let code = session_account_code(session_id);
    let rows = if let Some(o) = owner.map(str::trim).filter(|s| !s.is_empty()) {
        sqlx::query(
            r#"
            SELECT b.mint, b.amount_raw
            FROM wallet_gl_balance b
            JOIN wallet_gl_account a ON a.id = b.account_id
            WHERE a.account_code = $1 AND a.owner = $2
            ORDER BY b.mint
            "#,
        )
        .bind(&code)
        .bind(o)
        .fetch_all(db.pool())
        .await?
    } else {
        sqlx::query(
            r#"
            SELECT b.mint, b.amount_raw
            FROM wallet_gl_balance b
            JOIN wallet_gl_account a ON a.id = b.account_id
            WHERE a.account_code = $1
            ORDER BY b.mint
            "#,
        )
        .bind(&code)
        .fetch_all(db.pool())
        .await?
    };

    Ok(rows
        .iter()
        .map(|r| SessionBalanceMint {
            mint: r.get("mint"),
            amount_raw: r.get("amount_raw"),
        })
        .collect())
}

pub async fn compute_session_balances_from_pslr(
    db: &Database,
    session_id: &str,
) -> Result<Vec<SessionBalanceMint>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT raw_json, lp_collected_token_a_raw, lp_collected_token_b_raw
        FROM position_stream_ledger_rows
        WHERE rebalance_session_id = $1
        ORDER BY ts_utc ASC NULLS LAST
        "#,
    )
    .bind(session_id)
    .fetch_all(db.pool())
    .await?;

    let agg: Vec<(Value, Option<i64>, Option<i64>)> = rows
        .iter()
        .map(|r| {
            let raw: Value = r.get("raw_json");
            let lp_a: Option<i64> = r.try_get("lp_collected_token_a_raw").ok().flatten();
            let lp_b: Option<i64> = r.try_get("lp_collected_token_b_raw").ok().flatten();
            (raw, lp_a, lp_b)
        })
        .collect();
    let sums = aggregate_session_sums_from_lifecycle_rows(agg, session_id);
    Ok(sums
        .into_iter()
        .map(|(mint, amount_raw)| SessionBalanceMint {
            mint,
            amount_raw: format_raw_i128(amount_raw),
        })
        .collect())
}

/// Balances for session USD metrics: GL when it matches PSLR aggregate, else PSLR (lifecycle truth).
pub async fn session_balances_for_metrics(
    db: &Database,
    session_id: &str,
    gl_balances: &[SessionBalanceMint],
    owner: Option<&str>,
) -> Result<Vec<SessionBalanceMint>, sqlx::Error> {
    let pslr = compute_session_balances_from_pslr(db, session_id).await?;
    if gl_balances.is_empty() {
        return Ok(pslr);
    }
    if gl_pslr_match(gl_balances, &pslr) {
        return Ok(gl_balances.to_vec());
    }
    let _ = owner;
    Ok(pslr)
}

pub fn gl_pslr_match(gl: &[SessionBalanceMint], pslr: &[SessionBalanceMint]) -> bool {
    let mut gl_map: BTreeMap<String, String> = BTreeMap::new();
    for b in gl {
        gl_map.insert(b.mint.clone(), b.amount_raw.clone());
    }
    let mut pslr_map: BTreeMap<String, String> = BTreeMap::new();
    for b in pslr {
        pslr_map.insert(b.mint.clone(), b.amount_raw.clone());
    }
    let mut all: BTreeMap<String, ()> = BTreeMap::new();
    for m in gl_map.keys() {
        all.insert(m.clone(), ());
    }
    for m in pslr_map.keys() {
        all.insert(m.clone(), ());
    }
    for mint in all.keys() {
        match (gl_map.get(mint), pslr_map.get(mint)) {
            (Some(gv), Some(pv)) if gv != pv => return false,
            (Some(_), None) | (None, Some(_)) => return false,
            _ => {}
        }
    }
    true
}

pub fn reopen_session_require_reconcile() -> bool {
    match std::env::var("CLMM_REOPEN_SESSION_REQUIRE_RECONCILE") {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

/// Resolve spend caps: GL + PSLR (min per mint), else lifecycle file.
pub async fn resolve_session_mint_caps(
    db: Option<&Database>,
    session_id: &str,
    owner: Option<&str>,
) -> SessionMintCaps {
    let sid = session_id.trim().to_string();
    if sid.is_empty() {
        return SessionMintCaps::empty(sid);
    }

    if let Some(db) = db {
        let gl = read_session_balances(db, &sid, owner).await.unwrap_or_default();
        let pslr = compute_session_balances_from_pslr(db, &sid)
            .await
            .unwrap_or_default();
        let gl_rows: Vec<(String, String)> = gl
            .iter()
            .map(|b| (b.mint.clone(), b.amount_raw.clone()))
            .collect();
        let pslr_rows: Vec<(String, String)> = pslr
            .iter()
            .map(|b| (b.mint.clone(), b.amount_raw.clone()))
            .collect();
        let gl_caps = caps_map_from_balance_rows(&gl_rows);
        let pslr_caps = caps_map_from_balance_rows(&pslr_rows);

        if reopen_session_require_reconcile() && !gl_pslr_match(&gl, &pslr) {
            return SessionMintCaps {
                session_id: sid,
                caps_by_mint: BTreeMap::new(),
                source: SessionCapsSource::Empty,
            };
        }

        if !gl_caps.is_empty() || !pslr_caps.is_empty() {
            let merged = merge_min_caps(&gl_caps, &pslr_caps);
            let source = if !gl_caps.is_empty() && gl_pslr_match(&gl, &pslr) {
                SessionCapsSource::Gl
            } else if !pslr_caps.is_empty() && gl_caps.is_empty() {
                SessionCapsSource::PslrFallback
            } else {
                SessionCapsSource::ReconciledMin
            };
            return SessionMintCaps {
                session_id: sid,
                caps_by_mint: merged,
                source,
            };
        }
    }

    caps_from_lifecycle_jsonl_path(&sid, default_lifecycle_ledger_path())
}

/// Outcome of applying one lifecycle row to SESSION GL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLifecyclePostingOutcome {
    SkippedNoDeltas,
    SkippedAlready,
    Applied,
}

pub async fn session_lifecycle_posting_already_applied(
    db: &Database,
    event_id: &str,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query(
        r#"SELECT 1 AS ok FROM wallet_gl_posting WHERE event_id = $1 LIMIT 1"#,
    )
    .bind(event_id)
    .fetch_optional(db.pool())
    .await?;
    Ok(row.is_some())
}

async fn ensure_session_account(
    db: &Database,
    owner: Option<&str>,
    session_id: &str,
) -> Result<i64, sqlx::Error> {
    let code = session_account_code(session_id);
    let row = sqlx::query(
        r#"
        INSERT INTO wallet_gl_account (account_type, account_code, owner, session_id, notes)
        VALUES ('session', $1, $2, $3, 'analytics retention; never auto-closed or liquidated')
        ON CONFLICT (account_code) DO UPDATE SET
            updated_at = NOW()
        RETURNING id
        "#,
    )
    .bind(&code)
    .bind(owner)
    .bind(session_id)
    .fetch_one(db.pool())
    .await?;
    Ok(row.get::<i64, _>("id"))
}

async fn apply_balance_delta(
    db: &Database,
    account_id: i64,
    mint: &str,
    delta: i128,
    event_id: &str,
    kind: &str,
) -> Result<(), sqlx::Error> {
    let mut tx = db.pool().begin().await?;

    let row = sqlx::query(
        r#"
        SELECT amount_raw FROM wallet_gl_balance
        WHERE account_id = $1 AND mint = $2
        FOR UPDATE
        "#,
    )
    .bind(account_id)
    .bind(mint)
    .fetch_optional(&mut *tx)
    .await?;

    let new_raw = if let Some(r) = row {
        let cur = parse_raw_i128(r.get::<String, _>("amount_raw").as_str()).unwrap_or(0);
        cur.saturating_add(delta)
    } else {
        delta
    };

    sqlx::query(
        r#"
        INSERT INTO wallet_gl_balance (account_id, mint, amount_raw, last_event_id, updated_at)
        VALUES ($1, $2, $3, $4, NOW())
        ON CONFLICT (account_id, mint) DO UPDATE SET
            amount_raw = EXCLUDED.amount_raw,
            last_event_id = EXCLUDED.last_event_id,
            updated_at = NOW()
        "#,
    )
    .bind(account_id)
    .bind(mint)
    .bind(format_raw_i128(new_raw))
    .bind(event_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO wallet_gl_posting (event_id, account_id, mint, delta_raw, kind)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(event_id)
    .bind(account_id)
    .bind(mint)
    .bind(format_raw_i128(delta))
    .bind(kind)
    .execute(&mut *tx)
    .await?;

    tx.commit().await
}

/// Apply signed mint deltas to a SESSION account (creates account row if needed).
pub async fn apply_session_mint_postings(
    db: &Database,
    session_id: &str,
    owner: Option<&str>,
    event_id: &str,
    kind: &str,
    postings: &[(String, i128)],
) -> Result<(), sqlx::Error> {
    if postings.is_empty() {
        return Ok(());
    }
    let account_id = ensure_session_account(db, owner, session_id).await?;
    for (mint, delta) in postings {
        apply_balance_delta(db, account_id, mint, *delta, event_id, kind).await?;
    }
    Ok(())
}

fn owner_from_lifecycle_json(v: &Value) -> Option<&str> {
    v.get("fee_payer_pubkey")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Apply SESSION GL postings from one lifecycle JSON object (idempotent on `event_id`).
pub async fn apply_session_postings_from_lifecycle_row(
    db: &Database,
    v: &Value,
    lp_collected_a_raw: Option<i64>,
    lp_collected_b_raw: Option<i64>,
) -> Result<SessionLifecyclePostingOutcome, sqlx::Error> {
    let Some((session_id, signature, event, postings)) =
        session_mint_deltas_from_lifecycle_json(v, lp_collected_a_raw, lp_collected_b_raw)
    else {
        return Ok(SessionLifecyclePostingOutcome::SkippedNoDeltas);
    };
    let event_id = lifecycle_posting_event_id(&signature);
    if session_lifecycle_posting_already_applied(db, &event_id).await? {
        return Ok(SessionLifecyclePostingOutcome::SkippedAlready);
    }
    let owner = owner_from_lifecycle_json(v);
    apply_session_mint_postings(db, &session_id, owner, &event_id, &event, &postings).await?;
    Ok(SessionLifecyclePostingOutcome::Applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_close_row_posts_principal_and_lp() {
        let v = serde_json::json!({
            "event": "bot_close_position",
            "signature": "sig-close-1",
            "rebalance_session_id": "sess-abc",
            "lp_collected_token_a_raw": 50_000,
            "lp_collected_token_b_raw": 0,
            "details": {
                "token_mint_a": WSOL_MINT,
                "token_mint_b": USDC_MINT,
                "close_amount_a_raw": 1_000_000_000u64,
                "close_amount_b_raw": 2_000_000u64
            }
        });
        let (sid, sig, ev, posts) =
            session_mint_deltas_from_lifecycle_json(&v, Some(50_000), Some(0)).expect("posts");
        assert_eq!(sid, "sess-abc");
        assert_eq!(sig, "sig-close-1");
        assert_eq!(ev, "bot_close_position");
        assert_eq!(posts.len(), 3);
        assert!(posts.iter().any(|(m, d)| m == WSOL_MINT && *d == 1_000_000_000));
        assert!(posts.iter().any(|(m, d)| m == USDC_MINT && *d == 2_000_000));
        assert!(posts.iter().any(|(m, d)| m == WSOL_MINT && *d == 50_000));
    }

    #[test]
    fn merge_min_caps_picks_smaller() {
        let mut gl = BTreeMap::new();
        gl.insert("mintA".to_string(), 100);
        let mut pslr = BTreeMap::new();
        pslr.insert("mintA".to_string(), 40);
        let m = merge_min_caps(&gl, &pslr);
        assert_eq!(m.get("mintA"), Some(&40));
    }

    #[test]
    fn spend_cap_clamps_negative() {
        assert_eq!(raw_i128_to_spend_cap(-5), 0);
        assert_eq!(raw_i128_to_spend_cap(42), 42);
    }

    #[test]
    fn session_account_code_format() {
        assert_eq!(session_account_code("abc"), "SESSION:abc");
    }

    #[test]
    fn lifecycle_posting_event_id_fits_wallet_gl_column() {
        let sig = "5WjbEASV4hjQd1yAwb2HyJnBxWeDJqv7M1xc3r4amv9KfECMskwQkNre5gGWdHLf4Mc9BSjxYUMKzgiueeoTZwFb";
        let id = lifecycle_posting_event_id(sig);
        assert!(
            id.len() <= LIFECYCLE_POSTING_EVENT_ID_MAX_LEN,
            "len={}",
            id.len()
        );
    }

    #[test]
    fn aggregate_session_sums_net_of_open_and_close() {
        let sid = "sess-net-1";
        let close = serde_json::json!({
            "event": "bot_close_position",
            "signature": "sig-c",
            "rebalance_session_id": sid,
            "details": {
                "token_mint_a": WSOL_MINT,
                "token_mint_b": USDC_MINT,
                "close_amount_a_raw": 1_000u64,
                "close_amount_b_raw": 500u64
            }
        });
        let open = serde_json::json!({
            "event": "bot_open_position",
            "signature": "sig-o",
            "rebalance_session_id": sid,
            "details": {
                "token_mint_a": WSOL_MINT,
                "token_mint_b": USDC_MINT,
                "open_amount_a_raw": 300u64,
                "open_amount_b_raw": 100u64
            }
        });
        let rows = vec![(close, None, None), (open, None, None)];
        let sums = aggregate_session_sums_from_lifecycle_rows(rows, sid);
        assert_eq!(sums.get(WSOL_MINT), Some(&700)); // 1000 - 300
        assert_eq!(sums.get(USDC_MINT), Some(&400)); // 500 - 100
        let caps = sums_to_spend_caps(sums);
        assert_eq!(caps.get(WSOL_MINT), Some(&700));
        assert_eq!(caps.get(USDC_MINT), Some(&400));
    }

    #[test]
    fn caps_from_lifecycle_jsonl_path_filters_session_and_mints() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("lifecycle.jsonl");
        let sid = "sess-jsonl-1";
        let other = "sess-other";
        let lines = [
            serde_json::json!({
                "event": "bot_close_position",
                "signature": "sig-1",
                "rebalance_session_id": sid,
                "details": {
                    "token_mint_a": WSOL_MINT,
                    "token_mint_b": USDC_MINT,
                    "close_amount_a_raw": 2_000u64,
                    "close_amount_b_raw": 0u64
                }
            })
            .to_string(),
            serde_json::json!({
                "event": "bot_close_position",
                "signature": "sig-2",
                "rebalance_session_id": other,
                "details": {
                    "token_mint_a": WSOL_MINT,
                    "token_mint_b": USDC_MINT,
                    "close_amount_a_raw": 9_999u64,
                    "close_amount_b_raw": 0u64
                }
            })
            .to_string(),
        ];
        std::fs::write(&path, lines.join("\n")).expect("write jsonl");

        let caps = caps_from_lifecycle_jsonl_path(sid, &path);
        assert_eq!(caps.session_id, sid);
        assert_eq!(caps.source, SessionCapsSource::LifecycleFile);
        assert_eq!(caps.cap_u64_for_mint(WSOL_MINT), 2_000);
        assert_eq!(caps.cap_u64_for_mint(USDC_MINT), 0);
    }

    #[test]
    fn caps_from_lifecycle_jsonl_missing_file_is_empty() {
        let caps = caps_from_lifecycle_jsonl_path(
            "sess-x",
            std::path::Path::new("/nonexistent/lifecycle-test.jsonl"),
        );
        assert!(caps.is_empty());
        assert_eq!(caps.source, SessionCapsSource::Empty);
    }

    #[test]
    fn compute_open_start_from_first_open_row() {
        let sid = "sess-open-1";
        let open = serde_json::json!({
            "event": "bot_open_position",
            "signature": "sig-open-1",
            "ts_utc": "2026-05-20T12:00:00Z",
            "rebalance_session_id": sid,
            "position_pubkey": "Pos111111111111111111111111111111111111111111",
            "details": {
                "token_mint_a": WSOL_MINT,
                "token_mint_b": USDC_MINT,
                "open_amount_a_raw": 60_000_000u64,
                "open_amount_b_raw": 4_900_000u64,
                "open_quote_estimated_value_usd": 9.42,
                "event_price_a_usd": 150.0,
                "event_price_b_usd": 1.0
            }
        });
        let rows = vec![(open, None, None)];
        let snap = compute_session_open_start_from_lifecycle_rows(rows, sid).expect("snap");
        assert_eq!(snap.signature, "sig-open-1");
        assert_eq!(snap.deployed_balances.len(), 2);
        assert_eq!(snap.value_usd, Some(9.42));
        assert_eq!(snap.value_usd_source, "open_quote_estimated_value_usd");
        assert!(snap.pre_open_balances.is_empty());
    }

    #[test]
    fn compute_open_start_pre_open_after_close_in_same_session() {
        let sid = "sess-open-2";
        let close = serde_json::json!({
            "event": "bot_close_position",
            "signature": "sig-close-1",
            "rebalance_session_id": sid,
            "details": {
                "token_mint_a": WSOL_MINT,
                "token_mint_b": USDC_MINT,
                "close_amount_a_raw": 60_000_000u64,
                "close_amount_b_raw": 4_900_000u64
            }
        });
        let open = serde_json::json!({
            "event": "bot_open_position",
            "signature": "sig-open-2",
            "rebalance_session_id": sid,
            "details": {
                "token_mint_a": WSOL_MINT,
                "token_mint_b": USDC_MINT,
                "open_amount_a_raw": 60_000_000u64,
                "open_amount_b_raw": 4_900_000u64,
                "event_price_a_usd": 150.0,
                "event_price_b_usd": 1.0
            }
        });
        let rows = vec![(close, None, None), (open, None, None)];
        let snap = compute_session_open_start_from_lifecycle_rows(rows, sid).expect("snap");
        assert_eq!(snap.pre_open_balances.len(), 2);
        assert!(snap.pre_open_value_usd.unwrap_or(0.0) > 0.0);
        let current = vec![
            SessionBalanceMint {
                mint: WSOL_MINT.to_string(),
                amount_raw: "0".to_string(),
            },
            SessionBalanceMint {
                mint: USDC_MINT.to_string(),
                amount_raw: "0".to_string(),
            },
        ];
        let metrics = resolve_session_metrics_from_open_start(&snap, &current, true);
        assert!(metrics.current_value_usd.unwrap_or(1.0).abs() < 1e-6);
    }

    #[test]
    fn close_without_details_mints_uses_pool_address_not_fee_payer_deltas() {
        let sid = "sess-mint-bug-1";
        let close = serde_json::json!({
            "event": "bot_close_position",
            "signature": "sig-close-bug",
            "rebalance_session_id": sid,
            "pool_address": "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE",
            "details": {
                "close_amount_a_raw": 94_165_873u64,
                "close_amount_b_raw": 0u64
            },
            "fee_payer_token_deltas": {
                "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v": "0.000123",
                "HkuxEuzPTDtVMQVPtWTDdPiKz5jToCCsszhudbYNFL2X": "-1"
            }
        });
        let (_, _, _, posts) =
            session_mint_deltas_from_lifecycle_json(&close, None, None).expect("posts");
        let wsol = posts
            .iter()
            .find(|(m, _)| m == WSOL_MINT)
            .map(|(_, d)| *d)
            .expect("SOL leg on WSOL mint");
        assert_eq!(wsol, 94_165_873);
        assert!(
            !posts.iter().any(|(m, d)| {
                m == USDC_MINT && *d > 10_000_000
            }),
            "must not book ~94 USDC from SOL lamports raw"
        );
        let open = serde_json::json!({
            "event": "bot_open_position",
            "signature": "sig-open-after",
            "rebalance_session_id": sid,
            "pool_address": "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE",
            "details": {
                "token_mint_a": WSOL_MINT,
                "token_mint_b": USDC_MINT,
                "open_amount_a_raw": 47_424_411u64,
                "open_amount_b_raw": 4_118_888u64,
                "open_quote_estimated_value_usd": 8.26,
                "event_price_a_usd": 88.17,
                "event_price_b_usd": 1.003
            }
        });
        let pre_open_usd = compute_session_open_start_from_lifecycle_rows(
            vec![(close, None, None), (open, None, None)],
            sid,
        )
        .and_then(|s| s.pre_open_value_usd);
        assert!(
            pre_open_usd.unwrap_or(0.0) < 20.0,
            "pre_open USD must not inflate to ~97: got {:?}",
            pre_open_usd
        );
    }

    #[test]
    fn session_balances_for_metrics_prefers_pslr_when_gl_doubles_open() {
        let open = serde_json::json!({
            "event": "bot_open_position",
            "signature": "sig-o",
            "rebalance_session_id": "sess-metrics-1",
            "details": {
                "token_mint_a": WSOL_MINT,
                "token_mint_b": USDC_MINT,
                "open_amount_a_raw": 60_435_307u64,
                "open_amount_b_raw": 4_720_942u64,
                "event_price_a_usd": 85.0,
                "event_price_b_usd": 1.0
            }
        });
        let (_, _, _, posts) =
            session_mint_deltas_from_lifecycle_json(&open, None, None).expect("posts");
        let pslr: Vec<SessionBalanceMint> = posts
            .into_iter()
            .map(|(mint, amount_raw)| SessionBalanceMint {
                mint,
                amount_raw: format_raw_i128(amount_raw),
            })
            .collect();
        let gl = vec![
            SessionBalanceMint {
                mint: WSOL_MINT.to_string(),
                amount_raw: "-120870615".to_string(),
            },
            SessionBalanceMint {
                mint: USDC_MINT.to_string(),
                amount_raw: "-9586801".to_string(),
            },
        ];
        assert!(!gl_pslr_match(&gl, &pslr));
        let snap = build_open_start_from_row(&open, &BTreeMap::new()).expect("open_start");
        let metrics_gl = resolve_session_metrics_from_open_start(&snap, &gl, true);
        let metrics_pslr = resolve_session_metrics_from_open_start(&snap, &pslr, true);
        let gl_usd = metrics_gl.current_value_usd.unwrap_or(0.0);
        let pslr_usd = metrics_pslr.current_value_usd.unwrap_or(0.0);
        assert!(gl_usd < -15.0, "doubled GL should be ~-20 USD: {gl_usd}");
        assert!(pslr_usd > -12.0 && pslr_usd < -8.0, "PSLR single open ~-10 USD: {pslr_usd}");
    }

    #[test]
    fn gl_pslr_match_requires_same_mint_amounts() {
        let gl = vec![SessionBalanceMint {
            mint: WSOL_MINT.to_string(),
            amount_raw: "1000".to_string(),
        }];
        let pslr_ok = vec![SessionBalanceMint {
            mint: WSOL_MINT.to_string(),
            amount_raw: "1000".to_string(),
        }];
        let pslr_bad = vec![SessionBalanceMint {
            mint: WSOL_MINT.to_string(),
            amount_raw: "999".to_string(),
        }];
        assert!(gl_pslr_match(&gl, &pslr_ok));
        assert!(!gl_pslr_match(&gl, &pslr_bad));
        assert!(!gl_pslr_match(&gl, &[]));
    }

    #[tokio::test]
    async fn resolve_session_mint_caps_without_db_reads_jsonl_env_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("lifecycle.jsonl");
        let sid = "sess-resolve-1";
        let line = serde_json::json!({
            "event": "bot_close_position",
            "signature": "sig-r",
            "rebalance_session_id": sid,
            "details": {
                "token_mint_a": WSOL_MINT,
                "token_mint_b": USDC_MINT,
                "close_amount_a_raw": 42u64,
                "close_amount_b_raw": 7u64
            }
        });
        std::fs::write(&path, line.to_string()).expect("write jsonl");
        let path_s = path.to_string_lossy().to_string();
        unsafe {
            std::env::set_var("CLMM_POSITION_LIFECYCLE_LEDGER_PATH", &path_s);
            std::env::remove_var("CLMM_REOPEN_SESSION_REQUIRE_RECONCILE");
        }

        let caps = resolve_session_mint_caps(None, sid, None).await;
        assert_eq!(caps.source, SessionCapsSource::LifecycleFile);
        assert_eq!(caps.cap_u64_for_mint(WSOL_MINT), 42);
        assert_eq!(caps.cap_u64_for_mint(USDC_MINT), 7);

        unsafe {
            std::env::remove_var("CLMM_POSITION_LIFECYCLE_LEDGER_PATH");
        }
    }
}
