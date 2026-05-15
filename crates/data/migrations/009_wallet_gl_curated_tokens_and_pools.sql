-- Wallet GL: chart-of-accounts seed for SPL tokens appearing in curated CLMM pairs.
-- Source of pair/mint list: keep in sync with `crates/api/src/handlers/backtests.rs::curated_backtest_pools()`
-- and `tools/orca_curated_mainnet_pools.ps1` (Orca subset).
-- Do not put semicolons (;) inside SQL string literals here: `Database::migrate` splits on `;`.

CREATE TABLE IF NOT EXISTS wallet_gl_token_account (
    mint VARCHAR(64) PRIMARY KEY,
    symbol VARCHAR(32) NOT NULL,
    account_code VARCHAR(96) NOT NULL UNIQUE,
    decimals SMALLINT NOT NULL,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_wallet_gl_token_account_symbol ON wallet_gl_token_account(symbol);

CREATE TABLE IF NOT EXISTS wallet_gl_curated_pool (
    pair_id VARCHAR(64) NOT NULL UNIQUE,
    protocol VARCHAR(32) NOT NULL,
    pool_address VARCHAR(64) PRIMARY KEY,
    label TEXT NOT NULL,
    symbol_a VARCHAR(32) NOT NULL,
    mint_a VARCHAR(64) NOT NULL REFERENCES wallet_gl_token_account(mint) ON DELETE RESTRICT,
    symbol_b VARCHAR(32) NOT NULL,
    mint_b VARCHAR(64) NOT NULL REFERENCES wallet_gl_token_account(mint) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_wallet_gl_curated_pool_protocol ON wallet_gl_curated_pool(protocol);

INSERT INTO wallet_gl_token_account (mint, symbol, account_code, decimals, notes) VALUES
    ('So11111111111111111111111111111111111111112', 'SOL', 'SPL:So11111111111111111111111111111111111111112', 9, 'wSOL mint (Orca token order), curated pairs'),
    ('EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v', 'USDC', 'SPL:EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v', 6, NULL),
    ('Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB', 'USDT', 'SPL:Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB', 6, 'Raydium SOL/USDT curated pool'),
    ('7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs', 'WHETH', 'SPL:7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs', 8, 'Portal whETH'),
    ('cbbtcf3aa214zXHbiAZQwf4122FBYbraNdFqgw4iMij', 'CBBTC', 'SPL:cbbtcf3aa214zXHbiAZQwf4122FBYbraNdFqgw4iMij', 8, 'cbBTC'),
    ('3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh', 'WBTC', 'SPL:3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh', 8, 'Portal WBTC leg')
ON CONFLICT (mint) DO NOTHING;

INSERT INTO wallet_gl_curated_pool (pair_id, protocol, pool_address, label, symbol_a, mint_a, symbol_b, mint_b) VALUES
    ('ORCA_SOL_USDC', 'orca', 'Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE', 'Orca SOL/USDC 0.04%', 'SOL', 'So11111111111111111111111111111111111111112', 'USDC', 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v'),
    ('ORCA_WHETH_SOL', 'orca', 'HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF', 'Orca whETH/SOL 0.05%', 'SOL', 'So11111111111111111111111111111111111111112', 'WHETH', '7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs'),
    ('ORCA_CBBTC_USDC', 'orca', 'HxA6SKW5qA4o12fjVgTpXdq2YnZ5Zv1s7SB4FFomsyLM', 'Orca cbBTC/USDC 0.04%', 'CBBTC', 'cbbtcf3aa214zXHbiAZQwf4122FBYbraNdFqgw4iMij', 'USDC', 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v'),
    ('ORCA_CBBTC_WBTC', 'orca', '4v8ufj8Hj7UvFgtofQJAtzUud5xomwZfEqfCTHZ4wM72', 'Orca cbBTC/WBTC 0.01%', 'CBBTC', 'cbbtcf3aa214zXHbiAZQwf4122FBYbraNdFqgw4iMij', 'WBTC', '3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh'),
    ('RAYDIUM_SOL_USDT', 'raydium', '3nMFwZXwY1s1M5s8vYAHqd4wGs4iSxXE4LRoUMMYqEgF', 'Raydium SOL/USDT 0.01%', 'SOL', 'So11111111111111111111111111111111111111112', 'USDT', 'Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB'),
    ('METEORA_SOL_USDC_S1', 'meteora', 'HTvjzsfX3yU6BUodCjZ5vZkUrAxMDTrBs3CJaq43ashR', 'Meteora SOL/USDC Step1', 'SOL', 'So11111111111111111111111111111111111111112', 'USDC', 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v'),
    ('METEORA_SOL_USDC_S4', 'meteora', '5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6', 'Meteora SOL/USDC Step4', 'SOL', 'So11111111111111111111111111111111111111112', 'USDC', 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v'),
    ('METEORA_SOL_USDC_S10', 'meteora', 'BGm1tav58oGcsQJehL9WXBFXF7D27vZsKefj4xJKD5Y', 'Meteora SOL/USDC Step10', 'SOL', 'So11111111111111111111111111111111111111112', 'USDC', 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v')
ON CONFLICT (pool_address) DO NOTHING;
