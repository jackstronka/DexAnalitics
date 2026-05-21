/** Curated Orca Whirlpool pools for operator open / experiment flows. */
export type CuratedPool = {
  label: string
  address: string
}

export const CURATED_ORCA_POOLS: CuratedPool[] = [
  {
    label: 'SOL/USDC (0.04%)',
    address: 'Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE',
  },
  {
    label: 'whETH/SOL (0.05%)',
    address: 'HktfL7iwGKT5QHjywQkcDnZXScoh811k7akrMZJkCcEF',
  },
  {
    label: 'cbBTC/USDC (0.04%)',
    address: 'HxA6SKW5qA4o12fjVgTpXdq2YnZ5Zv1s7SB4FFomsyLM',
  },
  {
    label: 'WBTC/cbBTC (0.01%)',
    address: '4v8ufj8Hj7UvFgtofQJAtzUud5xomwZfEqfCTHZ4wM72',
  },
]
