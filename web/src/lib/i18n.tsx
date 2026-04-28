import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from 'react'

export type Locale = 'pl' | 'en'

type Dictionary = Record<string, string>

const LS_LOCALE_KEY = 'clmm.locale'

const pl: Dictionary = {
  'nav.dashboard': 'Dashboard',
  'nav.wallet': 'Portfel',
  'nav.swap': 'Swap',
  'nav.positions': 'Pozycje',
  'nav.closed': 'Zamknięte',
  'nav.strategies': 'Strategie',
  'nav.backtests': 'Backtesty',
  'nav.pools': 'Pule',
  'nav.scripts': 'Skrypty',
  'nav.logs': 'Logi',
  'nav.botActivity': 'Aktywność bota',
  'nav.settings': 'Ustawienia',
  'layout.apiPending': 'API…',
  'layout.apiMissing': 'Brak API',
  'layout.apiOk': 'API OK',
  'layout.langLabel': 'Język',
  'wallet.title': 'Portfel',
  'wallet.subtitle':
    'Dwie warstwy danych: (1) on-chain saldo dla wybranego portfela (read-only RPC), (2) agregaty USD z /analytics/portfolio (monitor pozycji) — mogą być 0, jeśli monitor nie ma pozycji.',
  'wallet.walletsTitle': 'Portfele (pliki keypair na hoście API)',
  'wallet.walletsHint':
    'Lista portfeli pochodzi z plików keypair na hoście API (jeden plik JSON = jeden wpis). Saldo i Orca na dole dotyczą wybranego przyciskiem portfela — nie musi to być ten sam co przy transferze.',
  'wallet.noWalletsHint':
    'Brak portfeli w skonfigurowanym katalogu na hoście API. Dodaj pliki *.json albo utwórz portfel poniżej i zrestartuj API jeśli trzeba.',
  'wallet.transferFrom': 'Z portfela',
  'wallet.transferTo': 'Do portfela / odbiorcy',
  'wallet.transferToCustom': 'Inny adres (wklej poniżej)',
  'wallet.recipientPubkey': 'Adres odbiorcy (pubkey)',
  'wallet.lamportsLabel': 'Lamports',
  'wallet.lamportsTooltip':
    '1 SOL = 1 000 000 000 lamportów — najmniejsza jednostka natywnego SOL na Solanie.',
  'wallet.solPreview': 'SOL (podgląd)',
  'wallet.quickAmounts': 'Szybkie kwoty',
  'wallet.transferHistory': 'Ostatnie transfery (lokalny log)',
  'wallet.currentWallet': 'Aktualny portfel',
  'wallet.copy': 'Kopiuj',
  'wallet.onChainTitle': 'Saldo on-chain (read-only)',
  'wallet.loading': 'Ładowanie salda…',
  'wallet.tokenCount': 'Tokeny SPL',
  'wallet.showZeros': 'Pokaż zera',
  'wallet.hideZeros': 'Ukryj zera',
  'wallet.noTokens': 'Brak tokenów SPL (lub chwilowy problem RPC).',
  'wallet.totalValue': 'Total value',
  'wallet.netPnl': 'Net PnL',
  'wallet.feesUsd': 'Fees (USD)',
  'wallet.ilAvg': 'IL (avg %)',
  'wallet.openPositions': 'Otwarte pozycje',
  'wallet.allPositions': 'Wszystkie pozycje',
  'positions.title': 'Pozycje',
  'positions.refresh': 'Odśwież',
  'positions.openPosition': 'Otwórz pozycję',
  'positions.monitoredTitle': 'Monitorowane pozycje (API)',
  'positions.loading': 'Ładowanie...',
  'positions.notLinked': 'Niepodpięta',
  'positions.checking': 'Sprawdzanie…',
  'positions.pendingTitle': 'Zamknięte przez bota, oczekują na reopen',
  'positions.pendingEmpty': 'Brak oczekujących close->open.',
  'positions.remove': 'Usuń',
  'positions.removing': 'Usuwanie...',
  'positions.onchainTitle': 'Pozycje Orca on-chain (RPC)',
  'positions.loadOnchain': 'Wczytaj on-chain',
  'swap.title': 'Swap',
  'swap.modeOrca': 'Tryb Orca',
  'swap.modeJupiter': 'Tryb Jupiter',
  'swap.orcaDescription':
    'Tryb Orca: wykonuje swap ExactIn w wybranej puli Whirlpool przez backend (ten sam mechanizm co „swap-before-open”).',
  'swap.jupiterDescription': 'Jupiter deep-link ma prefill mintów i kwoty (ExactIn).',
  'swap.swapNow': 'Swap teraz (Orca pool)',
  'swap.swapping': 'Swapowanie…',
  'backtests.title': 'Backtesty',
  'backtests.subtitle': 'Pełne porównanie strategii i parametrów dla okien 24/48/72/96h.',
  'backtests.timeWindows': 'Okna czasowe (h)',
  'backtests.pairs': 'Pary',
  'backtests.strategies': 'Strategie (rodziny + parametry)',
  'backtests.qualifying': 'Strategie spełniające target',
  'backtests.liquidityRegime': 'Reżim płynności (orientacyjnie)',
  'backtests.targetPerWindow': 'Target per okno',
  'backtests.globalTop': 'Globalny ranking TOP (cały run)',
  'positionDetail.title': 'Szczegóły pozycji',
  'positionDetail.info': 'Informacje o pozycji',
  'positionDetail.performance': 'Wyniki',
  'positionDetail.automation': 'Automatyzacja strategii (ta pozycja)',
}

const en: Dictionary = {
  'nav.dashboard': 'Dashboard',
  'nav.wallet': 'Wallet',
  'nav.swap': 'Swap',
  'nav.positions': 'Positions',
  'nav.closed': 'Closed',
  'nav.strategies': 'Strategies',
  'nav.backtests': 'Backtests',
  'nav.pools': 'Pools',
  'nav.scripts': 'Scripts',
  'nav.logs': 'Logs',
  'nav.botActivity': 'Bot activity',
  'nav.settings': 'Settings',
  'layout.apiPending': 'API…',
  'layout.apiMissing': 'API unavailable',
  'layout.apiOk': 'API OK',
  'layout.langLabel': 'Language',
  'wallet.title': 'Wallet',
  'wallet.subtitle':
    'Two data layers: (1) on-chain balance for selected wallet (read-only RPC), (2) USD aggregates from /analytics/portfolio (position monitor) — may be 0 if monitor has no positions.',
  'wallet.walletsTitle': 'Wallets (keypair files on API host)',
  'wallet.walletsHint':
    'Wallet list comes from keypair files on the API host (one JSON file = one row). Balance/Orca below follow the wallet selected via buttons — it may differ from transfer source.',
  'wallet.noWalletsHint':
    'No wallets in the configured directory on the API host. Add *.json files or create a wallet below and restart the API if needed.',
  'wallet.transferFrom': 'From wallet',
  'wallet.transferTo': 'To wallet / recipient',
  'wallet.transferToCustom': 'Other address (paste below)',
  'wallet.recipientPubkey': 'Recipient pubkey',
  'wallet.lamportsLabel': 'Lamports',
  'wallet.lamportsTooltip':
    '1 SOL = 1,000,000,000 lamports — the smallest unit of native SOL on Solana.',
  'wallet.solPreview': 'SOL (preview)',
  'wallet.quickAmounts': 'Quick amounts',
  'wallet.transferHistory': 'Recent transfers (local log)',
  'wallet.currentWallet': 'Current wallet',
  'wallet.copy': 'Copy',
  'wallet.onChainTitle': 'On-chain balance (read-only)',
  'wallet.loading': 'Loading balance…',
  'wallet.tokenCount': 'SPL tokens',
  'wallet.showZeros': 'Show zero balances',
  'wallet.hideZeros': 'Hide zero balances',
  'wallet.noTokens': 'No SPL tokens found (or temporary RPC issue).',
  'wallet.totalValue': 'Total value',
  'wallet.netPnl': 'Net PnL',
  'wallet.feesUsd': 'Fees (USD)',
  'wallet.ilAvg': 'IL (avg %)',
  'wallet.openPositions': 'Open positions',
  'wallet.allPositions': 'All positions',
  'positions.title': 'Positions',
  'positions.refresh': 'Refresh',
  'positions.openPosition': 'Open position',
  'positions.monitoredTitle': 'Monitored positions (API)',
  'positions.loading': 'Loading...',
  'positions.notLinked': 'Not linked',
  'positions.checking': 'Checking…',
  'positions.pendingTitle': 'Closed by bot, waiting for reopen',
  'positions.pendingEmpty': 'No pending close->open items.',
  'positions.remove': 'Remove',
  'positions.removing': 'Removing...',
  'positions.onchainTitle': 'On-chain Orca positions (RPC)',
  'positions.loadOnchain': 'Load on-chain',
  'swap.title': 'Swap',
  'swap.modeOrca': 'Orca mode',
  'swap.modeJupiter': 'Jupiter mode',
  'swap.orcaDescription':
    'Orca mode: executes ExactIn swap in selected Whirlpool pool through backend (same mechanism as swap-before-open).',
  'swap.jupiterDescription': 'Jupiter deep-link pre-fills token mints and amount (ExactIn).',
  'swap.swapNow': 'Swap now (Orca pool)',
  'swap.swapping': 'Swapping…',
  'backtests.title': 'Backtests',
  'backtests.subtitle': 'Full comparison of strategies and parameters for 24/48/72/96h windows.',
  'backtests.timeWindows': 'Time windows (h)',
  'backtests.pairs': 'Pairs',
  'backtests.strategies': 'Strategies (families + parameters)',
  'backtests.qualifying': 'Strategies meeting target',
  'backtests.liquidityRegime': 'Liquidity regime (approximate)',
  'backtests.targetPerWindow': 'Target per window',
  'backtests.globalTop': 'Global TOP ranking (full run)',
  'positionDetail.title': 'Position details',
  'positionDetail.info': 'Position info',
  'positionDetail.performance': 'Performance',
  'positionDetail.automation': 'Strategy automation (this position)',
}

const dictByLocale: Record<Locale, Dictionary> = { pl, en }

type I18nContextValue = {
  locale: Locale
  setLocale: (next: Locale) => void
  t: (key: string, fallback?: string) => string
}

const I18nContext = createContext<I18nContextValue | null>(null)

export function I18nProvider({ children }: { children: ReactNode }) {
  const [locale, setLocaleState] = useState<Locale>(() => {
    if (typeof window === 'undefined') return 'pl'
    const raw = window.localStorage.getItem(LS_LOCALE_KEY)
    return raw === 'en' ? 'en' : 'pl'
  })

  useEffect(() => {
    if (typeof window === 'undefined') return
    window.localStorage.setItem(LS_LOCALE_KEY, locale)
  }, [locale])

  const value = useMemo<I18nContextValue>(() => {
    const dict = dictByLocale[locale]
    const setLocale = (next: Locale) => setLocaleState(next)
    const t = (key: string, fallback?: string) => dict[key] ?? fallback ?? key
    return { locale, setLocale, t }
  }, [locale])

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>
}

export function useI18n() {
  const ctx = useContext(I18nContext)
  if (!ctx) throw new Error('useI18n must be used within I18nProvider')
  return ctx
}

