-- Portfel łańcucha sesji: chain_session_id on PSLR + registry (analytics retention).
-- Do not put semicolons (;) inside SQL string literals here: Database::migrate splits on ;

ALTER TABLE position_stream_ledger_rows
ADD COLUMN IF NOT EXISTS chain_session_id TEXT;

CREATE INDEX IF NOT EXISTS idx_position_stream_ledger_rows_chain_session
ON position_stream_ledger_rows (chain_session_id)
WHERE chain_session_id IS NOT NULL AND TRIM(chain_session_id) <> '';

CREATE TABLE IF NOT EXISTS chain_session_registry (
    chain_session_id TEXT PRIMARY KEY,
    anchor_position TEXT,
    head_position TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    closed_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_chain_session_registry_anchor
ON chain_session_registry (anchor_position)
WHERE anchor_position IS NOT NULL AND TRIM(anchor_position) <> '';
