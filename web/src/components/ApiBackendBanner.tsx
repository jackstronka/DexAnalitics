import { isCancelledError, useQuery } from '@tanstack/react-query'
import { AlertTriangle } from 'lucide-react'
import { getLiveness } from '@/lib/api'

/**
 * Gdy backend nie odpowiada, większość zakładek wygląda na „puste” (Vite proxy `/api` → API_UPSTREAM).
 */
export default function ApiBackendBanner() {
  const q = useQuery({
    queryKey: ['liveness'],
    queryFn: ({ signal }) => getLiveness({ signal }),
    retry: 4,
    retryDelay: (n) => Math.min(750 * 2 ** n, 8000),
  })

  if (q.isPending || q.isSuccess) {
    return null
  }
  if (q.isError && isCancelledError(q.error)) {
    return null
  }

  return (
    <div
      role="alert"
      className="flex items-start gap-3 border-b border-destructive/60 bg-destructive/25 px-6 py-3 text-sm text-destructive-foreground"
    >
      <AlertTriangle className="h-5 w-5 shrink-0 text-destructive mt-0.5" />
      <div className="space-y-1 min-w-0">
        <p className="font-medium">Backend API nie odpowiada (Vite proxy: /api → API_UPSTREAM).</p>
        <p className="text-muted-foreground text-xs leading-relaxed">
          Uruchom API przed panelem, np.{' '}
          <code className="rounded bg-muted px-1 py-0.5 text-[11px]">cargo run --bin clmm-lp-api</code> albo podwójne
          kliknięcie <code className="rounded bg-muted px-1 py-0.5 text-[11px]">Start-Dashboard.bat</code> w korzeniu
          repozytorium. Upewnij się, że Vite proxy celuje w poprawny port: ustaw{' '}
          <code className="rounded bg-muted px-1 py-0.5 text-[11px]">API_PORT</code> (np. 8081) albo{' '}
          <code className="rounded bg-muted px-1 py-0.5 text-[11px]">API_UPSTREAM</code> (np. http://127.0.0.1:8081).
        </p>
        {q.error instanceof Error && <p className="text-xs font-mono break-all">{q.error.message}</p>}
      </div>
    </div>
  )
}
