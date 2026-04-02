import { Copy } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { getDevWalletPubkey } from '@/lib/devWallet'
import { shortenAddress } from '@/lib/utils'
import { useToast } from '@/hooks/use-toast'

/**
 * Shows pinned dev wallet from `VITE_DEV_WALLET_PUBKEY` when set.
 */
export default function DevWalletBar() {
  const { toast } = useToast()
  const pk = getDevWalletPubkey()
  if (!pk) return null

  return (
    <div className="flex items-center gap-2 rounded-md border border-primary/30 bg-primary/10 px-3 py-1.5 text-xs">
      <span className="text-muted-foreground shrink-0">Dev wallet</span>
      <code className="font-mono truncate max-w-[min(48ch,50vw)]" title={pk}>
        {shortenAddress(pk, 8)}
      </code>
      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="h-7 w-7 shrink-0"
        onClick={() => {
          void navigator.clipboard.writeText(pk)
          toast({ title: 'Copied', description: 'Wallet pubkey copied.' })
        }}
        aria-label="Copy wallet pubkey"
      >
        <Copy className="h-3.5 w-3.5" />
      </Button>
    </div>
  )
}
