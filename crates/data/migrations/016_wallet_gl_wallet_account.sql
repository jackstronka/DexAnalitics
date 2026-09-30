-- Wallet GL Phase D2: global operator wallet account (created on demand; seed documents convention).
-- Do not put semicolons (;) inside SQL string literals here: Database::migrate splits on ;

INSERT INTO wallet_gl_account (account_type, account_code, notes)
VALUES (
    'wallet',
    'WALLET:_convention',
    'Per-owner codes WALLET:{pubkey}, opening import + journal transfer/convert postings'
)
ON CONFLICT (account_code) DO NOTHING;
