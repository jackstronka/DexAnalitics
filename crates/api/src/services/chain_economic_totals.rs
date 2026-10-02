//! Chain-level economic net PnL rollup (wynik ekonomiczny łańcucha).
//!
//! Single place for: end NAV per node / chain headline, totals refresh from lineage nodes,
//! and reconciliation with stream PnL totals. See `doc/CHAIN_ECONOMIC_NET_REFACTOR_PLAN.md`.

use crate::models::{
    PositionStreamLineageNode, PositionStreamPnLResponse, StreamPnLInterpretation,
};
use rust_decimal::Decimal;

/// Source tag for end NAV resolution (diagnostics / future API `end_nav_source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainEndNavSource {
    LiveCurrent,
    MaterializedEnd,
    LifecycleCloseAmounts,
    CloseEstimate,
    Missing,
}

/// Positive USD from optional chain-history column string (materialized end/start marks).
fn positive_usd_from_chain_history_column(col: Option<&str>) -> Option<Decimal> {
    let t = col?.trim();
    if t.is_empty() {
        return None;
    }
    let d: Decimal = t.parse().ok()?;
    (d > Decimal::ZERO).then_some(d)
}

fn ui_amount_from_raw(raw: u64, decimals: u8) -> Decimal {
    if decimals == 0 {
        return Decimal::from(raw);
    }
    let scale = Decimal::from(10u64.pow(decimals as u32));
    Decimal::from(raw) / scale
}

/// Close NAV from lifecycle `close_amount_*_raw` × event USD prices (exact at close tx).
pub fn close_nav_usd_from_raw_amounts_and_prices(
    close_amount_a_raw: u64,
    close_amount_b_raw: u64,
    decimals_a: u8,
    decimals_b: u8,
    price_a_usd: Decimal,
    price_b_usd: Decimal,
) -> Option<Decimal> {
    if price_a_usd <= Decimal::ZERO && price_b_usd <= Decimal::ZERO {
        return None;
    }
    let qa = ui_amount_from_raw(close_amount_a_raw, decimals_a);
    let qb = ui_amount_from_raw(close_amount_b_raw, decimals_b);
    let nav = qa * price_a_usd + qb * price_b_usd;
    (nav > Decimal::ZERO).then_some(nav)
}

/// End NAV for one lineage node with source tag (see [`lineage_node_end_nav_usd`]).
pub fn lineage_node_end_nav_with_source(
    n: &PositionStreamLineageNode,
) -> (Decimal, ChainEndNavSource) {
    if n.current_value_usd > Decimal::ZERO {
        return (n.current_value_usd, ChainEndNavSource::LiveCurrent);
    }
    if let Some(end) =
        positive_usd_from_chain_history_column(n.chain_history_end_value_usd.as_deref())
    {
        return (end, ChainEndNavSource::MaterializedEnd);
    }
    if let Some(end) = positive_usd_from_chain_history_column(n.lifecycle_close_nav_usd.as_deref())
    {
        return (end, ChainEndNavSource::LifecycleCloseAmounts);
    }
    if n.closed_ts_utc.is_some() && n.baseline_value_usd > Decimal::ZERO {
        let est =
            n.baseline_value_usd + n.fees_collected_usd + n.realized_cashflow_usd - n.tx_fees_usd;
        if est > Decimal::ZERO {
            return (est, ChainEndNavSource::CloseEstimate);
        }
    }
    (Decimal::ZERO, ChainEndNavSource::Missing)
}

/// End NAV for one lineage node: live/current mark, else materialized / lifecycle close, else estimate.
pub fn lineage_node_end_nav_usd(n: &PositionStreamLineageNode) -> Decimal {
    lineage_node_end_nav_with_source(n).0
}

/// Headline **current** for chain totals: last node's end NAV, scanning backward if still zero.
pub fn chain_headline_end_nav_usd(nodes: &[PositionStreamLineageNode]) -> Decimal {
    for n in nodes.iter().rev() {
        let v = lineage_node_end_nav_usd(n);
        if v > Decimal::ZERO {
            return v;
        }
    }
    Decimal::ZERO
}

/// When DB stream PnL totals disagree with per-node lineage marks (e.g. one-leg baseline snapshot
/// before `open_amount_*_raw` enrichment), align headline totals with node rows best-effort.
pub fn reconcile_stream_pnl_totals_with_nodes(
    totals: &mut PositionStreamPnLResponse,
    nodes: &[PositionStreamLineageNode],
) {
    let (Some(first), Some(last)) = (nodes.first(), nodes.last()) else {
        return;
    };
    let baseline = first.baseline_value_usd;
    if baseline.is_zero() {
        return;
    }

    let end_nav = chain_headline_end_nav_usd(nodes);
    let hodl_degraded = totals.hodl_value_usd < baseline * Decimal::new(8, 1);
    let current_stale = totals.current_ts_utc.is_none()
        || totals.current_ts_utc == totals.baseline_ts_utc
        || totals.current_value_usd == totals.baseline_value_usd;
    let current_missing = totals.current_value_usd.is_zero() && end_nav > Decimal::ZERO;

    if !hodl_degraded && !current_stale && !current_missing {
        return;
    }

    totals.baseline_value_usd = baseline;
    totals.baseline_ts_utc = first.opened_ts_utc.clone();
    totals.current_value_usd = if end_nav > Decimal::ZERO {
        end_nav
    } else {
        last.current_value_usd
    };
    totals.current_ts_utc = last
        .closed_ts_utc
        .clone()
        .or_else(|| last.opened_ts_utc.clone());

    if hodl_degraded {
        totals.hodl_value_usd = baseline;
        totals.il_usd = totals.current_value_usd - totals.hodl_value_usd;
        totals.il_pct = if totals.hodl_value_usd.is_zero() {
            Decimal::ZERO
        } else {
            totals.il_usd / totals.hodl_value_usd
        };
        totals.clean_il_usd = totals.il_usd;
        totals.clean_il_pct = totals.il_pct;
        totals.lp_vs_hodl_with_fees_usd = totals.il_usd + totals.lp_fees_total_usd;
        totals.lp_vs_hodl_with_fees_pct = if totals.hodl_value_usd.is_zero() {
            Decimal::ZERO
        } else {
            totals.lp_vs_hodl_with_fees_usd / totals.hodl_value_usd
        };
    }

    totals.net_pnl_usd = totals.current_value_usd + totals.realized_cashflow_usd
        - totals.baseline_value_usd
        - totals.tx_fees_usd;
    totals.net_pnl_pct = if totals.baseline_value_usd.is_zero() {
        Decimal::ZERO
    } else {
        totals.net_pnl_usd / totals.baseline_value_usd
    };
}

pub(crate) fn maybe_compute_totals_from_nodes(
    entry: &str,
    existing: &Option<PositionStreamPnLResponse>,
    nodes: &[PositionStreamLineageNode],
    note: Option<&str>,
) -> Option<PositionStreamPnLResponse> {
    if nodes.is_empty() {
        return None;
    }
    let totals_is_placeholder = existing.as_ref().is_some_and(|t| {
        let note_lc = t.note.as_deref().unwrap_or_default().to_ascii_lowercase();
        let stale_meta = note_lc.contains("no valuation snapshots")
            || t.valuation_price_time_kind == "node_fallback_unavailable";
        (t.baseline_value_usd.is_zero()
            && t.current_value_usd.is_zero()
            && t.tx_fees_usd.is_zero()
            && t.realized_cashflow_usd.is_zero()
            && t.net_pnl_usd.is_zero()
            && stale_meta)
            || (t.baseline_value_usd.is_zero() && !t.current_value_usd.is_zero() && stale_meta)
    });
    if existing.is_some() && !totals_is_placeholder {
        return None;
    }

    let baseline_value_usd = nodes
        .first()
        .map(|n| n.baseline_value_usd)
        .unwrap_or(Decimal::ZERO);
    let current_value_usd = chain_headline_end_nav_usd(nodes);
    let tx_fees_usd: Decimal = nodes.iter().map(|n| n.tx_fees_usd).sum();
    let realized_cashflow_usd: Decimal = nodes.iter().map(|n| n.realized_cashflow_usd).sum();
    let realized_lp_fees_usd: Decimal = nodes.iter().map(|n| n.fees_collected_usd).sum();
    let clean_il_usd = Decimal::ZERO;
    let clean_il_pct = Decimal::ZERO;
    let lp_fees_total_usd = realized_lp_fees_usd;
    let net_pnl_usd = current_value_usd + realized_cashflow_usd - baseline_value_usd - tx_fees_usd;
    let net_pnl_pct = if baseline_value_usd.is_zero() {
        Decimal::ZERO
    } else {
        net_pnl_usd / baseline_value_usd
    };

    let mut totals = PositionStreamPnLResponse {
        position_address: entry.to_string(),
        baseline_ts_utc: nodes.first().and_then(|n| n.opened_ts_utc.clone()),
        current_ts_utc: nodes.last().and_then(|n| n.closed_ts_utc.clone()),
        baseline_value_usd,
        current_value_usd,
        hodl_value_usd: Decimal::ZERO,
        il_usd: Decimal::ZERO,
        il_pct: Decimal::ZERO,
        clean_il_usd,
        clean_il_pct,
        realized_lp_fees_usd,
        uncollected_lp_fees_usd: Decimal::ZERO,
        lp_fees_total_usd,
        lp_vs_hodl_with_fees_usd: lp_fees_total_usd,
        lp_vs_hodl_with_fees_pct: Decimal::ZERO,
        valuation_price_time_kind: "node_fallback_unavailable".to_string(),
        price_basis_note: Some(
            "Fallback totals from lineage nodes do not have baseline token basket, so HODL/IL price basis is unavailable.".to_string(),
        ),
        tx_fees_usd,
        realized_cashflow_usd,
        net_pnl_usd,
        net_pnl_pct,
        economic_quality: None,
        end_nav_source: None,
        interpretation: StreamPnLInterpretation {
            economic_net_pnl_caption_pl:
                "Wynik ekonomiczny (fallback z węzłów lineage, bez pełnych snapshotów DB): końcowy NAV + suma cashflow z węzłów − baseline pierwszego węzła − suma tx fees z węzłów."
                    .to_string(),
            il_vs_initial_hodl_caption_pl:
                "Benchmark IL vs HODL: w tym trybie nie liczony (brak ilości tokenów ze snapshotów); pola il_* są zerowe."
                    .to_string(),
        },
        note: note.map(|s| s.to_string()),
    };
    apply_chain_economic_quality_meta(&mut totals, nodes);
    Some(totals)
}

fn lift_node_end_nav_on_nodes(nodes: &mut [PositionStreamLineageNode]) {
    for n in nodes.iter_mut() {
        if n.current_value_usd.is_zero() {
            let end = lineage_node_end_nav_usd(n);
            if end > Decimal::ZERO {
                n.current_value_usd = end;
                n.net_pnl_usd =
                    end + n.realized_cashflow_usd - n.baseline_value_usd - n.tx_fees_usd;
                if !n.baseline_value_usd.is_zero() {
                    n.net_pnl_pct = n.net_pnl_usd / n.baseline_value_usd;
                }
            }
        }
    }
}

fn apply_economic_totals_fields_from_nodes(
    t: &mut PositionStreamPnLResponse,
    nodes: &[PositionStreamLineageNode],
    refresh_hodl_meta: bool,
) {
    let tx_sum: Decimal = nodes.iter().map(|n| n.tx_fees_usd).sum();
    let fee_sum: Decimal = nodes.iter().map(|n| n.fees_collected_usd).sum();
    let cashflow_sum: Decimal = nodes.iter().map(|n| n.realized_cashflow_usd).sum();
    if t.tx_fees_usd.is_zero() && tx_sum > Decimal::ZERO {
        t.tx_fees_usd = tx_sum;
    }
    if t.realized_lp_fees_usd.is_zero() && fee_sum > Decimal::ZERO {
        t.realized_lp_fees_usd = fee_sum;
        t.lp_fees_total_usd = fee_sum + t.uncollected_lp_fees_usd;
    }
    if t.realized_cashflow_usd.is_zero() && cashflow_sum != Decimal::ZERO {
        t.realized_cashflow_usd = cashflow_sum;
    }
    let end_nav = chain_headline_end_nav_usd(nodes);
    if t.current_value_usd.is_zero() && end_nav > Decimal::ZERO {
        t.current_value_usd = end_nav;
        t.current_ts_utc = nodes
            .last()
            .and_then(|n| n.closed_ts_utc.clone())
            .or_else(|| nodes.last().and_then(|n| n.opened_ts_utc.clone()));
    }
    if !t.hodl_value_usd.is_zero() {
        t.lp_vs_hodl_with_fees_usd = t.il_usd + t.lp_fees_total_usd;
        t.lp_vs_hodl_with_fees_pct = if t.hodl_value_usd.is_zero() {
            Decimal::ZERO
        } else {
            t.lp_vs_hodl_with_fees_usd / t.hodl_value_usd
        };
    }
    t.net_pnl_usd =
        t.current_value_usd + t.realized_cashflow_usd - t.baseline_value_usd - t.tx_fees_usd;
    t.net_pnl_pct = if t.baseline_value_usd.is_zero() {
        Decimal::ZERO
    } else {
        t.net_pnl_usd / t.baseline_value_usd
    };
    if refresh_hodl_meta
        && !t.hodl_value_usd.is_zero()
        && (!t.il_usd.is_zero() || !t.clean_il_usd.is_zero())
        && t.valuation_price_time_kind == "node_fallback_unavailable"
    {
        t.valuation_price_time_kind = "lineage_nodes_reconciled".to_string();
        t.price_basis_note = Some(
            "HODL/IL from first→last node USD marks on read. Full token-basket HODL needs DB valuation snapshots (refresh chain-history after rotations)."
                .to_string(),
        );
        t.interpretation.il_vs_initial_hodl_caption_pl =
            "Benchmark IL vs HODL: wartość LP na końcu łańcucha minus HODL przybliżony z baseline pierwszego węzła (bez pełnego koszyka tokenów ze snapshotów DB)."
                .to_string();
    }
    apply_chain_economic_quality_meta(t, nodes);
}

/// Wire `economic_quality`, `end_nav_source`, and caption suffix from lineage nodes.
pub fn apply_chain_economic_quality_meta(
    totals: &mut PositionStreamPnLResponse,
    nodes: &[PositionStreamLineageNode],
) {
    let end_src = headline_end_nav_source(nodes);
    let end_label = chain_end_nav_source_str(end_src);
    let quality = infer_chain_economic_quality(totals, nodes, end_src);
    totals.end_nav_source = Some(end_label.to_string());
    totals.economic_quality = Some(quality.to_string());
    append_economic_quality_caption_suffix(totals, quality, end_label);
}

/// End NAV source for quality tags — closed nodes ignore lifted `current_value_usd` (would read as live).
fn lineage_end_nav_source_for_quality(
    n: &PositionStreamLineageNode,
) -> (Decimal, ChainEndNavSource) {
    if n.closed_ts_utc.is_some() {
        if let Some(end) =
            positive_usd_from_chain_history_column(n.chain_history_end_value_usd.as_deref())
        {
            return (end, ChainEndNavSource::MaterializedEnd);
        }
        if let Some(end) =
            positive_usd_from_chain_history_column(n.lifecycle_close_nav_usd.as_deref())
        {
            return (end, ChainEndNavSource::LifecycleCloseAmounts);
        }
        if n.baseline_value_usd > Decimal::ZERO {
            let est = n.baseline_value_usd + n.fees_collected_usd + n.realized_cashflow_usd
                - n.tx_fees_usd;
            if est > Decimal::ZERO {
                return (est, ChainEndNavSource::CloseEstimate);
            }
        }
        return (Decimal::ZERO, ChainEndNavSource::Missing);
    }
    lineage_node_end_nav_with_source(n)
}

fn headline_end_nav_source(nodes: &[PositionStreamLineageNode]) -> ChainEndNavSource {
    for n in nodes.iter().rev() {
        let (v, src) = lineage_end_nav_source_for_quality(n);
        if v > Decimal::ZERO {
            return src;
        }
    }
    ChainEndNavSource::Missing
}

pub fn chain_end_nav_source_str(src: ChainEndNavSource) -> &'static str {
    match src {
        ChainEndNavSource::LiveCurrent => "live_current",
        ChainEndNavSource::MaterializedEnd => "materialized_end",
        ChainEndNavSource::LifecycleCloseAmounts => "lifecycle_close_amounts",
        ChainEndNavSource::CloseEstimate => "close_estimate",
        ChainEndNavSource::Missing => "missing",
    }
}

fn node_mark_is_exact(q: Option<&str>) -> bool {
    q.is_some_and(|s| s.eq_ignore_ascii_case("exact"))
}

fn node_mark_is_soft(q: Option<&str>) -> bool {
    q.is_some_and(|s| {
        let t = s.trim();
        !t.is_empty() && !t.eq_ignore_ascii_case("exact")
    })
}

fn infer_chain_economic_quality(
    totals: &PositionStreamPnLResponse,
    nodes: &[PositionStreamLineageNode],
    end_src: ChainEndNavSource,
) -> &'static str {
    if nodes.is_empty() {
        return "degraded";
    }
    let baseline = totals.baseline_value_usd;
    if baseline.is_zero()
        || matches!(
            totals.valuation_price_time_kind.as_str(),
            "node_fallback_unavailable" | "unavailable"
        )
    {
        return "degraded";
    }
    if matches!(end_src, ChainEndNavSource::Missing) {
        return "degraded";
    }
    let hodl_degraded =
        totals.hodl_value_usd.is_zero() || totals.hodl_value_usd < baseline * Decimal::new(8, 1);

    let mut exact_marks = 0u32;
    let mut soft_marks = 0u32;
    for n in nodes {
        if node_mark_is_exact(n.baseline_valuation_quality.as_deref()) {
            exact_marks += 1;
        } else if node_mark_is_soft(n.baseline_valuation_quality.as_deref()) {
            soft_marks += 1;
        }
        if node_mark_is_exact(n.current_valuation_quality.as_deref()) {
            exact_marks += 1;
        } else if node_mark_is_soft(n.current_valuation_quality.as_deref()) {
            soft_marks += 1;
        }
    }

    if matches!(end_src, ChainEndNavSource::CloseEstimate) {
        return "estimated";
    }
    if matches!(
        end_src,
        ChainEndNavSource::LifecycleCloseAmounts | ChainEndNavSource::MaterializedEnd
    ) {
        if exact_marks > 0 && soft_marks > 0 {
            return "mixed";
        }
        return "estimated";
    }
    if exact_marks > 0 && soft_marks > 0 {
        return "mixed";
    }
    if matches!(end_src, ChainEndNavSource::LiveCurrent)
        && node_mark_is_exact(
            nodes
                .first()
                .and_then(|n| n.baseline_valuation_quality.as_deref()),
        )
        && !hodl_degraded
    {
        return "exact";
    }
    if hodl_degraded {
        return "degraded";
    }
    "mixed"
}

fn append_economic_quality_caption_suffix(
    totals: &mut PositionStreamPnLResponse,
    quality: &str,
    end_src: &str,
) {
    if totals
        .interpretation
        .economic_net_pnl_caption_pl
        .contains("Jakość:")
    {
        return;
    }
    let suffix = match quality {
        "exact" => " Jakość: exact (live / snapshoty DB).",
        "estimated" => match end_src {
            "close_estimate" => {
                " Jakość: estimated — NAV z estymaty (baseline+fees−tx); net może różnić się od księgowości."
            }
            "lifecycle_close_amounts" => {
                " Jakość: estimated — NAV z kwot close w ledgerze; net może różnić się od księgowości."
            }
            _ => " Jakość: estimated — końcowy NAV bez pełnego live mark.",
        },
        "mixed" => " Jakość: mixed — część węzłów bez pełnej wyceny.",
        "degraded" => {
            " Jakość: degraded — brak pełnego HODL/baseline; nie traktuj dużego ujemnego % jako pewnik."
        }
        _ => "",
    };
    if !suffix.is_empty() {
        totals
            .interpretation
            .economic_net_pnl_caption_pl
            .push_str(suffix);
    }
}

/// Align an existing stream-pnl totals row with lineage node marks (rotation chains).
pub fn sync_chain_economic_totals_from_nodes(
    pnl: &mut PositionStreamPnLResponse,
    nodes: &mut [PositionStreamLineageNode],
) {
    if nodes.is_empty() {
        return;
    }
    lift_node_end_nav_on_nodes(nodes);
    reconcile_stream_pnl_totals_with_nodes(pnl, nodes);
    apply_economic_totals_fields_from_nodes(pnl, nodes, false);
}

/// Refresh persisted/stream totals from per-node marks (e.g. stale `totals_json` on chain-history read).
pub fn refresh_lineage_totals_from_nodes(
    entry: &str,
    totals: &mut Option<PositionStreamPnLResponse>,
    nodes: &mut [PositionStreamLineageNode],
) {
    if nodes.is_empty() {
        return;
    }
    lift_node_end_nav_on_nodes(nodes);
    let first_baseline = nodes
        .first()
        .map(|n| n.baseline_value_usd)
        .unwrap_or(Decimal::ZERO);
    let stale_baseline = totals
        .as_ref()
        .is_some_and(|t| t.baseline_value_usd.is_zero() && first_baseline > Decimal::ZERO);
    if (totals.is_none() || stale_baseline)
        && let Some(fresh) = maybe_compute_totals_from_nodes(
            entry,
            if stale_baseline { &None } else { totals },
            nodes,
            Some(
                "Totals refreshed from lineage nodes on read (materialized totals_json was stale).",
            ),
        )
    {
        *totals = Some(fresh);
    }
    if let Some(t) = totals.as_mut() {
        reconcile_stream_pnl_totals_with_nodes(t, nodes);
        apply_economic_totals_fields_from_nodes(t, nodes, true);
    }
}

#[derive(Debug, Clone, Default)]
struct CloseLedgerAux {
    close_amount_a_raw: Option<u64>,
    close_amount_b_raw: Option<u64>,
    event_price_close_a_usd: Decimal,
    event_price_close_b_usd: Decimal,
}

async fn fetch_close_ledger_aux_best_effort(
    pool: &sqlx::PgPool,
    position_pubkey: &str,
) -> Option<CloseLedgerAux> {
    use serde_json::Value as JsonValue;
    let pos = position_pubkey.trim();
    if pos.is_empty() {
        return None;
    }
    let raw: JsonValue = sqlx::query_scalar::<_, JsonValue>(
        r#"SELECT raw_json FROM position_stream_ledger_rows
           WHERE position_pubkey = $1
             AND event IN ('bot_close_position','position_close')
           ORDER BY ts_utc DESC NULLS LAST
           LIMIT 1"#,
    )
    .bind(pos)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()?;
    let d = raw.get("details").filter(|x| !x.is_null()).unwrap_or(&raw);
    let obj = d.as_object()?;
    let parse_px = |k: &str| -> Decimal {
        obj.get(k)
            .and_then(|v| {
                v.as_f64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
            .and_then(Decimal::from_f64_retain)
            .filter(|x| *x > Decimal::ZERO)
            .unwrap_or(Decimal::ZERO)
    };
    let parse_u64 = |k: &str| -> Option<u64> {
        obj.get(k).and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_i64().and_then(|i| u64::try_from(i).ok()))
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
    };
    Some(CloseLedgerAux {
        close_amount_a_raw: parse_u64("close_amount_a_raw"),
        close_amount_b_raw: parse_u64("close_amount_b_raw"),
        event_price_close_a_usd: parse_px("event_price_a_usd"),
        event_price_close_b_usd: parse_px("event_price_b_usd"),
    })
}

fn lifecycle_close_nav_column_string(v: Option<Decimal>) -> Option<String> {
    v.filter(|d| *d > Decimal::ZERO)
        .map(|d| d.round_dp(12).normalize().to_string())
}

/// Fill `lifecycle_close_nav_usd` from latest close ledger row (`close_amount_*_raw` × event spot).
pub async fn enrich_nodes_lifecycle_close_nav_from_ledger(
    state: &crate::state::AppState,
    pool: &sqlx::PgPool,
    nodes: &mut [PositionStreamLineageNode],
) {
    use crate::services::position_stream_lineage::fetch_mint_decimals_best_effort;
    use solana_sdk::pubkey::Pubkey;
    use std::str::FromStr;

    for node in nodes.iter_mut() {
        if node.closed_ts_utc.is_none() {
            continue;
        }
        if node
            .lifecycle_close_nav_usd
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .and_then(|s| s.parse::<Decimal>().ok())
            .is_some_and(|d| d > Decimal::ZERO)
        {
            continue;
        }
        let Some(aux) =
            fetch_close_ledger_aux_best_effort(pool, node.position_address.as_str()).await
        else {
            continue;
        };
        let (Some(raw_a), Some(raw_b)) = (aux.close_amount_a_raw, aux.close_amount_b_raw) else {
            continue;
        };
        let (Some(ma), Some(mb)) = (node.token_mint_a.as_deref(), node.token_mint_b.as_deref())
        else {
            continue;
        };
        let Ok(pk_a) = Pubkey::from_str(ma.trim()) else {
            continue;
        };
        let Ok(pk_b) = Pubkey::from_str(mb.trim()) else {
            continue;
        };
        let Some(dec_a) = fetch_mint_decimals_best_effort(state.provider.as_ref(), &pk_a).await
        else {
            continue;
        };
        let Some(dec_b) = fetch_mint_decimals_best_effort(state.provider.as_ref(), &pk_b).await
        else {
            continue;
        };
        let Some(nav) = close_nav_usd_from_raw_amounts_and_prices(
            raw_a,
            raw_b,
            dec_a,
            dec_b,
            aux.event_price_close_a_usd,
            aux.event_price_close_b_usd,
        ) else {
            continue;
        };
        node.lifecycle_close_nav_usd = lifecycle_close_nav_column_string(Some(nav));
        if node.current_value_usd.is_zero() {
            node.current_value_usd = nav;
            node.current_valuation_quality = Some("lifecycle_close_amounts".to_string());
            node.net_pnl_usd =
                nav + node.realized_cashflow_usd - node.baseline_value_usd - node.tx_fees_usd;
            if !node.baseline_value_usd.is_zero() {
                node.net_pnl_pct = node.net_pnl_usd / node.baseline_value_usd;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{PositionStreamPnLResponse, StreamPnLInterpretation};
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn mk_node(addr: &str, baseline: Decimal, current: Decimal) -> PositionStreamLineageNode {
        PositionStreamLineageNode {
            position_address: addr.to_string(),
            token_a_label: None,
            token_b_label: None,
            token_mint_a: None,
            token_mint_b: None,
            opened_ts_utc: None,
            closed_ts_utc: None,
            baseline_value_usd: baseline,
            baseline_valuation_quality: None,
            current_value_usd: current,
            current_valuation_quality: None,
            tx_fee_lamports: 0,
            tx_fees_usd: Decimal::ZERO,
            fees_collected_usd: Decimal::ZERO,
            fees_collected_token_a_ui: None,
            fees_collected_token_b_ui: None,
            fees_collected_token_a_raw: None,
            fees_collected_token_b_raw: None,
            collect_events: 0,
            realized_cashflow_usd: Decimal::ZERO,
            net_pnl_usd: Decimal::ZERO,
            net_pnl_pct: Decimal::ZERO,
            note: None,
            collect_zero_diagnostics: None,
            chain_history_start_value_usd: None,
            chain_history_end_value_usd: None,
            chain_history_current_value_usd: None,
            chain_history_pool_address: None,
            chain_history_tick_lower_open: None,
            chain_history_tick_upper_open: None,
            chain_history_event_spot_token_a_usd_open: None,
            chain_history_event_spot_token_a_usd_close: None,
            lifecycle_close_nav_usd: None,
        }
    }

    #[test]
    fn lineage_node_end_nav_prefers_lifecycle_close_nav_over_estimate() {
        let mut n = mk_node("PDA", Decimal::from_str("9.901").unwrap(), Decimal::ZERO);
        n.closed_ts_utc = Some("2026-05-21T20:00:00Z".to_string());
        n.fees_collected_usd = Decimal::from_str("0.032").unwrap();
        n.tx_fees_usd = Decimal::from_str("0.0035").unwrap();
        n.lifecycle_close_nav_usd = Some("9.932".to_string());
        let (nav, src) = lineage_node_end_nav_with_source(&n);
        assert_eq!(src, ChainEndNavSource::LifecycleCloseAmounts);
        assert!(nav > Decimal::from_str("9.92").unwrap());
        let est = n.baseline_value_usd + n.fees_collected_usd - n.tx_fees_usd;
        assert!(nav > est);
    }

    #[test]
    fn close_nav_usd_from_raw_amounts_usdc_close_leg() {
        let nav = close_nav_usd_from_raw_amounts_and_prices(
            0,
            9_931_867,
            9,
            6,
            Decimal::ZERO,
            Decimal::from_str("1").unwrap(),
        )
        .expect("nav");
        assert!(nav > Decimal::from_str("9.92").unwrap());
        assert!(nav < Decimal::from_str("9.94").unwrap());
    }

    #[test]
    fn chain_headline_end_nav_uses_close_estimate_when_current_zero() {
        let mut n = mk_node("PDA", Decimal::from_str("9.901").unwrap(), Decimal::ZERO);
        n.closed_ts_utc = Some("2026-05-21T20:00:00Z".to_string());
        n.fees_collected_usd = Decimal::from_str("0.032").unwrap();
        n.tx_fees_usd = Decimal::from_str("0.0035").unwrap();
        assert!(lineage_node_end_nav_usd(&n) > Decimal::from_str("9.92").unwrap());
        assert_eq!(
            chain_headline_end_nav_usd(std::slice::from_ref(&n)),
            lineage_node_end_nav_usd(&n)
        );
    }

    #[test]
    fn refresh_lineage_totals_repairs_zero_current_closed_chain_net_pnl() {
        let mut totals = Some(PositionStreamPnLResponse {
            position_address: "PDA".to_string(),
            baseline_ts_utc: None,
            current_ts_utc: None,
            baseline_value_usd: Decimal::from_str("9.901").unwrap(),
            current_value_usd: Decimal::ZERO,
            hodl_value_usd: Decimal::from_str("9.901").unwrap(),
            il_usd: Decimal::ZERO,
            il_pct: Decimal::ZERO,
            clean_il_usd: Decimal::ZERO,
            clean_il_pct: Decimal::ZERO,
            realized_lp_fees_usd: Decimal::from_str("0.032").unwrap(),
            uncollected_lp_fees_usd: Decimal::ZERO,
            lp_fees_total_usd: Decimal::from_str("0.032").unwrap(),
            lp_vs_hodl_with_fees_usd: Decimal::from_str("0.032").unwrap(),
            lp_vs_hodl_with_fees_pct: Decimal::ZERO,
            valuation_price_time_kind: "live_price".to_string(),
            price_basis_note: None,
            tx_fees_usd: Decimal::from_str("0.0035").unwrap(),
            realized_cashflow_usd: Decimal::ZERO,
            net_pnl_usd: Decimal::from_str("-9.905").unwrap(),
            net_pnl_pct: Decimal::from_str("-1").unwrap(),
            economic_quality: None,
            end_nav_source: None,
            interpretation: StreamPnLInterpretation {
                economic_net_pnl_caption_pl: String::new(),
                il_vs_initial_hodl_caption_pl: String::new(),
            },
            note: None,
        });
        let mut nodes = vec![mk_node(
            "PDA",
            Decimal::from_str("9.901").unwrap(),
            Decimal::ZERO,
        )];
        nodes[0].closed_ts_utc = Some("2026-05-21T20:00:00Z".to_string());
        nodes[0].fees_collected_usd = Decimal::from_str("0.032").unwrap();
        nodes[0].tx_fees_usd = Decimal::from_str("0.0035").unwrap();
        refresh_lineage_totals_from_nodes("PDA", &mut totals, &mut nodes);
        let t = totals.as_ref().expect("totals");
        assert_eq!(t.economic_quality.as_deref(), Some("estimated"));
        assert_eq!(t.end_nav_source.as_deref(), Some("close_estimate"));
        assert!(t.current_value_usd > Decimal::from_str("9.90").unwrap());
        assert!(t.net_pnl_usd > Decimal::from_str("-0.05").unwrap());
        assert!(t.net_pnl_usd < Decimal::from_str("0.10").unwrap());
        assert!(t.net_pnl_pct > Decimal::from_str("-0.05").unwrap());
        assert!(t.net_pnl_pct < Decimal::from_str("0.02").unwrap());
    }

    #[test]
    fn refresh_lineage_totals_repairs_stale_chain_history_meta_baseline_zero() {
        let mut totals = Some(PositionStreamPnLResponse {
            position_address: "HySR".to_string(),
            baseline_ts_utc: None,
            current_ts_utc: None,
            baseline_value_usd: Decimal::ZERO,
            current_value_usd: Decimal::from_str("9.95054476807043").unwrap(),
            hodl_value_usd: Decimal::ZERO,
            il_usd: Decimal::ZERO,
            il_pct: Decimal::ZERO,
            clean_il_usd: Decimal::ZERO,
            clean_il_pct: Decimal::ZERO,
            realized_lp_fees_usd: Decimal::ZERO,
            uncollected_lp_fees_usd: Decimal::ZERO,
            lp_fees_total_usd: Decimal::ZERO,
            lp_vs_hodl_with_fees_usd: Decimal::ZERO,
            lp_vs_hodl_with_fees_pct: Decimal::ZERO,
            valuation_price_time_kind: "node_fallback_unavailable".to_string(),
            price_basis_note: None,
            tx_fees_usd: Decimal::ZERO,
            realized_cashflow_usd: Decimal::ZERO,
            net_pnl_usd: Decimal::from_str("9.95054476807043").unwrap(),
            net_pnl_pct: Decimal::ZERO,
            economic_quality: None,
            end_nav_source: None,
            interpretation: StreamPnLInterpretation {
                economic_net_pnl_caption_pl: String::new(),
                il_vs_initial_hodl_caption_pl: String::new(),
            },
            note: Some("No valuation snapshots yet; totals computed best-effort from lineage nodes (IL/HODL unavailable).".to_string()),
        });
        let mut nodes = vec![
            mk_node(
                "At6",
                Decimal::from_str("9.973205210329806").unwrap(),
                Decimal::from_str("10.003062304519507").unwrap(),
            ),
            mk_node(
                "HySR",
                Decimal::from_str("9.948260832969006").unwrap(),
                Decimal::from_str("9.94637160185368").unwrap(),
            ),
        ];
        refresh_lineage_totals_from_nodes("HySR", &mut totals, &mut nodes);
        let t = totals.as_ref().expect("totals");
        assert!(t.baseline_value_usd > Decimal::from_str("9.9").unwrap());
        assert!(t.hodl_value_usd > Decimal::from_str("9.9").unwrap());
        assert!(t.net_pnl_usd.abs() < Decimal::from_str("0.5").unwrap());
        assert!(!t.il_usd.is_zero() || !t.clean_il_usd.is_zero());
    }

    #[test]
    fn reconcile_stream_pnl_totals_with_nodes_repairs_degraded_hodl_and_stale_current() {
        let mut totals = PositionStreamPnLResponse {
            position_address: "PDA".to_string(),
            baseline_ts_utc: Some("2026-05-20T20:49:54Z".to_string()),
            current_ts_utc: Some("2026-05-20T20:49:54Z".to_string()),
            baseline_value_usd: Decimal::from_str("10.004").unwrap(),
            current_value_usd: Decimal::from_str("10.004").unwrap(),
            hodl_value_usd: Decimal::from_str("4.859").unwrap(),
            il_usd: Decimal::from_str("5.145").unwrap(),
            il_pct: Decimal::ONE,
            clean_il_usd: Decimal::from_str("5.145").unwrap(),
            clean_il_pct: Decimal::ONE,
            realized_lp_fees_usd: Decimal::ZERO,
            uncollected_lp_fees_usd: Decimal::ZERO,
            lp_fees_total_usd: Decimal::ZERO,
            lp_vs_hodl_with_fees_usd: Decimal::from_str("5.145").unwrap(),
            lp_vs_hodl_with_fees_pct: Decimal::ONE,
            valuation_price_time_kind: "live_price".to_string(),
            price_basis_note: None,
            tx_fees_usd: Decimal::ZERO,
            realized_cashflow_usd: Decimal::ZERO,
            net_pnl_usd: Decimal::ZERO,
            net_pnl_pct: Decimal::ZERO,
            economic_quality: None,
            end_nav_source: None,
            interpretation: StreamPnLInterpretation {
                economic_net_pnl_caption_pl: String::new(),
                il_vs_initial_hodl_caption_pl: String::new(),
            },
            note: None,
        };
        let mut node = mk_node(
            "PDA",
            Decimal::from_str("10.004").unwrap(),
            Decimal::from_str("10.011").unwrap(),
        );
        node.opened_ts_utc = Some("2026-05-20T20:49:54Z".to_string());
        reconcile_stream_pnl_totals_with_nodes(&mut totals, std::slice::from_ref(&node));
        assert_eq!(totals.hodl_value_usd, Decimal::from_str("10.004").unwrap());
        assert_eq!(
            totals.current_value_usd,
            Decimal::from_str("10.011").unwrap()
        );
        assert_eq!(totals.net_pnl_usd, Decimal::from_str("0.007").unwrap());
    }

    fn usd(d: Decimal) -> String {
        d.round_dp(6).normalize().to_string()
    }

    fn headline(t: &PositionStreamPnLResponse) -> serde_json::Value {
        serde_json::json!({
            "baseline_value_usd": usd(t.baseline_value_usd),
            "end_nav_usd": usd(t.current_value_usd),
            "end_nav_source": t.end_nav_source,
            "economic_quality": t.economic_quality,
            "hodl_value_usd": usd(t.hodl_value_usd),
            "il_usd": usd(t.il_usd),
            "clean_il_usd": usd(t.clean_il_usd),
            "realized_lp_fees_usd": usd(t.realized_lp_fees_usd),
            "uncollected_lp_fees_usd": usd(t.uncollected_lp_fees_usd),
            "lp_fees_total_usd": usd(t.lp_fees_total_usd),
            "lp_vs_hodl_with_fees_usd": usd(t.lp_vs_hodl_with_fees_usd),
            "realized_cashflow_usd": usd(t.realized_cashflow_usd),
            "tx_fees_usd": usd(t.tx_fees_usd),
            "net_pnl_usd": usd(t.net_pnl_usd),
            "net_pnl_pct": usd(t.net_pnl_pct),
        })
    }

    fn golden_9vhky(use_materialized_totals: bool) -> serde_json::Value {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/chain_history_9vhKY.json"
        ))
        .expect("parse chain_history_9vhKY fixture");
        let entry = fixture["position_address"].as_str().expect("entry");
        let mut nodes: Vec<PositionStreamLineageNode> =
            serde_json::from_value(fixture["nodes"].clone()).expect("nodes");
        let mut totals: Option<PositionStreamPnLResponse> = if use_materialized_totals {
            Some(serde_json::from_value(fixture["totals"].clone()).expect("totals"))
        } else {
            None
        };
        assert_eq!(nodes.len(), 22, "fixture shape changed");

        refresh_lineage_totals_from_nodes(entry, &mut totals, &mut nodes);

        let t = totals.as_ref().expect("totals after refresh");
        let per_node: Vec<serde_json::Value> = nodes
            .iter()
            .map(|n| {
                serde_json::json!({
                    "position": n.position_address,
                    "baseline_usd": usd(n.baseline_value_usd),
                    "end_nav_usd": usd(lineage_node_end_nav_usd(n)),
                    "fees_collected_usd": usd(n.fees_collected_usd),
                    "tx_fees_usd": usd(n.tx_fees_usd),
                    "net_pnl_usd": usd(n.net_pnl_usd),
                })
            })
            .collect();
        serde_json::json!({ "headline": headline(t), "nodes": per_node })
    }

    /// B1 golden: chain 9vhKY (22 rotations, public on-chain data snapshot 2026-05-26).
    /// A diff here is an `economic_regression` — review with `cargo insta review` and explain
    /// it in the PR "Golden delta" section; never update just to get CI green.
    #[test]
    fn golden_9vhky_totals_computed_from_nodes() {
        insta::assert_json_snapshot!(golden_9vhky(false));
    }

    /// Same chain, starting from the materialized `totals_json` in the fixture (chain-history read path).
    #[test]
    fn golden_9vhky_totals_refreshed_from_materialized() {
        insta::assert_json_snapshot!(golden_9vhky(true));
    }
}
