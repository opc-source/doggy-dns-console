import { onMounted, onUnmounted, ref, watch, type Ref } from 'vue'

import { apiGet } from '@/api/http'
import type { AnalysisMarker, FailureGroup } from '@/api/types'
import {
  isAbsolute,
  resolveBracket,
  toRequestQuery,
  validateBracket,
  validateDraft,
  type FilterDraft,
} from '@/lib/failureFilter'

/** The markers endpoint accepts at most 200 unique trace ids per request. */
const MAX_MARKER_TRACE_IDS = 200
const POLL_INTERVAL_MS = 15_000

function buildQuery(params: Record<string, string>): string {
  const search = new URLSearchParams()
  for (const [key, value] of Object.entries(params)) search.set(key, value)
  return search.toString()
}

export function useFailureList(draft: Ref<FilterDraft>, limit = 5) {
  const groups = ref<FailureGroup[]>([])
  const markers = ref<Set<string>>(new Set())
  const uncovered = ref(0)
  const error = ref<string | null>(null)
  let timer: ReturnType<typeof setInterval> | null = null
  let listSeq = 0
  let markerSeq = 0
  let uncoveredSeq = 0

  async function refreshMarkers() {
    const seq = ++markerSeq
    const ids = Array.from(
      new Set(
        groups.value
          .map((group) => group.traceId)
          .filter((traceId): traceId is string => traceId !== null),
      ),
    ).slice(0, MAX_MARKER_TRACE_IDS)

    if (ids.length === 0) {
      markers.value = new Set()
      return
    }

    const query = ids.map((traceId) => encodeURIComponent(traceId)).join(',')
    try {
      const found = await apiGet<AnalysisMarker[]>(`/api/analysis/markers?traceIds=${query}`)
      // A newer draft's refresh owns the markers; drop this older answer.
      if (seq !== markerSeq) return
      const requested = new Set(ids)
      markers.value = new Set(
        found.map((marker) => marker.traceId).filter((traceId) => requested.has(traceId)),
      )
    } catch {
      // Markers are cosmetic: never surface an error or hide the failure list.
      if (seq !== markerSeq) return
      markers.value = new Set()
    }
  }

  async function refreshUncovered() {
    const seq = ++uncoveredSeq
    if (!draft.value.instance) {
      if (seq === uncoveredSeq) uncovered.value = 0
      return
    }
    try {
      const found = await apiGet<{ unlabeled: number }>(
        `/api/failures/uncovered?${buildQuery(toRequestQuery(draft.value))}`,
      )
      // A newer draft's refresh owns the notice; drop this older answer.
      if (seq !== uncoveredSeq) return
      uncovered.value = found.unlabeled
    } catch {
      // The notice is advisory; a failed count must not hide the list.
      if (seq !== uncoveredSeq) return
      uncovered.value = 0
    }
  }

  function restartTimer() {
    if (timer) {
      clearInterval(timer)
      timer = null
    }
    // An absolute bracket is a fixed question, so refreshing it on a timer would
    // only move the answer under the operator's feet.
    if (!isAbsolute(draft.value)) timer = setInterval(() => void refresh(), POLL_INTERVAL_MS)
  }

  async function refresh() {
    const seq = ++listSeq
    const now = Date.now()
    const bracket = resolveBracket(draft.value, now)
    const invalid = validateDraft(draft.value) ?? validateBracket(bracket, now)
    if (invalid) {
      error.value = invalid
      groups.value = []
      markers.value = new Set()
      // Retire both in-flight answers: the bracket those requests describe was
      // just refused, so neither may re-land above the empty table.
      markerSeq += 1
      uncoveredSeq += 1
      uncovered.value = 0
      return
    }

    try {
      const found = await apiGet<FailureGroup[]>(
        `/api/failures?${buildQuery({ ...toRequestQuery(draft.value), limit: String(limit) })}`,
      )
      if (seq !== listSeq) return
      groups.value = found
      error.value = null
    } catch (cause) {
      if (seq !== listSeq) return
      error.value = String(cause)
      markers.value = new Set()
      uncoveredSeq += 1
      uncovered.value = 0
      return
    }
    await Promise.all([refreshMarkers(), refreshUncovered()])
  }

  onMounted(() => {
    void refresh()
    restartTimer()
  })
  onUnmounted(() => {
    if (timer) clearInterval(timer)
  })
  watch(draft, () => {
    void refresh()
    restartTimer()
  })

  return { groups, markers, error, uncovered, refresh, refreshMarkers }
}
