import { shortenAddress, formatUsdTokenSpot, formatNumber } from '@/lib/utils'

export function PoolPairLabels(props: {
  labelA?: string | null
  labelB?: string | null
  mintA?: string | null
  mintB?: string | null
  priceA?: number | null
  priceB?: number | null
}) {
  const pair =
    props.labelA && props.labelB ? `${props.labelA} / ${props.labelB}` : null
  const hasPrices =
    props.priceA != null &&
    props.priceB != null &&
    Number.isFinite(props.priceA) &&
    Number.isFinite(props.priceB)
  const ratioAperB =
    hasPrices &&
    props.labelA &&
    props.labelB &&
    props.priceA != null &&
    props.priceB != null &&
    props.priceA > 0 &&
    props.priceB > 0
      ? props.priceB / props.priceA
      : null
  return (
    <div className="space-y-0.5">
      {pair ? <div className="font-medium">{pair}</div> : null}
      {hasPrices ? (
        <div className="text-xs text-muted-foreground font-mono">
          {formatUsdTokenSpot(props.priceA)} · {formatUsdTokenSpot(props.priceB)}
          <span className="text-[10px] font-sans text-muted-foreground/90"> / 1 token</span>
        </div>
      ) : null}
      {ratioAperB != null && props.labelA && props.labelB ? (
        <div className="text-[11px] text-muted-foreground font-mono">
          {formatNumber(ratioAperB, 4)} {props.labelA} / 1 {props.labelB}
        </div>
      ) : null}
      {props.mintA && props.mintB ? (
        <div className="text-[10px] text-muted-foreground font-mono">
          {shortenAddress(props.mintA, 4)} · {shortenAddress(props.mintB, 4)}
        </div>
      ) : null}
    </div>
  )
}
