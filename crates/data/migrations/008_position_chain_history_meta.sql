-- Materialized chain-history: metrics_mode dimension + meta row for totals (read path).
-- Idempotent with prior 007: drop old uniqueness, add (anchor, seq, mode) uniqueness, meta table.

ALTER TABLE position_chain_history_nodes
    ADD COLUMN IF NOT EXISTS metrics_mode TEXT NOT NULL DEFAULT 'live';

ALTER TABLE position_chain_history_nodes
    DROP CONSTRAINT IF EXISTS uq_position_chain_history_anchor_seq;

ALTER TABLE position_chain_history_nodes
    DROP CONSTRAINT IF EXISTS uq_position_chain_history_anchor_position;

ALTER TABLE position_chain_history_nodes
    DROP CONSTRAINT IF EXISTS uq_position_chain_history_anchor_seq_mode;

ALTER TABLE position_chain_history_nodes
    DROP CONSTRAINT IF EXISTS uq_position_chain_history_anchor_position_mode;

ALTER TABLE position_chain_history_nodes
    ADD CONSTRAINT uq_position_chain_history_anchor_seq_mode
        UNIQUE (chain_anchor_pubkey, chain_seq, metrics_mode);

ALTER TABLE position_chain_history_nodes
    ADD CONSTRAINT uq_position_chain_history_anchor_position_mode
        UNIQUE (chain_anchor_pubkey, position_pubkey, metrics_mode);

CREATE TABLE IF NOT EXISTS position_chain_history_meta (
    chain_anchor_pubkey TEXT NOT NULL,
    metrics_mode TEXT NOT NULL DEFAULT 'live',
    entry_position_address TEXT NOT NULL,
    chain_json JSONB NOT NULL,
    totals_json JSONB,
    chain_cost_summary_json JSONB,
    note TEXT,
    materialized_ts_utc TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (chain_anchor_pubkey, metrics_mode)
);

CREATE INDEX IF NOT EXISTS idx_position_chain_history_meta_anchor
    ON position_chain_history_meta(chain_anchor_pubkey);
