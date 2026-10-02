# B6 fixture — synthetic CHAIN portfolio ledger

**Source:** constructed in `chain_portfolio::tests::golden_chain_portfolio_ledger_events_and_footer` (no on-chain dump).
**Date:** 2026-10-02
**What it represents:** one SOL/USDC cycle at $150 / $1.

| Piece | Designed numbers |
| ----- | ---------------- |
| Pre-open wallet | 0.1 SOL + 5 USDC → start **20.00** USD |
| Open to pool | 0.05 SOL + 4 USDC → **11.50** USD out |
| Collect | 0.001 SOL + 2 USDC → **2.15** USD in |
| Close principal + LP | 0.048 SOL + 3.5 USDC + 0.0005 SOL LP → **10.775** USD in |
| Collected fees (collect + close LP) | 1.5M SOL raw + 2M USDC raw → **2.225** USD |
| Tx fee (3× 5000 lamports) | **0.00075** USD each |
| Footer leftover wallet | 0.05 SOL + 1 USDC → **8.50** USD |

A snapshot diff is an `economic_regression`. Update only locally (`INSTA_UPDATE=always cargo test -p clmm-lp-api golden_chain_portfolio_ledger`) with a “Golden delta” section in the PR.
