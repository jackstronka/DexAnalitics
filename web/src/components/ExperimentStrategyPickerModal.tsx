import { useEffect, useRef } from 'react'
import ExperimentStrategyPicker from '@/components/ExperimentStrategyPicker'
import type { ExperimentArm } from '@/lib/experimentArm'
import type { Strategy } from '@/lib/api'

type Props = {
  open: boolean
  strategies: Strategy[]
  insertLabel: string
  onSelectArm: (arm: ExperimentArm) => void
  onCancel: () => void
}

export default function ExperimentStrategyPickerModal({
  open,
  strategies,
  insertLabel,
  onSelectArm,
  onCancel,
}: Props) {
  const panelRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onCancel()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open, onCancel])

  useEffect(() => {
    if (open) panelRef.current?.focus()
  }, [open])

  if (!open) return null

  return (
    <div
      className="fixed inset-0 z-50 flex items-end sm:items-center justify-center p-0 sm:p-4"
      role="dialog"
      aria-modal="true"
    >
      <button
        type="button"
        className="absolute inset-0 bg-black/50 backdrop-blur-[2px]"
        aria-label="Close"
        onClick={onCancel}
      />
      <div
        ref={panelRef}
        tabIndex={-1}
        className="relative z-10 w-full sm:max-w-lg max-h-[min(90vh,720px)] overflow-y-auto rounded-t-2xl sm:rounded-2xl shadow-xl outline-none animate-in fade-in slide-in-from-bottom-4 sm:slide-in-from-bottom-0 duration-200"
      >
        <ExperimentStrategyPicker
          strategies={strategies}
          insertLabel={insertLabel}
          onSelectArm={onSelectArm}
          onCancel={onCancel}
        />
      </div>
    </div>
  )
}
