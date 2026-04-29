import type { HTMLAttributes, ReactNode } from 'react'
import { cn } from '@/lib/utils'

type ErrorBannerProps = {
  children: ReactNode
  as?: 'div' | 'p' | 'section' | 'span'
} & HTMLAttributes<HTMLElement>

export function ErrorBanner({ children, as = 'div', className, ...rest }: ErrorBannerProps) {
  const Comp = as
  return (
    <Comp
      className={cn(
        'rounded-md border border-destructive/60 bg-destructive/25 px-3 py-2 text-sm text-destructive-foreground',
        className,
      )}
      {...rest}
    >
      {children}
    </Comp>
  )
}

