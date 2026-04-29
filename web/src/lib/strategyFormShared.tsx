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
      'Rebalance co ustaloną liczbę minut od ostatniego rebalance’u. Domyślnie działa „po staremu”: zegar tyka niezależnie od tego, czy jesteś w zakresie. Opcjonalnie możesz włączyć wariant „tylko gdy OOR”.',
  },
  threshold: {
    title: 'Próg (cena)',
    body:
      'Rebalance przy wyjściu poza zakres albo gdy cena oddali się od środka zakresu o więcej niż podany próg %. Szerokość zakresu dotyczy nowego pasma po rebalance.',
  },
  bollinger: {
    title: 'Bollinger',
    body:
      'Rebalance interwałowy na bazie pasm Bollingera liczonych z live punktów ceny z pętli strategii (window, k). To nie jest tryb snapshot 5m/10m z Backtests. Nowy zakres pozycji jest ustawiany na [lower, upper] z bieżącego okna.',
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
  last_candle: {
    title: 'Last candle',
    body:
      'Trigger zdarzeniowy (OOR): strategia sprawdza wyjście poza zakres i wtedy wyznacza nowe granice z ostatniej zamkniętej świecy (low/high). Gdy brak świecy lub świeca jest płaska, fallback do pasma z Range Width %.',
  },
  last_candle_periodic: {
    title: 'Last candle (periodic)',
    body:
      'Trigger czasowy (interwał): rebalance co N minut, niezależnie od tego czy pozycja jest in-range/OOR. Granice są z ostatniej zamkniętej świecy (low/high), a przy braku świecy lub płaskiej świecy fallback do pasma z Range Width %.',
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
  bollinger: {
    rangeWidth: false,
    maxIl: false,
    rebalanceThreshold: false,
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
  last_candle: {
    rangeWidth: true,
    maxIl: false,
    rebalanceThreshold: false,
    minInterval: true,
  },
  last_candle_periodic: {
    rangeWidth: true,
    maxIl: false,
    rebalanceThreshold: false,
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
  bollingerWindow:
    'Bollinger (live): liczba ostatnich punktów ceny używana do wyliczenia SMA i odchylenia standardowego. Mniejsze okno = szybsza reakcja (więcej szumu), większe = gładsze pasma. Punkt = jedna próbka z pętli strategii (nie snapshot 5m/10m).',
  bollingerK:
    'Bollinger: mnożnik odchylenia standardowego (k) dla pasm (SMA ± k·σ). Niższe k = ciaśniejsze pasmo i częstsze rebalance; wyższe k = szersze pasmo i rzadsze rebalance. Start: 2.0 (agresywniej 1.5, spokojniej 2.5).',
  rebalanceThresholdIl:
    'Tryb limit IL: gdy |IL| przekroczy ten próg (i spełnione są reguły odstępu), rekomendowany jest rebalance.',
  minIntervalPeriodic:
    'Tryb czasowy: rebalance okresowy może wykonać się po upływie N minut od poprzedniego rebalance. Licznik resetuje się po każdej wykonanej akcji.',
  minIntervalOther:
    'Minimalna liczba minut między rebalance’ami tam, gdzie silnik egzekwuje odstęp (np. przy limit IL).',
  candleSeconds:
    'Rozmiar świecy dla strategii last_candle / last_candle_periodic w minutach (np. 15, 30, 60). Używana jest ostatnia w pełni zamknięta świeca.',
  periodicRequiresOor:
    'Gdy włączone: rebalance okresowy wykona się tylko wtedy, gdy w chwili wyzwolenia (po upływie interwału) pozycja jest poza zakresem (OOR). Gdy wyłączone: rebalance okresowy wykona się po interwale niezależnie od in-range/OOR.',
  rebalanceOnRangeExitImmediately:
    'Gdy włączone, wyjście poza zakres może od razu wywołać rebalance (close+open). Gdy wyłączone, OOR jest tylko sygnałem i rebalance czeka na interwał minimalny. Uwaga: to ustawienie nie dotyczy strategii Periodic oraz Last candle (periodic).',
  retouchOffsetPct:
    'Tylko RetouchShift: przesuwa cały nowy zakres po retouch o X% ceny. 0 = krawędź dotyka ceny OOR; dodatni przesuwa zakres w prawo, ujemny w lewo.',
  dryRun:
    'Gdy włączone, executor nie wykonuje prawdziwych transakcji on-chain (tryb bezpieczny).',
  autoExecute:
    'Gdy włączone i dry-run wyłączone, wymaga portfela API (KEYPAIR_PATH) — realne tx.',
  autoStart:
    'Jeśli włączone, API uruchomi tę strategię po starcie procesu. Globalny autostart jest domyślnie włączony (gdy zmienna `CLMM_STRATEGY_AUTOSTART_ON_BOOT` nie jest ustawiona). Ustaw ją na `0` / `false`, żeby wyłączyć autostart wszystkich strategii na bootcie.',
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
    minRebalanceIntervalMinutes: number | ''
    retouchOffsetPct: number | ''
    candleMinutes: number | ''
    bollingerWindow: number | ''
    bollingerK: number | ''
    periodicRequiresOutOfRange: boolean
    rebalanceOnRangeExitImmediately: boolean
    autoStart: boolean
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
  if (strategyType === 'retouch_shift' && state.retouchOffsetPct !== '') {
    p.retouch_offset_pct = Number(state.retouchOffsetPct)
  }
  if (e.minInterval && state.minRebalanceIntervalMinutes !== '') {
    const n = Number(state.minRebalanceIntervalMinutes)
    if (Number.isFinite(n) && n >= 0) {
      p.min_rebalance_interval_minutes = Math.floor(n)
    }
  }
  if ((strategyType === 'last_candle' || strategyType === 'last_candle_periodic') && state.candleMinutes !== '') {
    const n = Number(state.candleMinutes)
    if (Number.isFinite(n) && n > 0) {
      p.candle_seconds = Math.max(60, Math.floor(n * 60))
    }
  }
  if (strategyType === 'bollinger') {
    if (state.bollingerWindow !== '') {
      const n = Number(state.bollingerWindow)
      if (Number.isFinite(n) && n >= 2) {
        p.bollinger_window = Math.floor(n)
      }
    }
    if (state.bollingerK !== '') {
      const n = Number(state.bollingerK)
      if (Number.isFinite(n) && n > 0) {
        p.bollinger_k = n
      }
    }
  }

  // Optional execution semantics toggles.
  // We always send them so the behavior is explicit and visible in the saved config.
  p.periodic_requires_out_of_range = state.periodicRequiresOutOfRange
  if (strategyType !== 'periodic' && strategyType !== 'last_candle_periodic') {
    p.rebalance_on_range_exit_immediately = state.rebalanceOnRangeExitImmediately
  }
  p.auto_start = state.autoStart

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
