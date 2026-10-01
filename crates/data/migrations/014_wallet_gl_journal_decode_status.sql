-- Wallet GL journal: optional decode quality for Phase C (confirmed rows without full deltas).

ALTER TABLE wallet_gl_journal_event
ADD COLUMN IF NOT EXISTS decode_status VARCHAR(32);
