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
} from 'lucide-react'
import { useState, useEffect } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Button } from '@/components/ui/button'
import ApiBackendBanner from '@/components/ApiBackendBanner'
import DevWalletBar from '@/components/DevWalletBar'
import { getHealth } from '@/lib/api'
import { connectWebSockets, disconnectWebSockets } from '@/lib/websocket'

const navigation = [
  { name: 'Dashboard', href: '/dashboard', icon: LayoutDashboard },
  { name: 'Wallet', href: '/wallet', icon: Wallet },
  { name: 'Swap', href: '/swap', icon: ArrowLeftRight },
  { name: 'Positions', href: '/positions', icon: Activity },
  { name: 'Closed', href: '/positions/closed', icon: History },
  { name: 'Strategies', href: '/strategies', icon: Target },
  { name: 'Pools', href: '/pools', icon: Droplets },
  { name: 'Scripts', href: '/scripts', icon: Terminal },
  { name: 'Logs', href: '/logs', icon: ScrollText },
  { name: 'Bot activity', href: '/bot-activity', icon: History },
  { name: 'Settings', href: '/settings', icon: Settings },
]

export default function Layout() {
  const location = useLocation()
  const [sidebarOpen, setSidebarOpen] = useState(false)

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
            const isActive = location.pathname.startsWith(item.href)
            return (
              <Link
                key={item.name}
                to={item.href}
                className={`
                  flex items-center gap-3 rounded-lg px-3 py-2 text-sm font-medium transition-colors
                  ${isActive 
                    ? 'bg-primary text-primary-foreground' 
                    : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground'
                  }
                `}
                onClick={() => setSidebarOpen(false)}
              >
                <item.icon className="h-5 w-5" />
                {item.name}
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
                ? 'API…'
                : healthQ.isError
                  ? 'Brak API'
                  : 'API OK'}
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
          <div className="text-sm text-muted-foreground shrink-0">
            v0.1.1-alpha.2
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
