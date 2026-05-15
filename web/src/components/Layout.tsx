import { Outlet, Link, useLocation } from 'react-router-dom'
import { 
  LayoutDashboard, 
  Wallet, 
  ArrowLeftRight,
  Target, 
  Droplets, 
  Settings,
  Activity,
  History,
  Menu,
  X,
  Terminal,
  ScrollText,
  BarChart3,
  Database,
  ClipboardList,
} from 'lucide-react'
import { useState, useEffect } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Button } from '@/components/ui/button'
import ApiBackendBanner from '@/components/ApiBackendBanner'
import DevWalletBar from '@/components/DevWalletBar'
import { closedPositionsListQueryOptions, getHealth } from '@/lib/api'
import { useI18n } from '@/lib/i18n'
import { APP_VERSION_LABEL } from '@/lib/version'
import { connectWebSockets, disconnectWebSockets } from '@/lib/websocket'

function navItemIsActive(pathname: string, href: string): boolean {
  if (href === '/wallet') {
    return pathname === '/wallet' || pathname === '/wallet/'
  }
  return pathname === href || pathname.startsWith(`${href}/`)
}

export default function Layout() {
  const location = useLocation()
  const [sidebarOpen, setSidebarOpen] = useState(false)
  const queryClient = useQueryClient()
  const { locale, setLocale, t } = useI18n()

  const navigation = [
    { key: 'nav.dashboard', href: '/dashboard', icon: LayoutDashboard },
    { key: 'nav.wallet', href: '/wallet', icon: Wallet },
    { key: 'nav.walletLedger', href: '/wallet/ledger', icon: ClipboardList },
    { key: 'nav.swap', href: '/swap', icon: ArrowLeftRight },
    { key: 'nav.positions', href: '/positions', icon: Activity },
    { key: 'nav.closed', href: '/positions/closed', icon: History },
    { key: 'nav.strategies', href: '/strategies', icon: Target },
    { key: 'nav.backtests', href: '/backtests', icon: BarChart3 },
    { key: 'nav.dataQuality', href: '/data-quality', icon: Database },
    { key: 'nav.pools', href: '/pools', icon: Droplets },
    { key: 'nav.scripts', href: '/scripts', icon: Terminal },
    { key: 'nav.logs', href: '/logs', icon: ScrollText },
    { key: 'nav.botActivity', href: '/bot-activity', icon: History },
    { key: 'nav.settings', href: '/settings', icon: Settings },
  ] as const

  const healthQ = useQuery({
    queryKey: ['health'],
    queryFn: getHealth,
    refetchInterval: 30_000,
    retry: 2,
  })

  useEffect(() => {
    connectWebSockets()
    return () => disconnectWebSockets()
  }, [])

  // Warm closed-positions cache after API is up (idle) so /positions/closed opens instantly.
  useEffect(() => {
    if (!healthQ.isSuccess) return
    const prefetchClosed = () => {
      void queryClient.prefetchQuery(closedPositionsListQueryOptions(100, 0, false))
      void queryClient.prefetchQuery(closedPositionsListQueryOptions(100, 0, true))
    }
    const w = globalThis as Window & typeof globalThis
    let idleId: number | undefined
    let timeoutId: ReturnType<typeof setTimeout> | undefined
    if (typeof w.requestIdleCallback === 'function') {
      idleId = w.requestIdleCallback(prefetchClosed, { timeout: 4000 })
    } else {
      timeoutId = setTimeout(prefetchClosed, 1200)
    }
    return () => {
      if (idleId !== undefined && typeof w.cancelIdleCallback === 'function') {
        w.cancelIdleCallback(idleId)
      }
      if (timeoutId !== undefined) clearTimeout(timeoutId)
    }
  }, [healthQ.isSuccess, queryClient])

  return (
    <div className="flex h-screen">
      {/* Mobile sidebar backdrop */}
      {sidebarOpen && (
        <div 
          className="fixed inset-0 z-40 bg-black/50 lg:hidden"
          onClick={() => setSidebarOpen(false)}
        />
      )}

      {/* Sidebar */}
      <aside className={`
        fixed inset-y-0 left-0 z-50 w-64 bg-card border-r transform transition-transform duration-200 ease-in-out
        lg:translate-x-0 lg:static lg:inset-auto
        ${sidebarOpen ? 'translate-x-0' : '-translate-x-full'}
      `}>
        <div className="flex h-16 items-center justify-between px-6 border-b">
          <Link to="/dashboard" className="flex items-center gap-2">
            <Activity className="h-6 w-6 text-primary" />
            <span className="font-bold text-lg">Bociarz LP</span>
          </Link>
          <Button
            variant="ghost"
            size="icon"
            className="lg:hidden"
            onClick={() => setSidebarOpen(false)}
          >
            <X className="h-5 w-5" />
          </Button>
        </div>

        <nav className="flex flex-col gap-1 p-4">
          {navigation.map((item) => {
            const isActive = navItemIsActive(location.pathname, item.href)
            return (
              <Link
                key={item.key}
                to={item.href}
                className={`
                  flex items-center gap-3 rounded-lg px-3 py-2 text-sm font-medium transition-colors
                  ${isActive 
                    ? 'bg-primary text-primary-foreground' 
                    : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground'
                  }
                `}
                onClick={() => setSidebarOpen(false)}
                onMouseEnter={() => {
                  if (item.href !== '/positions/closed') return
                  void queryClient.prefetchQuery(closedPositionsListQueryOptions(100, 0, false))
                  void queryClient.prefetchQuery(closedPositionsListQueryOptions(100, 0, true))
                }}
              >
                <item.icon className="h-5 w-5" />
                {t(item.key)}
              </Link>
            )
          })}
        </nav>

        <div className="absolute bottom-0 left-0 right-0 p-4 border-t">
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <div
              className={`h-2 w-2 rounded-full shrink-0 ${
                healthQ.isSuccess ? 'bg-green-500' : healthQ.isError ? 'bg-destructive' : 'bg-yellow-500'
              }`}
            />
            <span className="leading-tight">
              {healthQ.isPending
                ? t('layout.apiPending')
                : healthQ.isError
                  ? t('layout.apiMissing')
                  : t('layout.apiOk')}
            </span>
          </div>
        </div>
      </aside>

      {/* Main content */}
      <div className="flex-1 flex flex-col overflow-hidden">
        {/* Top bar */}
        <header className="h-16 border-b flex items-center px-6 gap-4">
          <Button
            variant="ghost"
            size="icon"
            className="lg:hidden"
            onClick={() => setSidebarOpen(true)}
          >
            <Menu className="h-5 w-5" />
          </Button>
          <div className="flex-1 flex justify-end min-w-0">
            <DevWalletBar />
          </div>
          <div className="flex items-center gap-2 shrink-0 text-xs text-muted-foreground">
            <span>{t('layout.langLabel')}</span>
            <select
              className="h-8 rounded border bg-background px-2 text-xs text-foreground"
              value={locale}
              onChange={(e) => setLocale(e.target.value === 'en' ? 'en' : 'pl')}
            >
              <option value="pl">PL</option>
              <option value="en">EN</option>
            </select>
          </div>
          <div className="text-sm text-muted-foreground shrink-0">
            {APP_VERSION_LABEL}
          </div>
        </header>

        <ApiBackendBanner />

        {/* Page content */}
        <main className="flex-1 overflow-auto p-6">
          <Outlet />
        </main>
      </div>
    </div>
  )
}
