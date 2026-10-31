import { onMounted, onUnmounted, ref, watch, type Ref } from 'vue'

import { apiGet } from '@/api/http'
import type { AnalysisMarker, FailureGroup } from '@/api/types'

/** The markers endpoint accepts at most 200 unique trace ids per request. */
const MAX_MARKER_TRACE_IDS = 200

export function useFailureList(windowMinutes: Ref<number>, limit = 5) {
  const groups = ref<FailureGroup[]>([])
  const markers = ref<Set<string>>(new Set())
  const error = ref<string | null>(null)
  let timer: ReturnType<typeof setInterval> | null = null
  let markerSeq = 0

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
      // A newer window's refresh owns the markers; drop this older answer.
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

  async function refresh() {
    try {
      groups.value = await apiGet<FailureGroup[]>(
        `/api/failures?windowMinutes=${windowMinutes.value}&limit=${limit}`,
      )
      error.value = null
    } catch (cause) {
      error.value = String(cause)
      // A failed window switch must not leave the previous window's badges behind.
      markers.value = new Set()
      return
    }
    await refreshMarkers()
  }

  onMounted(() => {
    void refresh()
    timer = setInterval(refresh, 15_000)
  })
  onUnmounted(() => {
    if (timer) clearInterval(timer)
  })
  watch(windowMinutes, () => void refresh())

  return { groups, markers, error, refresh, refreshMarkers }
}
