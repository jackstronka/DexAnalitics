-- Wallet GL: SESSION sub-ledger (per rebalance_session_id / cost_session_id), analytics retention.
-- Sessions are never closed or liquidated after manual close — balances kept for analysis.
-- Do not put semicolons (;) inside SQL string literals here: Database::migrate splits on ;

CREATE TABLE IF NOT EXISTS wallet_gl_account (
    id BIGSERIAL PRIMARY KEY,
    account_type VARCHAR(32) NOT NULL,
    account_code VARCHAR(160) NOT NULL UNIQUE,
    owner VARCHAR(64),
    session_id VARCHAR(128),
    position_pda VARCHAR(64),
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_wallet_gl_account_owner ON wallet_gl_account (owner);

CREATE INDEX IF NOT EXISTS idx_wallet_gl_account_session ON wallet_gl_account (session_id)
WHERE session_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_wallet_gl_account_type ON wallet_gl_account (account_type);

CREATE TABLE IF NOT EXISTS wallet_gl_balance (
    account_id BIGINT NOT NULL REFERENCES wallet_gl_account (id) ON DELETE RESTRICT,
    mint VARCHAR(64) NOT NULL,
    amount_raw VARCHAR(48) NOT NULL DEFAULT '0',
    last_event_id VARCHAR(64),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (account_id, mint)
);

CREATE INDEX IF NOT EXISTS idx_wallet_gl_balance_mint ON wallet_gl_balance (mint);

CREATE TABLE IF NOT EXISTS wallet_gl_posting (
    id BIGSERIAL PRIMARY KEY,
    event_id VARCHAR(64) NOT NULL,
    account_id BIGINT NOT NULL REFERENCES wallet_gl_account (id) ON DELETE RESTRICT,
    mint VARCHAR(64) NOT NULL,
    delta_raw VARCHAR(48) NOT NULL,
    kind VARCHAR(64),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_wallet_gl_posting_event ON wallet_gl_posting (event_id);

CREATE INDEX IF NOT EXISTS idx_wallet_gl_posting_account ON wallet_gl_posting (account_id);

CREATE INDEX IF NOT EXISTS idx_wallet_gl_journal_event_cost_session ON wallet_gl_journal_event (cost_session_id)
WHERE cost_session_id IS NOT NULL;
