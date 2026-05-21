import { describe, expect, it, vi } from 'vitest'
import {
  launchExperiment,
  shouldBlockOpenAfterConfirmedSwap,
  type ExperimentArmLaunchSpec,
  type LaunchExperimentDeps,
} from '@/lib/experimentLaunch'

function arm(overrides: Partial<ExperimentArmLaunchSpec> = {}): ExperimentArmLaunchSpec {
  return {
    armId: 'arm-1',
    enabled: true,
    tick_lower: -100,
    tick_upper: 100,
    amount_a: 1_000,
    amount_b: 2_000,
    createStrategy: {
      name: 'exp-threshold',
      strategy_type: 'threshold',
      parameters: { range_width_pct: 5 },
    },
    ...overrides,
  }
}

function deps(overrides: Partial<LaunchExperimentDeps> = {}): LaunchExperimentDeps {
  return {
    generateSessionId: () => 'session-1',
    generateBatchId: () => 'batch-1',
    createStrategy: vi.fn(async () => ({ id: 'strat-1' })),
    openPosition: vi.fn(async () => ({
      position_pda: 'pda-1',
      cost_session_id: 'session-1',
    })),
    swapBeforeOpen: vi.fn(async () => ({
      swap_signature: 'swap-sig',
      cost_session_id: 'swap-session',
    })),
    getFundingStatus: vi.fn(async () => ({
      shortA: false,
      shortB: false,
      deficitA: 0,
      deficitB: 0,
    })),
    ...overrides,
  }
}

describe('experimentLaunch', () => {
  describe('shouldBlockOpenAfterConfirmedSwap', () => {
    it('blocks when either leg is still short (BUG-20260512-05 guard)', () => {
      expect(shouldBlockOpenAfterConfirmedSwap({ shortA: true, shortB: false })).toBe(true)
      expect(shouldBlockOpenAfterConfirmedSwap({ shortA: false, shortB: true })).toBe(true)
      expect(shouldBlockOpenAfterConfirmedSwap({ shortA: false, shortB: false })).toBe(false)
    })
  })

  describe('launchExperiment', () => {
    it('opens each enabled arm sequentially after strategy create', async () => {
      const d = deps()
      const result = await launchExperiment(
        {
          poolAddress: 'pool-1',
          arms: [arm(), arm({ armId: 'arm-2' })],
        },
        d,
      )
      expect(result.arms).toHaveLength(2)
      expect(result.arms.every((a) => a.status === 'opened')).toBe(true)
      expect(d.createStrategy).toHaveBeenCalledTimes(2)
      expect(d.openPosition).toHaveBeenCalledTimes(2)
    })

    it('runs shared swap before opens when plan provided', async () => {
      const d = deps()
      await launchExperiment(
        {
          poolAddress: 'pool-1',
          arms: [arm()],
          sharedSwap: {
            specified_mint: 'mint-a',
            amount_in: 1000,
            direction: 'a_to_b',
            label: 'A→B',
          },
        },
        d,
      )
      expect(d.swapBeforeOpen).toHaveBeenCalledTimes(1)
      expect(d.getFundingStatus).toHaveBeenCalled()
    })

    it('does not open when funding still short after confirmed swap', async () => {
      const d = deps({
        getFundingStatus: vi.fn(async () => ({
          shortA: false,
          shortB: true,
          deficitA: 0,
          deficitB: 2.5,
        })),
      })
      const result = await launchExperiment(
        {
          poolAddress: 'pool-1',
          arms: [arm()],
          sharedSwap: {
            specified_mint: 'mint-a',
            amount_in: 1000,
            direction: 'a_to_b',
            label: 'A→B',
          },
          sharedSwapSignature: 'already-swapped',
        },
        d,
      )
      expect(result.arms[0].status).toBe('failed')
      expect(result.arms[0].error).toMatch(/still does not cover/i)
      expect(d.openPosition).not.toHaveBeenCalled()
    })

    it('continues remaining arms when one open fails', async () => {
      const d = deps({
        openPosition: vi
          .fn()
          .mockRejectedValueOnce(new Error('tick invalid'))
          .mockResolvedValueOnce({ position_pda: 'pda-2', cost_session_id: 's2' }),
      })
      const result = await launchExperiment(
        {
          poolAddress: 'pool-1',
          arms: [arm({ armId: 'arm-1' }), arm({ armId: 'arm-2' })],
        },
        d,
      )
      expect(result.arms[0].status).toBe('failed')
      expect(result.arms[1].status).toBe('opened')
    })

    it('aborts when shared swap fails', async () => {
      const d = deps({
        swapBeforeOpen: vi.fn(async () => {
          throw new Error('swap failed')
        }),
      })
      const result = await launchExperiment(
        {
          poolAddress: 'pool-1',
          arms: [arm()],
          sharedSwap: {
            specified_mint: 'mint-a',
            amount_in: 1000,
            direction: 'a_to_b',
            label: 'A→B',
          },
        },
        d,
      )
      expect(result.aborted).toBe(true)
      expect(result.arms[0].status).toBe('skipped')
      expect(d.openPosition).not.toHaveBeenCalled()
    })
  })
})
