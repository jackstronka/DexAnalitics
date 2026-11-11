/** Pure helpers for Agent Decisions browser (U3 / AH5). */

export type AgentDecisionKindFilter = 'all' | 'dip_propose' | 'other'

/** Extract `decision.kind` when present (e.g. dip_propose). */
export function agentDecisionKind(decision: unknown): string | null {
  if (!decision || typeof decision !== 'object') return null
  const k = (decision as { kind?: unknown }).kind
  if (typeof k !== 'string') return null
  const t = k.trim()
  return t.length > 0 ? t : null
}

/** Extract dip propose `action` when kind is dip_propose. */
export function agentDecisionAction(decision: unknown): string | null {
  if (!decision || typeof decision !== 'object') return null
  const a = (decision as { action?: unknown }).action
  if (typeof a !== 'string') return null
  const t = a.trim()
  return t.length > 0 ? t : null
}

export function matchesAgentDecisionKindFilter(
  decision: unknown,
  filter: AgentDecisionKindFilter,
): boolean {
  if (filter === 'all') return true
  const kind = agentDecisionKind(decision)
  if (filter === 'dip_propose') return kind === 'dip_propose'
  return kind !== 'dip_propose'
}

export function formatAgentDecisionPreview(decision: unknown, maxLen = 120): string {
  try {
    const s = JSON.stringify(decision)
    if (s.length <= maxLen) return s
    return `${s.slice(0, maxLen)}…`
  } catch {
    return '—'
  }
}
