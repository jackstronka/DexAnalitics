-- Position stream performance (rebalance lineage + costs/fees from lifecycle ledgers)
-- Migration: 002_position_stream_performance

-- Edges: old position -> new position (typically from IL ledger).
CREATE TABLE IF NOT EXISTS position_stream_edges (
    rebalance_session_id TEXT NOT NULL,
    ts_utc TIMESTAMPTZ,
    old_position TEXT NOT NULL,
    new_position TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT 'il_ledger',
    PRIMARY KEY (rebalance_session_id, old_position, new_position)
);

CREATE INDEX IF NOT EXISTS idx_position_stream_edges_new ON position_stream_edges(new_position);
CREATE INDEX IF NOT EXISTS idx_position_stream_edges_old ON position_stream_edges(old_position);

-- Raw lifecycle/ledger rows we can aggregate later (tx cost + collect deltas).
CREATE TABLE IF NOT EXISTS position_stream_ledger_rows (
    -- Signature is unique when present; may be NULL (e.g. synthetic rows).
    signature TEXT UNIQUE,
    ts_utc TIMESTAMPTZ,
    source TEXT,
    event TEXT,
    rebalance_session_id TEXT,
    position_pubkey TEXT,
    pool_pubkey TEXT,
    tx_fee_lamports BIGINT,
    fee_payer_token_a_delta_ui NUMERIC,
    fee_payer_token_b_delta_ui NUMERIC,
    raw_json JSONB NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_position_stream_ledger_rows_session ON position_stream_ledger_rows(rebalance_session_id);
CREATE INDEX IF NOT EXISTS idx_position_stream_ledger_rows_position ON position_stream_ledger_rows(position_pubkey);
CREATE INDEX IF NOT EXISTS idx_position_stream_ledger_rows_event ON position_stream_ledger_rows(event);

