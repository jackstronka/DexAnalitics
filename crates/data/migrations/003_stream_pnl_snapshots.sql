-- Stream PnL/IL support: valuation snapshots + generic fee payer token deltas in ledger rows
-- Migration: 003_stream_pnl_snapshots

-- Extend ledger rows with generic mint->delta map (JSONB) for fee payer.
ALTER TABLE position_stream_ledger_rows
    ADD COLUMN IF NOT EXISTS fee_payer_token_deltas JSONB;

CREATE INDEX IF NOT EXISTS idx_position_stream_ledger_rows_token_deltas
    ON position_stream_ledger_rows USING GIN (fee_payer_token_deltas);

-- Valuation snapshots: persist "mark" and baseline basket for stream PnL / IL across rotates.
CREATE TABLE IF NOT EXISTS position_stream_valuation_snapshots (
    position_pubkey TEXT NOT NULL,
    ts_utc TIMESTAMPTZ NOT NULL,
    pool_pubkey TEXT,
    value_usd NUMERIC NOT NULL,
    amount_a_ui NUMERIC,
    amount_b_ui NUMERIC,
    fees_usd NUMERIC,
    price_source TEXT,
    raw_json JSONB NOT NULL,
    PRIMARY KEY (position_pubkey, ts_utc)
);

CREATE INDEX IF NOT EXISTS idx_position_stream_valuation_snapshots_ts
    ON position_stream_valuation_snapshots(ts_utc DESC);
CREATE INDEX IF NOT EXISTS idx_position_stream_valuation_snapshots_position
    ON position_stream_valuation_snapshots(position_pubkey);

