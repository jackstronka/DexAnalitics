/** `GET …/chain-history` appends this marker to the top-level `note` (see API `load_chain_history_from_db`). */
export function isLineageFromPostgresMaterialized(
  lineage: { note?: string | null } | null | undefined,
): boolean {
  return (lineage?.note ?? '').toLowerCase().includes('postgres_chain_history')
}
