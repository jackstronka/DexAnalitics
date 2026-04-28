-- Persistent readiness rows for Backtests/Data Quality (DB-first path with fallback to on-demand JSONL scan)
-- Migration: 006_backtest_data_readiness

CREATE TABLE IF NOT EXISTS backtest_data_readiness_rows (
    pool_id TEXT NOT NULL,
    pool_label TEXT NOT NULL,
    protocol TEXT NOT NULL,
    pool_address TEXT NOT NULL,
    snapshot_variant TEXT NOT NULL,
    cadence_minutes BIGINT NOT NULL,
    rows BIGINT NOT NULL DEFAULT 0,
    oldest_ts_utc TIMESTAMPTZ,
    latest_ts_utc TIMESTAMPTZ,
    oldest_continuous_ts_utc TIMESTAMPTZ,
    max_gap_minutes DOUBLE PRECISION,
    coverage_pct DOUBLE PRECISION,
    max_backtest_hours_hard BIGINT NOT NULL DEFAULT 0,
    max_backtest_hours_recommended BIGINT NOT NULL DEFAULT 0,
    note TEXT,
    computed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (pool_id, snapshot_variant)
);

CREATE INDEX IF NOT EXISTS idx_backtest_data_readiness_rows_computed_at
    ON backtest_data_readiness_rows(computed_at DESC);

CREATE INDEX IF NOT EXISTS idx_backtest_data_readiness_rows_protocol_pool
    ON backtest_data_readiness_rows(protocol, pool_address);
