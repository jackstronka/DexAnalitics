import { HelpCircle } from 'lucide-react'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import type { StrategyParameters, StrategyType } from '@/lib/api'

export const STRATEGY_COPY: Record<
  StrategyType,
  { title: string; body: string }
> = {
  static_range: {
    title: 'Statyczna',
    body:
      'Tryb statyczny: szerokość zakresu % jest zapisana w strategii i służy m.in. do podpowiedzi ticków przy Open Position (wokół bieżącej ceny). Automatyzacja rebalance z logiki strategii jest ograniczona; zbieranie fee może działać osobno.',
  },
  periodic: {
    title: 'Okresowa',
    body:
      'Rebalance co ustaloną liczbę godzin od ostatniego rebalance’u. Domyślnie działa „po staremu”: zegar tyka niezależnie od tego, czy jesteś w zakresie. Opcjonalnie możesz włączyć wariant „tylko gdy OOR”.',
  },
  threshold: {
    title: 'Próg (cena)',
    body:
      'Rebalance przy wyjściu poza zakres albo gdy cena oddali się od środka zakresu o więcej niż podany próg %. Szerokość zakresu dotyczy nowego pasma po rebalance.',
  },
  il_limit: {
    title: 'Limit IL',
    body:
      'Sygnały z impermanent loss: przekroczenie maks. IL może skończyć się rekomendacją zamknięcia; rebalance przy progu IL lub przy wyjściu z zakresu — z uwzględnieniem minimalnego odstępu między akcjami.',
  },
  oor_recenter: {
    title: 'OOR recenter',
    body:
      'Poza zakresem: pełne recentrowanie pasma na bieżącą cenę (jak w backteście oor_recenter); inna geometria niż retouch.',
  },
  retouch_shift: {
    title: 'Retouch shift',
    body:
      'Poza zakresem: przesuwanie jednej krawędzi (retouch), szerokość pasma zachowana; szczegóły w doc (RetouchShift).',
  },
}

export type FieldKey = 'rangeWidth' | 'maxIl' | 'rebalanceThreshold' | 'minInterval'

export const FIELD_ENABLED: Record<StrategyType, Record<FieldKey, boolean>> = {
  static_range: {
    rangeWidth: true,
    maxIl: false,
    rebalanceThreshold: false,
    minInterval: false,
  },
  periodic: {
    rangeWidth: true,
    maxIl: false,
    rebalanceThreshold: false,
    minInterval: true,
  },
  threshold: {
    rangeWidth: true,
    maxIl: false,
    rebalanceThreshold: true,
    minInterval: true,
  },
  il_limit: {
    rangeWidth: true,
    maxIl: true,
    rebalanceThreshold: true,
    minInterval: true,
  },
  oor_recenter: {
    rangeWidth: true,
    maxIl: false,
    rebalanceThreshold: true,
    minInterval: true,
  },
  retouch_shift: {
    rangeWidth: true,
    maxIl: false,
    rebalanceThreshold: true,
    minInterval: true,
  },
}

export const TOOLTIPS = {
  name: 'Nazwa widoczna w panelu — tylko identyfikacja; sama z siebie nie zmienia zachowania on-chain.',
  description:
    'Opcjonalna notatka. Endpoint tworzenia strategii może obecnie nie zapisywać tego pola — sprawdź backend, jeśli ma być trwałe.',
  strategyType:
    'Wybór logiki executora: kiedy rebalance, zamknięcie lub utrzymanie pozycji.',
  rangeWidth:
    'Szerokość nowego zakresu po rebalance, w procentach (dokładna interpretacja ticków/ceny jest w silniku wykonania).',
  maxIl:
    'Tryb limit IL: gdy |IL| przekroczy ten %, silnik może zarekomendować zamknięcie (próg „close”).',
  rebalanceThresholdThreshold:
    'Przy pozycji w zakresie: rebalance, gdy cena oddali się od środka pasma o co najmniej ten %. Poza zakresem rebalance może nastąpić wcześniej.',
  rebalanceThresholdIl:
    'Tryb limit IL: gdy |IL| przekroczy ten próg (i spełnione są reguły odstępu), rekomendowany jest rebalance.',
  minIntervalPeriodic:
    'Co tyle godzin od ostatniego rebalance’u wykonywana jest kolejna akcja okresowa; w backendzie powiązane z interwałem periodycznym.',
  minIntervalOther:
    'Minimalna liczba godzin między rebalance’ami tam, gdzie silnik egzekwuje odstęp (np. przy limit IL).',
  periodicRequiresOor:
    'Gdy włączone, periodic wykonuje rebalance tylko jeśli pozycja jest poza zakresem (OOR). Gdy wyłączone — „po staremu”: rebalance dokładnie co N godzin niezależnie od in-range.',
  rebalanceOnRangeExitImmediately:
    'Gdy włączone — „po staremu”: wyjście poza zakres może od razu wywołać rebalance (close+open). Gdy wyłączone — OOR jest tylko sygnałem; rebalance czeka na minimalny interwał.',
  dryRun:
    'Gdy włączone, executor nie wykonuje prawdziwych transakcji on-chain (tryb bezpieczny).',
  autoExecute:
    'Gdy włączone i dry-run wyłączone, wymaga portfela API (KEYPAIR_PATH) — realne tx.',
} as const

export function FieldLabel({
  htmlFor,
  label,
  tooltip,
}: {
  htmlFor?: string
  label: string
  tooltip: string
}) {
  return (
    <div className="flex items-center gap-1.5 mb-1">
      <label htmlFor={htmlFor} className="text-sm font-medium">
        {label}
      </label>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            className="inline-flex rounded-sm text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-label={`Pomoc: ${label}`}
          >
            <HelpCircle className="h-3.5 w-3.5 shrink-0" />
          </button>
        </TooltipTrigger>
        <TooltipContent className="max-w-xs leading-snug">{tooltip}</TooltipContent>
      </Tooltip>
    </div>
  )
}

export function buildParameters(
  strategyType: StrategyType,
  state: {
    rangeWidthPct: number | ''
    maxIlPct: number | ''
    rebalanceThresholdPct: number | ''
    minRebalanceIntervalHours: number | ''
    periodicRequiresOutOfRange: boolean
    rebalanceOnRangeExitImmediately: boolean
  },
): StrategyParameters {
  const e = FIELD_ENABLED[strategyType] ?? FIELD_ENABLED.static_range
  const p: StrategyParameters = {}
  if (e.rangeWidth && state.rangeWidthPct !== '') {
    p.range_width_pct = Number(state.rangeWidthPct)
  }
  if (e.maxIl && state.maxIlPct !== '') {
    p.max_il_pct = Number(state.maxIlPct)
  }
  if (e.rebalanceThreshold && state.rebalanceThresholdPct !== '') {
    p.rebalance_threshold_pct = Number(state.rebalanceThresholdPct)
  }
  if (e.minInterval && state.minRebalanceIntervalHours !== '') {
    p.min_rebalance_interval_hours = Number(state.minRebalanceIntervalHours)
  }

  // Optional execution semantics toggles.
  // We always send them so the behavior is explicit and visible in the saved config.
  p.periodic_requires_out_of_range = state.periodicRequiresOutOfRange
  p.rebalance_on_range_exit_immediately = state.rebalanceOnRangeExitImmediately

  return p
}

/** When `rangeWidth` is enabled for this type, `range_width_pct` must be set (0–100]. */
export function isRangeWidthSatisfied(
  strategyType: StrategyType,
  rangeWidthPct: number | '',
): boolean {
  const e = FIELD_ENABLED[strategyType] ?? FIELD_ENABLED.static_range
  if (!e.rangeWidth) {
    return true
  }
  if (rangeWidthPct === '') {
    return false
  }
  const n = Number(rangeWidthPct)
  return Number.isFinite(n) && n > 0 && n <= 100
}
