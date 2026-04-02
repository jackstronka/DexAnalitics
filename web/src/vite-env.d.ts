/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** Solana wallet pubkey (base58) shown as the pinned dev/test operator wallet in the UI. */
  readonly VITE_DEV_WALLET_PUBKEY?: string
}
