/**
 * Pinned dev/test wallet pubkey from `VITE_DEV_WALLET_PUBKEY` (see `web/.env.example`).
 * For display and future Phantom / tx flows — does not hold private keys.
 */
export function getDevWalletPubkey(): string | null {
  const raw = import.meta.env.VITE_DEV_WALLET_PUBKEY
  if (typeof raw !== 'string') return null
  const t = raw.trim()
  return t.length > 0 ? t : null
}
