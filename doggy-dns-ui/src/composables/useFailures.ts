import { ref, watch, type Ref } from 'vue'

import { apiGet } from '@/api/http'
import type { FailureDetail } from '@/api/types'

export function useFailureDetail(traceId: Ref<string | null>, windowMinutes: Ref<number>) {
  const detail = ref<FailureDetail | null>(null)
  const error = ref<string | null>(null)
  const loading = ref(false)

  async function refresh() {
    if (!traceId.value) {
      detail.value = null
      return
    }
    loading.value = true
    try {
      detail.value = await apiGet<FailureDetail>(
        `/api/failures/${encodeURIComponent(traceId.value)}?windowMinutes=${windowMinutes.value}`,
      )
      error.value = null
    } catch (cause) {
      error.value = cause instanceof Error ? cause.message : String(cause)
    } finally {
      loading.value = false
    }
  }

  watch([traceId, windowMinutes], () => void refresh(), { immediate: true })
  return { detail, error, loading, refresh }
}
