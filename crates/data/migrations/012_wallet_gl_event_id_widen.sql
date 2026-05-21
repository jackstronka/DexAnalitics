-- Solana tx signatures are ~88 chars; SESSION postings use event_id = lifecycle:{signature}.
-- Widen from VARCHAR(64) so backfill / lifecycle GL posting does not fail.
-- Do not put semicolons (;) inside SQL string literals here: Database::migrate splits on ;

ALTER TABLE wallet_gl_posting
    ALTER COLUMN event_id TYPE VARCHAR(128);

ALTER TABLE wallet_gl_balance
    ALTER COLUMN last_event_id TYPE VARCHAR(128);
