-- Wallet GL Phase C4: system account for accumulated network tx fees (lamports as WSOL raw).
-- Do not put semicolons (;) inside SQL string literals here: Database::migrate splits on ;

INSERT INTO wallet_gl_account (account_type, account_code, notes)
VALUES (
    'system',
    'TX_FEE',
    'Accumulated Solana network fees, balance mint WSOL raw lamports per lifecycle tx_fee_lamports'
)
ON CONFLICT (account_code) DO NOTHING;
