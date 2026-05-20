-- Wallet GL: append-only journal rows (dual-write with JSONL until cutover).
-- Do not put semicolons (;) inside SQL string literals here: `Database::migrate` splits on `;`.

CREATE TABLE IF NOT EXISTS wallet_gl_journal_event (
    event_id VARCHAR(64) PRIMARY KEY,
    schema_version INTEGER NOT NULL DEFAULT 1,
    ts_utc TIMESTAMPTZ NOT NULL,
    correlation_id VARCHAR(128) NOT NULL,
    status VARCHAR(16) NOT NULL,
    kind VARCHAR(64) NOT NULL,
    owner VARCHAR(64),
    signature VARCHAR(128),
    pool_address VARCHAR(64),
    position_pda VARCHAR(64),
    cost_session_id VARCHAR(128),
    dry_run BOOLEAN NOT NULL DEFAULT false,
    native_lamports_delta VARCHAR(32),
    deltas_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    error TEXT,
    source VARCHAR(128) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_wallet_gl_journal_event_ts ON wallet_gl_journal_event (ts_utc DESC);

CREATE INDEX IF NOT EXISTS idx_wallet_gl_journal_event_owner ON wallet_gl_journal_event (owner);

CREATE INDEX IF NOT EXISTS idx_wallet_gl_journal_event_kind ON wallet_gl_journal_event (kind);

CREATE INDEX IF NOT EXISTS idx_wallet_gl_journal_event_status ON wallet_gl_journal_event (status);
