import type { SessionSummary } from '@/api/types'

function text(state: Record<string, unknown>, key: string): string | null {
  const value = state[key]
  return typeof value === 'string' && value !== '' ? value : null
}

function ms(state: Record<string, unknown>, key: string): number | null {
  const value = state[key]
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

/**
 * The label a session carries in its own state. The list endpoint never replays
 * events, so state is the only thing that can say what a session was for;
 * sessions created before it was written are named by their id so that two
 * unlabelled ones can still be told apart.
 */
export function sessionTitle(session: SessionSummary): string {
  return text(session.state, 'title') ?? `（无标题）${session.id.slice(0, 8)}`
}

/** Where a handoff was asked from, or null for a chat that never was. */
export function sessionFailureTarget(session: SessionSummary) {
  const traceId = text(session.state, 'traceId')
  if (!traceId) return null
  const query: Record<string, string> = {}
  const analysisId = text(session.state, 'analysisId')
  const from = ms(session.state, 'from')
  const to = ms(session.state, 'to')
  // Without the bracket the drawer would reopen on the default window, which
  // rarely still contains the failure the question was asked about.
  if (from !== null && to !== null) {
    query.from = String(from)
    query.to = String(to)
  }
  if (analysisId) query.analysis = analysisId
  return { name: 'failure-detail', params: { traceId }, query }
}
