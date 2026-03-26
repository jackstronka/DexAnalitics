# Orca: external implementations (patterns worth copying)

## Why this doc exists

Orca CLMM integrations are deceptively complex: the transaction builder needs the *right* instruction variants, the *right* token/position account model (Token-2022 vs legacy), and the builder has to be careful about rent/refunds (position mint/data/ATA + tick arrays).

This file captures the most relevant patterns from well-known open implementations so we can align our production-like devnet bot roadmap with proven approaches.

## Hummingbot Gateway (Orca CLMM connector)

Hummingbot wraps Orca operations behind standardized connector endpoints, split into:

- read/quote style endpoints (e.g. pool/position info and quotes)
- execute endpoints that submit the on-chain transaction (e.g. open/close/add/remove/collect)

Key production lessons shown by their Orca pages + linked issues/PRs:

1. **Use on-chain whirlpool pricing for execution decisions**
   - They explicitly improved “on-chain pool pricing” so rebalancing decisions do not rely on potentially stale Orca API data.

2. **Token-2022 instruction variants for full rent recovery**
   - Gateway had a bug where rent fees were not returned fully when closing Orca CLMM positions.
   - The root cause was using `openPositionWithMetadataIx` in the open-position path.
   - The fix was to use the Token-2022 position instruction variant (mentioned in the linked PR).

3. **Be transparent about tick-array rent**
   - There is an open feature request to include tick-array rent in `positionRent` accounting when tick arrays are initialized as part of the transaction.
   - Tick rent should be refundable on close, but builders and accounting layers must track it explicitly.

4. **Expose slippage and operational configs**
   - Their connector configuration includes default slippage percentage and operational parameters (e.g. max hops for swaps).

References:
- [Hummingbot Gateway Orca page](https://hummingbot.org/exchanges/gateway/orca/)
- [Hummingbot issue: rent fee recovery bug](https://github.com/hummingbot/gateway/issues/584)
- [Hummingbot issue: include tick array rent](https://github.com/hummingbot/gateway/issues/602)

## Orca Whirlpools “LP Repositioning Bot” (example)

Orca’s example repositioning bot demonstrates the control-loop and reliability patterns expected from an automated rebalancer:

1. **Repositioning logic based on deviation threshold**
   - Track the position center price vs the whirlpool price.
   - When deviation crosses a user threshold, close out-of-range and reopen with a range adjusted around the current price.

2. **Keep the initial range width**
   - The new range reuses the original width so the strategy behavior is consistent across rebalances.

3. **Atomic close + open (when possible)**
   - Review discussion explicitly calls out executing close and open atomically to avoid “manual reopening” when open fails.

4. **Dynamic priority fee via simulation + caps**
   - The bot dynamically estimates priority fees by simulating transactions to measure compute usage, then applies user-defined caps.

5. **Retry for transient RPC/transaction failures**
   - Uses retries to handle “transaction expired” and similar transient errors.

Reference:
- [Orca whirlpools PR #558: example repositioning bot](https://github.com/orca-so/whirlpools/pull/558)

## What we should copy into our bot on devnet (actionable mapping)

Even though we currently focus on building a production-like *flow* (unsigned tx build -> client signing -> policy-gated submit), the examples above suggest these near-term upgrades:

1. **Instruction correctness upgrade**
   - If we aim for accurate rent accounting and minimal user surprise, the open-position builder must use the Token-2022 instruction variant (or an equivalent approach) that matches how close expects to refund rent.

2. **Rent transparency & risk-aware budgeting**
   - Add tick-array rent accounting into our internal “cost/quality” reports (especially around open-position).

3. **On-chain pricing for execution decisions**
   - For rebalancing decisions, prefer our existing on-chain WhirlpoolReader-derived pricing over REST-derived pricing.

4. **Atomic close/open transaction mode**
   - When building `/tx/*/build` payloads for bot operations, support an “atomic close+open” mode if the instruction builder + tx size constraints allow it.

5. **Priority fee policy**
   - Add a “priority fee estimation” step (simulate -> set compute budget / caps) before submit, and log the derived values for audit.

## Follow-up questions (for the next planning session)

- Which position model will we target for devnet production-like mode first: legacy NFT or Token-2022?
- Do we want our `/tx/*/build` to produce “single tx” close/open sequences when possible, or keep it “one op per tx”?
- Should `min-out/slippage` be computed server-side (via on-chain reads) or fully client-driven?

