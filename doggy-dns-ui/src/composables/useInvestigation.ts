import { computed, getCurrentScope, onScopeDispose, ref, watch, type Ref } from 'vue'

import { ApiError, apiGet, apiPostWithStatus } from '@/api/http'
import type { AnalysisAccepted, AnalysisSnapshot, EvidenceBundle } from '@/api/types'

export type InvestigationState =
  | 'idle'
  | 'evidenceLoading'
  | 'evidenceReady'
  | 'analysisQueued'
  | 'analysisRunning'
  | 'analysisDone'
  | 'analysisUnavailable'
  | 'stale'

export const ANALYSIS_POLL_INTERVAL_MS = 1000

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause)
}

function isInFlight(snapshot: AnalysisSnapshot): boolean {
  return snapshot.status === 'queued' || snapshot.status === 'running'
}

function snapshotState(snapshot: AnalysisSnapshot): InvestigationState {
  if (snapshot.status === 'queued') return 'analysisQueued'
  if (snapshot.status === 'running') return 'analysisRunning'
  // A succeeded job always carries a conclusion: `ok`, `insufficient_evidence`
  // and `unavailable` are all settled results (spec §7), so none of them is an
  // unavailable state.
  if (snapshot.status === 'succeeded') return 'analysisDone'
  return 'analysisUnavailable'
}

function isSettled(state: InvestigationState): boolean {
  return state === 'analysisDone' || state === 'analysisUnavailable'
}

/**
 * One failure investigation: evidence first, then an asynchronous analysis
 * attempt with polling, deep-link restore and stale-fingerprint detection.
 *
 * Nothing loads implicitly; callers decide when to call `loadEvidence` or
 * `startAnalysis`. The scope owns the poll loop, so a disposed drawer cannot
 * keep fetching.
 */
export function useInvestigation(traceId: Ref<string | null>, windowMinutes: Ref<number>) {
  const state = ref<InvestigationState>('idle')
  const evidence = ref<EvidenceBundle | null>(null)
  const analysis = ref<AnalysisSnapshot | null>(null)
  const error = ref<string | null>(null)

  const stale = computed(
    () =>
      evidence.value !== null &&
      analysis.value !== null &&
      analysis.value.evidenceFingerprint !== evidence.value.fingerprint,
  )

  let generation = 0
  let pollToken = 0
  let pendingTimer: ReturnType<typeof setTimeout> | null = null
  let wake: (() => void) | null = null

  function sleep(ms: number): Promise<void> {
    return new Promise((resolve) => {
      wake = resolve
      pendingTimer = setTimeout(() => {
        pendingTimer = null
        wake = null
        resolve()
      }, ms)
    })
  }

  /** Cancels the outstanding sleep and invalidates any running poll loop. */
  function cancelPolling(): void {
    pollToken += 1
    if (pendingTimer !== null) {
      clearTimeout(pendingTimer)
      pendingTimer = null
    }
    const resume = wake
    wake = null
    resume?.()
  }

  function stateFromAnalysis(): InvestigationState | null {
    if (!analysis.value) return null
    const settled = snapshotState(analysis.value)
    return stale.value && isSettled(settled) ? 'stale' : settled
  }

  function stateFromEvidence(): InvestigationState {
    // The brief pins this: a fresh evidence load is a neutral state, only a
    // fingerprint mismatch with a loaded analysis escalates to `stale`.
    // `loadAnalysis` is the explicit route back to a settled analysis state.
    return stale.value ? 'stale' : 'evidenceReady'
  }

  function applySnapshot(snapshot: AnalysisSnapshot, gen: number): void {
    if (gen !== generation) return
    analysis.value = snapshot
    error.value = snapshot.error ?? null
    const next = snapshotState(snapshot)
    state.value = stale.value && isSettled(next) ? 'stale' : next
  }

  async function loadEvidence(): Promise<boolean> {
    const trace = traceId.value
    if (!trace) return false
    const gen = generation
    state.value = 'evidenceLoading'
    try {
      const bundle = await apiGet<EvidenceBundle>(
        `/api/failures/${encodeURIComponent(trace)}/evidence?windowMinutes=${windowMinutes.value}`,
      )
      if (gen !== generation) return false
      evidence.value = bundle
      error.value = null
      state.value = stateFromEvidence()
      return true
    } catch (cause) {
      if (gen !== generation) return false
      error.value = errorMessage(cause)
      state.value = stateFromAnalysis() ?? 'idle'
      return false
    }
  }

  async function pollAnalysis(trace: string, analysisId: string, gen: number): Promise<void> {
    cancelPolling()
    const token = pollToken
    while (gen === generation && token === pollToken) {
      await sleep(ANALYSIS_POLL_INTERVAL_MS)
      if (gen !== generation || token !== pollToken) return
      try {
        const snapshot = await apiGet<AnalysisSnapshot>(
          `/api/failures/${encodeURIComponent(trace)}/analysis?analysis=${encodeURIComponent(analysisId)}`,
        )
        if (gen !== generation || token !== pollToken) return
        applySnapshot(snapshot, gen)
        if (!isInFlight(snapshot)) return
      } catch (cause) {
        if (gen !== generation || token !== pollToken) return
        error.value = errorMessage(cause)
        return
      }
    }
  }

  async function startAnalysis(force = false): Promise<boolean> {
    const trace = traceId.value
    if (!trace) return false
    // A new start owns the poll loop: stop any surviving loop before issuing
    // requests so neither its timer nor an applied snapshot can clobber the
    // outcome of this attempt.
    cancelPolling()
    const gen = generation
    if (!evidence.value) {
      const loaded = await loadEvidence()
      if (!loaded || gen !== generation) return false
    }

    // A re-run must not keep exposing the previous attempt (e.g. as `stale`)
    // while the replacement is queued or running.
    if (force) analysis.value = null

    state.value = 'analysisQueued'
    error.value = null
    const params = new URLSearchParams({ windowMinutes: String(windowMinutes.value) })
    if (force) params.set('force', 'true')

    try {
      const response = await apiPostWithStatus<AnalysisSnapshot | AnalysisAccepted>(
        `/api/failures/${encodeURIComponent(trace)}/analysis?${params.toString()}`,
        {},
      )
      if (gen !== generation) return false
      if (response.status === 202) {
        const accepted = response.body as AnalysisAccepted
        state.value = 'analysisRunning'
        void pollAnalysis(trace, accepted.analysisId, gen)
        return true
      }
      const snapshot = response.body as AnalysisSnapshot
      applySnapshot(snapshot, gen)
      if (isInFlight(snapshot)) {
        void pollAnalysis(trace, snapshot.analysisId, gen)
      }
      return true
    } catch (cause) {
      if (gen !== generation) return false
      error.value = errorMessage(cause)
      if (cause instanceof ApiError && cause.status === 503) {
        state.value = 'analysisUnavailable'
      } else {
        // 429 (rate limited) and transient failures leave loaded evidence usable.
        state.value = evidence.value ? 'evidenceReady' : 'idle'
      }
      return false
    }
  }

  async function loadAnalysis(analysisId?: string): Promise<boolean> {
    const trace = traceId.value
    if (!trace) return false
    // Restoring an attempt replaces whatever was polling: cancel first so an
    // orphaned loop cannot overwrite the restored snapshot on its next tick.
    cancelPolling()
    const gen = generation
    const query = analysisId ? `?analysis=${encodeURIComponent(analysisId)}` : ''
    try {
      const snapshot = await apiGet<AnalysisSnapshot>(
        `/api/failures/${encodeURIComponent(trace)}/analysis${query}`,
      )
      if (gen !== generation) return false
      applySnapshot(snapshot, gen)
      if (isInFlight(snapshot)) {
        void pollAnalysis(trace, snapshot.analysisId, gen)
      }
      return true
    } catch (cause) {
      if (gen !== generation) return false
      if (cause instanceof ApiError && cause.status === 404) {
        // No saved attempt for this trace: clear it silently, no error banner.
        analysis.value = null
        error.value = null
        state.value = evidence.value ? 'evidenceReady' : 'idle'
        return false
      }
      error.value = errorMessage(cause)
      return false
    }
  }

  watch(traceId, () => {
    generation += 1
    cancelPolling()
    evidence.value = null
    analysis.value = null
    error.value = null
    state.value = 'idle'
  })

  if (getCurrentScope()) {
    onScopeDispose(() => {
      generation += 1
      cancelPolling()
    })
  }

  return { state, evidence, analysis, error, stale, loadEvidence, startAnalysis, loadAnalysis }
}
