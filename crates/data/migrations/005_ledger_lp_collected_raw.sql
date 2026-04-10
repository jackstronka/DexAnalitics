-- LP fees collected per pool leg (raw token amounts) from position fee_owed_* at collect time.
-- Written by bot `collect_fees` → `orca_position_lifecycle.jsonl` and ingested into position_stream_ledger_rows.

ALTER TABLE position_stream_ledger_rows
    ADD COLUMN IF NOT EXISTS lp_collected_token_a_raw BIGINT,
    ADD COLUMN IF NOT EXISTS lp_collected_token_b_raw BIGINT;
