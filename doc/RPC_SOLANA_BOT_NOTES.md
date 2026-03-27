# Solana RPC — notatka pod bota Orca (mainnet / produkcja)

**keywords:** rpc, solana, mainnet, devnet, fallback, helius, alchemy, quicknode, drpc, free tier, orca, whirlpool, bot, rebalance

**Cel:** zebrać decyzje i fakty z dyskusji (2026-03), żeby wrócić do wyboru endpointów bez przepisywania czatu.

## Orca a RPC

- Whirlpool to **program na Solanie**. Transakcje i odczyt kont idą przez **Solana JSON-RPC**.
- **Orca nie zastępuje RPC** „własnym połączeniem do łańcucha”. REST Orcy (`api.orca.so`, itd.) to **metadane / listy pooli** — nie zamiennik `sendTransaction` / `getAccount`.
- SDK (`orca_whirlpools`) przyjmuje normalny **`RpcClient`** → wybierasz **URL Solany**.

## Publiczny vs płatny / „własny”

- **`https://api.mainnet-beta.solana.com`** — darmowy, **współdzielony**, limity, **bez SLA**. Dobry jako **fallback** lub dev/test.
- **„Własny RPC” w praktyce** = najczęściej **dedykowany URL od dostawcy** (API key), nie self-hostowanie pełnego węzła (to osobna, ciężka infrastruktura).
- **Dwa URL-e w repo:** `SOLANA_RPC_URL` + `SOLANA_RPC_FALLBACK_URLS` (lista po przecinku). **Nie mieszać clusterów** (mainnet tylko z mainnetem).

## Profil botu (nasze założenia)

- Rebalanse **przy triggerze**, nie HFT.
- **Kilka minut opóźnienia** akceptowalne → niższe wymagania co do RPS niż przy agresywnym market-maku; nadal **niezawodny** `sendTransaction` przy triggerze jest ważny.
- **Dwie alternatywy** (primary + fallback lub drugi provider) — sensowny standard.

## Free tier — kogo porównać (limity zmieniają się → zawsze strona pricing)

Orientacyjnie w **porównaniach branżowych** często wymienia się hojne limity **CU/miesiąc** na free u m.in. **Alchemy**, **dRPC**; **QuickNode** — kredyty + RPS na free; **Helius** — dokumentacja podaje m.in. ~10 RPS na RPC na free (osobno np. `sendTransaction`).

**Źródła do weryfikacji przed wyborem:**

- [Helius — pricing / rate limits](https://www.helius.dev/pricing) — [docs rate limits](https://www.helius.dev/docs/billing/rate-limits)
- [Alchemy — Solana](https://www.alchemy.com/solana)
- [QuickNode — pricing](https://www.quicknode.com/pricing)
- [dRPC — pricing](https://drpc.org/pricing)
- Porównania (nie są „oficjalnym SLA”): np. [Made on Sol — compare RPC](https://madeonsol.com/compare-rpc), [dRPC — top providers](https://drpc.org/blog/top-solana-rpc-providers/)

**Co sprawdzić w cenniku:** miesięczny limit (CU / credits), **RPS**, osobny limit na **`sendTransaction`**, ewentualny limit `getProgramAccounts` (bot LP zwykle go nie spamuje).

## Sugerowany układ (do wdrożenia później)

1. **Primary:** jeden dostawca z free lub płatnym tierem po testach obciążenia.
2. **Fallback:** drugi provider **albo** publiczny `api.mainnet-beta.solana.com`.
3. Metryki własne: loguj `latency`, `429`, timeouty per endpoint — po tygodniu masz **swoją** statystykę (publicznych „dashboardów uptime” dla całej sieci nie ma).

## Linki w repo

- Konfiguracja RPC: `crates/protocols/src/rpc/config.rs` (`SOLANA_RPC_URL`, `SOLANA_RPC_FALLBACK_URLS`).
- Runbook Orca (operacje): `doc/ORCA_RUNBOOK.md`.
