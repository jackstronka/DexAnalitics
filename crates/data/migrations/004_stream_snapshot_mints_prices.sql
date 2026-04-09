-- Persist pool leg mints + prices used for valuation snapshots (for stable HODL/IL)
-- Migration: 004_stream_snapshot_mints_prices

ALTER TABLE position_stream_valuation_snapshots
    ADD COLUMN IF NOT EXISTS token_mint_a TEXT,
    ADD COLUMN IF NOT EXISTS token_mint_b TEXT,
    ADD COLUMN IF NOT EXISTS price_a_usd NUMERIC,
    ADD COLUMN IF NOT EXISTS price_b_usd NUMERIC;

CREATE INDEX IF NOT EXISTS idx_position_stream_valuation_snapshots_mints
    ON position_stream_valuation_snapshots(token_mint_a, token_mint_b);

