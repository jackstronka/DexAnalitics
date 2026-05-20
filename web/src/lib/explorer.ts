/** Solana mainnet explorer links (dev UI). */

export function solscanTxUrl(signature: string): string {
  return `https://solscan.io/tx/${encodeURIComponent(signature.trim())}`
}

export function solscanAccountUrl(address: string): string {
  return `https://solscan.io/account/${encodeURIComponent(address.trim())}`
}
