import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom'
import { Toaster } from '@/components/ui/toaster'
import Layout from '@/components/Layout'
import Dashboard from '@/pages/Dashboard'
import Positions from '@/pages/Positions'
import PositionCreate from '@/pages/PositionCreate'
import PositionDetail from '@/pages/PositionDetail'
import ClosedPositions from '@/pages/ClosedPositions'
import ClosedPositionDetail from '@/pages/ClosedPositionDetail'
import PageErrorBoundary from '@/components/PageErrorBoundary'
import Strategies from '@/pages/Strategies'
import StrategyDetail from '@/pages/StrategyDetail'
import StrategyCreate from '@/pages/StrategyCreate'
import StrategyEdit from '@/pages/StrategyEdit'
import Pools from '@/pages/Pools'
import PoolDetail from '@/pages/PoolDetail'
import Settings from '@/pages/Settings'
import BotActivity from '@/pages/BotActivity'
import Logs from '@/pages/Logs'
import Scripts from '@/pages/Scripts'
import Wallet from '@/pages/Wallet'
import WalletLedger from '@/pages/WalletLedger'
import Swap from '@/pages/Swap'
import Backtests from '@/pages/Backtests'
import DataQuality from '@/pages/DataQuality'
import ExperimentLaunch from '@/pages/ExperimentLaunch'

function App() {
  return (
    <BrowserRouter>
      <div className="min-h-screen bg-background">
        <Routes>
          <Route path="/" element={<Layout />}>
            <Route index element={<Navigate to="/dashboard" replace />} />
            <Route path="dashboard" element={<Dashboard />} />
            <Route path="wallet" element={<Wallet />} />
            <Route path="wallet/ledger" element={<WalletLedger />} />
            <Route path="swap" element={<Swap />} />
            <Route path="scripts" element={<Scripts />} />
            <Route path="positions" element={<Positions />} />
            <Route path="positions/new" element={<PositionCreate />} />
            <Route path="positions/closed" element={<ClosedPositions />} />
            <Route
              path="positions/closed/:address"
              element={
                <PageErrorBoundary title="Closed position details crashed while rendering">
                  <ClosedPositionDetail />
                </PageErrorBoundary>
              }
            />
            <Route
              path="positions/:address"
              element={
                <PageErrorBoundary title="Position details crashed while rendering">
                  <PositionDetail />
                </PageErrorBoundary>
              }
            />
            <Route path="strategies" element={<Strategies />} />
            <Route path="strategies/new" element={<StrategyCreate />} />
            <Route path="strategies/:id/edit" element={<StrategyEdit />} />
            <Route path="strategies/:id" element={<StrategyDetail />} />
            <Route path="pools" element={<Pools />} />
            <Route path="pools/:address" element={<PoolDetail />} />
            <Route path="settings" element={<Settings />} />
            <Route path="logs" element={<Logs />} />
            <Route path="bot-activity" element={<BotActivity />} />
            <Route path="backtests" element={<Backtests />} />
            <Route path="experiments/new" element={<ExperimentLaunch />} />
            <Route path="data-quality" element={<DataQuality />} />
          </Route>
        </Routes>
        <Toaster />
      </div>
    </BrowserRouter>
  )
}

export default App
