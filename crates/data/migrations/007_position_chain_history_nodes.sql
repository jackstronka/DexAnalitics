-- Materialized read-model for one rotation chain (UI "Historia pozycji" / lineage table).
-- Rows are populated by a future writer (bot / manual / backfill); API keeps existing stream-lineage path until switched.
-- Migration: 007_position_chain_history_nodes

CREATE TABLE IF NOT EXISTS position_chain_history_nodes (
    id BIGSERIAL PRIMARY KEY,

    -- PDA used when this snapshot was built (e.g. URL param / current head); allows re-materialize per anchor.
    chain_anchor_pubkey TEXT NOT NULL,

    -- 1 = oldest in this anchor's resolved chain, N = newest.
    chain_seq SMALLINT NOT NULL CHECK (chain_seq >= 1),

    -- This row's position NFT.
    position_pubkey TEXT NOT NULL,

    -- Optional: previous PDA in rotation (same semantics as stream lineage old→new).
    predecessor_position_pubkey TEXT,

    pool_address TEXT,

    opened_ts_utc TIMESTAMPTZ,
    closed_ts_utc TIMESTAMPTZ,

    -- Display strings (UI "zakres @ open" / "cena zamknięcia"); structured ticks optional.
    range_label_at_open TEXT,
    tick_lower_open INTEGER,
    tick_upper_open INTEGER,

    close_price_label TEXT,
    event_price_a_usd NUMERIC,
    event_price_b_usd NUMERIC,

    -- USD marks (aligned with lineage columns: start ≈ baseline, end at close, current for still-open).
    start_value_usd NUMERIC,
    end_value_usd NUMERIC,
    current_value_usd NUMERIC,

    principal_delta_usd NUMERIC,

    tx_fee_lamports BIGINT NOT NULL DEFAULT 0,
    tx_fees_usd NUMERIC,

    collect_events INTEGER NOT NULL DEFAULT 0,
    fees_collected_usd NUMERIC,
    fees_token_a_ui NUMERIC,
    fees_token_b_ui NUMERIC,
    fees_token_a_raw BIGINT,
    fees_token_b_raw BIGINT,

    token_mint_a TEXT,
    token_mint_b TEXT,

    realized_cashflow_usd NUMERIC,
    net_pnl_usd NUMERIC,
    net_pnl_pct NUMERIC,

    -- Row format for forward-compatible writers (increment when columns gain new meaning).
    source_version SMALLINT NOT NULL DEFAULT 1,

    -- When this row was last written.
    materialized_ts_utc TIMESTAMPTZ NOT NULL DEFAULT now(),

    -- Optional full copy of API node or lifecycle fragments for audit / replay.
    raw_snapshot JSONB,

    CONSTRAINT uq_position_chain_history_anchor_seq UNIQUE (chain_anchor_pubkey, chain_seq),
    CONSTRAINT uq_position_chain_history_anchor_position UNIQUE (chain_anchor_pubkey, position_pubkey)
);

CREATE INDEX IF NOT EXISTS idx_position_chain_history_anchor
    ON position_chain_history_nodes(chain_anchor_pubkey);

CREATE INDEX IF NOT EXISTS idx_position_chain_history_position
    ON position_chain_history_nodes(position_pubkey);

CREATE INDEX IF NOT EXISTS idx_position_chain_history_pool
    ON position_chain_history_nodes(pool_address);
