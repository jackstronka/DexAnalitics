import type { HTMLAttributes, ReactNode } from 'react'
import { cn } from '@/lib/utils'

type InlineErrorProps = {
  children: ReactNode
  as?: 'div' | 'span' | 'p'
} & HTMLAttributes<HTMLElement>

export function InlineError({ children, as = 'div', className, ...rest }: InlineErrorProps) {
  const Comp = as
  return (
    <Comp
      className={cn(
        'rounded-md border border-destructive/60 bg-destructive/25 px-2 py-1 text-xs text-destructive-foreground',
        className,
      )}
      {...rest}
    >
      {children}
    </Comp>
  )
}

