/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** Solana wallet pubkey (base58) shown as the pinned dev/test operator wallet in the UI. */
  readonly VITE_DEV_WALLET_PUBKEY?: string
  /** Same value as API `CLMM_CHAIN_HISTORY_REFRESH_SECRET` when set; sent as `Authorization: Bearer` for chain-history refresh from the web UI. */
  readonly VITE_CHAIN_HISTORY_REFRESH_SECRET?: string
}
